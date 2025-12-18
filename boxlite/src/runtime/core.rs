//! High-level sandbox runtime structures.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use crate::litebox::{BoxBuilder, LiteBox};
use crate::runtime::constants::filenames;
use crate::runtime::initrf::InitRootfs;
use crate::runtime::layout::{FilesystemLayout, FsLayoutConfig};
use crate::runtime::lock::RuntimeLock;
use crate::runtime::options::{BoxOptions, BoxliteOptions, RootfsSpec};
use crate::{
    images::ImageManager,
    init_logging_for,
    management::{BoxID, BoxInfo, BoxManager, BoxMetadata, BoxState, generate_box_id},
    metrics::{RuntimeMetrics, RuntimeMetricsStorage},
    vmm::VmmKind,
};
use boxlite_shared::{
    Transport,
    errors::{BoxliteError, BoxliteResult},
};
use chrono::Utc;
use tokio::sync::OnceCell;

// ============================================================================
// GLOBAL DEFAULT RUNTIME
// ============================================================================

/// Global default runtime singleton (lazy initialization).
///
/// This runtime uses `BoxliteOptions::default()` for configuration.
/// Most applications should use this instead of creating custom runtimes.
static DEFAULT_RUNTIME: OnceLock<BoxliteRuntime> = OnceLock::new();
// ============================================================================
// PUBLIC API
// ============================================================================

/// BoxliteRuntime provides the main entry point for creating and managing Boxes.
///
/// **Architecture**: Uses a single `RwLock` to protect all mutable state (boxes and images).
/// This eliminates nested locking and simplifies reasoning about concurrency.
///
/// **Lock Behavior**: Only one `BoxliteRuntime` can use a given `BOXLITE_HOME`
/// directory at a time. The filesystem lock is automatically released when dropped.
///
/// **Cloning**: Runtime is cheaply cloneable via `Arc` - all clones share the same state.
#[derive(Clone)]
pub struct BoxliteRuntime {
    inner: RuntimeInner,
}

/// Internal runtime state protected by single lock.
///
/// **Shared via Arc**: This is the actual shared state that can be cloned cheaply.
pub type RuntimeInner = Arc<RuntimeInnerImpl>;

/// Runtime inner implementation - just data structure with lock helpers.
///
/// **Design Philosophy**: This struct only holds state and provides lock acquisition helpers.
/// It does NOT wrap BoxManager/ImageManager operations. Components acquire the lock directly
/// and call manager methods.
///
/// **Locking Strategy**:
/// - `mutable`: Protected by `RwLock` - concurrent reads, exclusive writes
/// - `immutable`: No lock needed - never changes after creation
pub struct RuntimeInnerImpl {
    /// All mutable state (boxes, images) - protected by ONE lock
    pub(crate) sync_state: RwLock<SynchronizedState>,

    /// Immutable configuration and resources
    pub(crate) non_sync_state: NonSynchronizedState,
}

/// Mutable runtime state protected by RwLock.
///
/// **Design**: Both managers have their internal locks removed.
/// All access must go through RuntimeInnerImpl::mutable lock.
pub struct SynchronizedState {
    pub(crate) box_manager: BoxManager,
    pub(crate) image_manager: ImageManager,
}

/// Immutable runtime resources (no lock needed).
///
/// **Thread Safety**: All fields are either `Clone` or wrapped in `Arc`.
/// RuntimeMetricsStorage lives here because it uses AtomicU64 internally - no lock needed!
pub struct NonSynchronizedState {
    pub(crate) layout: FilesystemLayout,
    pub(crate) init_rootfs: Arc<OnceCell<InitRootfs>>,
    /// Runtime-wide metrics (AtomicU64 based, lock-free)
    pub(crate) runtime_metrics: RuntimeMetricsStorage,
    _runtime_lock: RuntimeLock,
}

// ============================================================================
// RUNTIME IMPLEMENTATION
// ============================================================================

impl BoxliteRuntime {
    /// Create a new BoxliteRuntime with the provided options.
    ///
    /// **Prepare Before Execute**: All setup (filesystem, locks, managers) completes
    /// before returning. No partial initialization states.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Another `BoxliteRuntime` is already using the same home directory
    /// - Filesystem initialization fails
    /// - Image API initialization fails
    pub fn new(options: BoxliteOptions) -> BoxliteResult<Self> {
        // Validate Early: Check preconditions before expensive work
        if !options.home_dir.is_absolute() {
            return Err(BoxliteError::Internal(format!(
                "home_dir must be absolute path, got: {}",
                options.home_dir.display()
            )));
        }

        // Prepare: All setup before point of no return
        // Configure bind mount support based on platform
        #[cfg(target_os = "linux")]
        let fs_config = FsLayoutConfig::with_bind_mount();
        #[cfg(not(target_os = "linux"))]
        let fs_config = FsLayoutConfig::without_bind_mount();

        let layout = FilesystemLayout::new(options.home_dir.clone(), fs_config);

        layout.prepare().map_err(|e| {
            BoxliteError::Storage(format!(
                "Failed to initialize filesystem at {}: {}",
                layout.home_dir().display(),
                e
            ))
        })?;

        init_logging_for(&layout)?;

        let runtime_lock = RuntimeLock::acquire(layout.home_dir()).map_err(|e| {
            BoxliteError::Internal(format!(
                "Failed to acquire runtime lock at {}: {}",
                layout.home_dir().display(),
                e
            ))
        })?;

        let image_manager = ImageManager::new(layout.images_dir()).map_err(|e| {
            BoxliteError::Storage(format!(
                "Failed to initialize image manager at {}: {}",
                layout.images_dir().display(),
                e
            ))
        })?;

        let inner = Arc::new(RuntimeInnerImpl {
            sync_state: RwLock::new(SynchronizedState {
                box_manager: BoxManager::new(),
                image_manager,
            }),
            non_sync_state: NonSynchronizedState {
                layout,
                init_rootfs: Arc::new(OnceCell::new()),
                runtime_metrics: RuntimeMetricsStorage::new(),
                _runtime_lock: runtime_lock,
            },
        });

        tracing::debug!("initialized runtime");

        Ok(Self { inner })
    }

    /// Create a new runtime with default options.
    ///
    /// This is equivalent to `BoxliteRuntime::new(BoxliteOptions::default())`
    /// but returns a `Result` instead of panicking.
    ///
    /// Prefer `default_runtime()` for most use cases (shares global instance).
    /// Use this when you need an owned, non-global runtime with default config.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use boxlite::runtime::BoxliteRuntime;
    ///
    /// let runtime = BoxliteRuntime::with_defaults()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn with_defaults() -> BoxliteResult<Self> {
        Self::new(BoxliteOptions::default())
    }

    /// Get or initialize the default global runtime.
    ///
    /// This runtime uses `BoxliteOptions::default()` for configuration.
    /// The runtime is created lazily on first access and reused for all
    /// subsequent calls.
    ///
    /// # Panics
    ///
    /// Panics if runtime initialization fails. This indicates a serious
    /// system issue (e.g., cannot create home directory, filesystem lock).
    ///
    /// # Example
    ///
    /// ```no_run
    /// use boxlite::runtime::BoxliteRuntime;
    ///
    /// let runtime = BoxliteRuntime::default_runtime();
    /// // All subsequent calls return the same runtime
    /// let same_runtime = BoxliteRuntime::default_runtime();
    /// ```
    pub fn default_runtime() -> &'static Self {
        DEFAULT_RUNTIME.get_or_init(|| {
            Self::with_defaults().expect("Failed to initialize default BoxliteRuntime")
        })
    }

    /// Try to get the default runtime if it's been initialized.
    ///
    /// Returns `None` if the default runtime hasn't been created yet.
    /// Useful for checking if default runtime exists without creating it.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use boxlite::runtime::BoxliteRuntime;
    ///
    /// if let Some(runtime) = BoxliteRuntime::try_default_runtime() {
    ///     println!("Default runtime already exists");
    /// } else {
    ///     println!("Default runtime not yet created");
    /// }
    /// ```
    pub fn try_default_runtime() -> Option<&'static Self> {
        DEFAULT_RUNTIME.get()
    }

    /// Initialize the default runtime with custom options.
    ///
    /// This must be called before the first use of `default_runtime()`.
    /// Returns an error if the default runtime has already been initialized.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Default runtime already initialized (call this early in main!)
    /// - Runtime initialization fails (filesystem, lock, etc.)
    ///
    /// # Example
    ///
    /// ```no_run
    /// use boxlite::runtime::{BoxliteRuntime, BoxliteOptions};
    /// use std::path::PathBuf;
    ///
    /// fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let mut opts = BoxliteOptions::default();
    ///     opts.home_dir = PathBuf::from("/custom/boxlite");
    ///
    ///     BoxliteRuntime::init_default_runtime(opts)?;
    ///
    ///     // All subsequent default_runtime() calls use custom config
    ///     let runtime = BoxliteRuntime::default_runtime();
    ///     Ok(())
    /// }
    /// ```
    pub fn init_default_runtime(options: BoxliteOptions) -> BoxliteResult<()> {
        let runtime = Self::new(options)?;
        DEFAULT_RUNTIME
            .set(runtime)
            .map_err(|_| BoxliteError::Internal(
                "Default runtime already initialized. Call init_default_runtime() before any use of default_runtime().".into()
            ))
    }

    /// Create a sandbox handle.
    ///
    /// Returns immediately with a LiteBox handle. Heavy initialization (image pulling,
    /// Box startup) is deferred until the first API call on the handle.
    ///
    /// **Single Responsibility**: Only registers box metadata and creates handle.
    /// Box startup happens separately in LiteBox::start().
    pub fn create(&self, options: BoxOptions) -> BoxliteResult<(BoxID, LiteBox)> {
        let box_id = generate_box_id();

        // Register box metadata
        self.register_box(&box_id, &options)?;

        // Increment boxes_created counter (lock-free!)
        self.inner
            .non_sync_state
            .runtime_metrics
            .boxes_created
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Create builder and handle
        let builder = BoxBuilder::new(box_id.clone(), Arc::clone(&self.inner), options);
        let handle = LiteBox::new(box_id.clone(), Arc::clone(&self.inner), builder);

        Ok((box_id, handle))
    }

    /// List all boxes, sorted by creation time (newest first).
    ///
    /// **State Refresh**: Automatically checks process liveness and updates states
    /// before returning results.
    pub fn list(&self) -> BoxliteResult<Vec<BoxInfo>> {
        // Acquire write lock
        let state = self.inner.acquire_write()?;

        // Call BoxManager methods directly
        state.box_manager.refresh_states()?;
        state.box_manager.list()
    }

    /// Get information about a specific box.
    pub fn get(&self, id: &BoxID) -> BoxliteResult<Option<BoxInfo>> {
        // Acquire read lock
        let state = self.inner.acquire_read()?;

        // Call BoxManager method directly and convert to BoxInfo
        Ok(state.box_manager.get(id)?.map(|m| m.to_info()))
    }

    /// Remove a stopped box from the manager.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Box doesn't exist
    /// - Box is still in an active state (Starting or Running)
    pub fn remove(&self, id: &BoxID) -> BoxliteResult<()> {
        // Acquire write lock
        let state = self.inner.acquire_write()?;

        // Call BoxManager method directly and discard the returned metadata
        state.box_manager.remove(id).map(|_| ())
    }

    /// Get runtime-wide metrics.
    ///
    /// Returns a handle for querying aggregate statistics across all boxes.
    /// All counters are monotonic and never reset.
    ///
    /// **Lock-Free**: Uses AtomicU64 internally, no lock needed!
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use boxlite_runtime::BoxliteRuntime;
    /// # fn example(runtime: &BoxliteRuntime) {
    /// let metrics = runtime.metrics();
    /// println!("Total boxes created: {}", metrics.boxes_created_total());
    /// println!("Total commands executed: {}", metrics.total_commands_executed());
    /// # }
    /// ```
    pub fn metrics(&self) -> RuntimeMetrics {
        // No lock needed! RuntimeMetricsStorage uses AtomicU64 internally
        RuntimeMetrics::new(self.inner.non_sync_state.runtime_metrics.clone())
    }
}

// ============================================================================
// INTERNAL HELPERS
// ============================================================================

impl BoxliteRuntime {
    /// Register box metadata in manager.
    ///
    /// **Single Responsibility**: Only creates and registers metadata.
    /// Does not start Box or initialize resources.
    fn register_box(&self, box_id: &BoxID, options: &BoxOptions) -> BoxliteResult<()> {
        let metadata = BoxMetadata {
            id: box_id.clone(),
            state: BoxState::Starting,
            created_at: Utc::now(),
            pid: None,
            transport: Transport::unix(filenames::unix_socket_path(
                self.inner.non_sync_state.layout.home_dir(),
                box_id,
            )),
            image: match &options.rootfs {
                RootfsSpec::Image(r) => r.clone(),
                RootfsSpec::RootfsPath(p) => format!("rootfs:{}", p),
            },
            cpus: options.cpus.unwrap_or(2),
            memory_mib: options.memory_mib.unwrap_or(512),
            labels: HashMap::new(),
            engine_kind: VmmKind::Libkrun,
        };

        // Acquire lock and register
        let state = self.inner.acquire_write()?;
        state.box_manager.register(metadata)
    }
}

// ============================================================================
// RUNTIME INNER - LOCK HELPERS ONLY
// ============================================================================

impl RuntimeInnerImpl {
    /// Acquire read lock with explicit error handling.
    ///
    /// **Single Responsibility**: Only acquires lock and provides error context.
    /// Does NOT wrap any BoxManager/ImageManager operations.
    ///
    /// **Explicit Errors**: Self-documenting error messages that explain
    /// what lock failed and why.
    pub(crate) fn acquire_read(
        &self,
    ) -> BoxliteResult<std::sync::RwLockReadGuard<'_, SynchronizedState>> {
        self.sync_state.read().map_err(|e| {
            BoxliteError::Internal(format!("Runtime state lock poisoned (read): {}", e))
        })
    }

    /// Acquire write lock with explicit error handling.
    ///
    /// **Single Responsibility**: Only acquires lock and provides error context.
    pub(crate) fn acquire_write(
        &self,
    ) -> BoxliteResult<std::sync::RwLockWriteGuard<'_, SynchronizedState>> {
        self.sync_state.write().map_err(|e| {
            BoxliteError::Internal(format!("Runtime state lock poisoned (write): {}", e))
        })
    }
}

impl std::fmt::Debug for BoxliteRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoxliteRuntime")
            .field("home_dir", &self.inner.non_sync_state.layout.home_dir())
            .finish()
    }
}

impl std::fmt::Debug for RuntimeInnerImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeInner")
            .field("home_dir", &self.non_sync_state.layout.home_dir())
            .finish()
    }
}

// ============================================================================
// THREAD SAFETY ASSERTIONS
// ============================================================================

// Compile-time assertions to ensure BoxliteRuntime is Send + Sync
// This is critical for multithreaded usage (e.g., Python GIL release)
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    let _ = assert_send_sync::<BoxliteRuntime>;
};
