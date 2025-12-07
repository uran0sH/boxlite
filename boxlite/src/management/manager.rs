//! Thread-safe box manager implementation.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use boxlite_shared::errors::{BoxliteError, BoxliteResult};

use crate::runtime::types::{BoxID, BoxInfo, BoxMetadata, BoxState};

/// Thread-safe manager for tracking live boxes.
///
/// This is shared between BoxliteRuntime and BoxliteHandles via Arc<>.
/// Uses RwLock for concurrent reads (list) with exclusive writes (register/update).
///
/// # Design
///
/// - **Shared ownership**: Cloneable via `Arc`, passed to runtime and handles
/// - **Concurrent access**: RwLock allows multiple readers, single writer
/// - **State tracking**: Detects crashed processes via `kill(pid, 0)` polling
/// - **No persistence**: In-memory only (Phase 1), can add disk storage later
#[derive(Clone, Debug)]
pub struct BoxManager {
    inner: Arc<RwLock<BoxManagerInner>>,
}

#[derive(Debug)]
struct BoxManagerInner {
    boxes: HashMap<BoxID, BoxMetadata>,
}

impl BoxManager {
    /// Create a new empty manager.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(BoxManagerInner {
                boxes: HashMap::new(),
            })),
        }
    }

    /// Register a new box.
    ///
    /// # Errors
    ///
    /// Returns error if a box with this ID already exists.
    #[allow(private_interfaces)]
    pub fn register(&self, metadata: BoxMetadata) -> BoxliteResult<()> {
        let mut inner = self
            .inner
            .write()
            .map_err(|e| BoxliteError::Internal(format!("manager lock poisoned: {}", e)))?;

        if inner.boxes.contains_key(&metadata.id) {
            return Err(BoxliteError::Internal(format!(
                "box {} already registered",
                metadata.id
            )));
        }

        tracing::debug!(
            box_id = %metadata.id,
            state = ?metadata.state,
            "Registering box"
        );
        inner.boxes.insert(metadata.id.clone(), metadata);
        Ok(())
    }

    /// Update the state of an existing box.
    ///
    /// # Errors
    ///
    /// Returns error if the box doesn't exist.
    pub fn update_state(&self, id: &BoxID, new_state: BoxState) -> BoxliteResult<()> {
        let mut inner = self
            .inner
            .write()
            .map_err(|e| BoxliteError::Internal(format!("manager lock poisoned: {}", e)))?;

        if let Some(metadata) = inner.boxes.get_mut(id) {
            tracing::debug!(
                box_id = %id,
                old_state = ?metadata.state,
                new_state = ?new_state,
                "Updating box state"
            );
            metadata.state = new_state;
            Ok(())
        } else {
            Err(BoxliteError::Internal(format!("box {} not found", id)))
        }
    }

    /// Update the PID of an existing box.
    ///
    /// # Errors
    ///
    /// Returns error if the box doesn't exist.
    pub fn update_pid(&self, id: &BoxID, pid: Option<u32>) -> BoxliteResult<()> {
        let mut inner = self
            .inner
            .write()
            .map_err(|e| BoxliteError::Internal(format!("manager lock poisoned: {}", e)))?;

        if let Some(metadata) = inner.boxes.get_mut(id) {
            tracing::trace!(box_id = %id, pid = ?pid, "Updating box PID");
            metadata.pid = pid;
            Ok(())
        } else {
            Err(BoxliteError::Internal(format!("box {} not found", id)))
        }
    }

    /// Get metadata for a specific box.
    ///
    /// Returns `Ok(None)` if the box doesn't exist.
    #[allow(private_interfaces)]
    pub fn get(&self, id: &BoxID) -> BoxliteResult<Option<BoxMetadata>> {
        let inner = self
            .inner
            .read()
            .map_err(|e| BoxliteError::Internal(format!("manager lock poisoned: {}", e)))?;

        Ok(inner.boxes.get(id).cloned())
    }

    /// List all boxes, sorted by creation time (newest first).
    pub fn list(&self) -> BoxliteResult<Vec<BoxInfo>> {
        let inner = self
            .inner
            .read()
            .map_err(|e| BoxliteError::Internal(format!("manager lock poisoned: {}", e)))?;

        let mut infos: Vec<BoxInfo> = inner.boxes.values().map(|m| m.to_info()).collect();

        // Sort by creation time (newest first)
        // ULID encoding makes this efficient (timestamp in first 48 bits)
        infos.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        Ok(infos)
    }

    /// Remove a box from the manager.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Box doesn't exist
    /// - Box is still in an active state (Starting or Running)
    #[allow(private_interfaces)]
    pub fn remove(&self, id: &BoxID) -> BoxliteResult<BoxMetadata> {
        let mut inner = self
            .inner
            .write()
            .map_err(|e| BoxliteError::Internal(format!("manager lock poisoned: {}", e)))?;

        // Check if box exists and is in terminal state
        if let Some(metadata) = inner.boxes.get(id)
            && metadata.state.is_active()
        {
            return Err(BoxliteError::Internal(format!(
                "cannot remove active box {} (state: {:?})",
                id, metadata.state
            )));
        }

        tracing::debug!(box_id = %id, "Removing box from manager");
        inner
            .boxes
            .remove(id)
            .ok_or_else(|| BoxliteError::Internal(format!("box {} not found", id)))
    }

    /// Check process liveness and update states accordingly.
    ///
    /// This is called periodically or on-demand during list() to ensure
    /// states reflect reality (e.g., detect crashed processes).
    ///
    /// Uses `kill(pid, 0)` to check if process exists without sending signal.
    pub fn refresh_states(&self) -> BoxliteResult<()> {
        let mut inner = self
            .inner
            .write()
            .map_err(|e| BoxliteError::Internal(format!("manager lock poisoned: {}", e)))?;

        for metadata in inner.boxes.values_mut() {
            // Only check active boxes
            if !metadata.state.is_active() {
                continue;
            }

            if let Some(pid) = metadata.pid {
                // Check if process exists using kill(pid, 0)
                // This is cross-platform on Unix and doesn't actually send a signal
                let alive = unsafe { libc::kill(pid as i32, 0) } == 0;

                if !alive {
                    tracing::warn!(
                        box_id = %metadata.id,
                        pid = pid,
                        old_state = ?metadata.state,
                        "Detected crashed box process, marking as Failed"
                    );
                    metadata.state = BoxState::Failed;
                }
            }
        }

        Ok(())
    }

    /// Get the number of boxes being tracked.
    pub fn count(&self) -> BoxliteResult<usize> {
        let inner = self
            .inner
            .read()
            .map_err(|e| BoxliteError::Internal(format!("manager lock poisoned: {}", e)))?;

        Ok(inner.boxes.len())
    }
}

impl Default for BoxManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vmm::VmmKind;
    use boxlite_shared::Transport;
    use chrono::Utc;
    use std::path::PathBuf;

    fn create_test_metadata(id: &str, state: BoxState) -> BoxMetadata {
        BoxMetadata {
            id: id.to_string(),
            state,
            created_at: Utc::now(),
            pid: Some(99999), // Non-existent PID for testing
            transport: Transport::unix(PathBuf::from("/tmp/test.sock")),
            image: "test:latest".to_string(),
            cpus: 2,
            memory_mib: 512,
            labels: HashMap::new(),
            engine_kind: VmmKind::Libkrun,
        }
    }

    #[test]
    fn test_register_and_get() {
        let manager = BoxManager::new();
        let metadata = create_test_metadata("test-id", BoxState::Starting);

        manager.register(metadata.clone()).unwrap();

        let retrieved = manager.get(&metadata.id).unwrap().unwrap();
        assert_eq!(retrieved.id, metadata.id);
        assert_eq!(retrieved.state, metadata.state);
    }

    #[test]
    fn test_duplicate_registration_fails() {
        let manager = BoxManager::new();
        let metadata = create_test_metadata("test-id", BoxState::Starting);

        manager.register(metadata.clone()).unwrap();
        let result = manager.register(metadata);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("already registered")
        );
    }

    #[test]
    fn test_update_state() {
        let manager = BoxManager::new();
        let metadata = create_test_metadata("test-id", BoxState::Starting);

        manager.register(metadata.clone()).unwrap();
        manager
            .update_state(&metadata.id, BoxState::Running)
            .unwrap();

        let retrieved = manager.get(&metadata.id).unwrap().unwrap();
        assert_eq!(retrieved.state, BoxState::Running);
    }

    #[test]
    fn test_update_pid() {
        let manager = BoxManager::new();
        let mut metadata = create_test_metadata("test-id", BoxState::Starting);
        metadata.pid = None;

        manager.register(metadata.clone()).unwrap();
        manager.update_pid(&metadata.id, Some(12345)).unwrap();

        let retrieved = manager.get(&metadata.id).unwrap().unwrap();
        assert_eq!(retrieved.pid, Some(12345));
    }

    #[test]
    fn test_list_boxes() {
        let manager = BoxManager::new();

        manager
            .register(create_test_metadata("id1", BoxState::Running))
            .unwrap();
        manager
            .register(create_test_metadata("id2", BoxState::Stopped))
            .unwrap();
        manager
            .register(create_test_metadata("id3", BoxState::Running))
            .unwrap();

        let boxes = manager.list().unwrap();
        assert_eq!(boxes.len(), 3);
    }

    #[test]
    fn test_remove_stopped_box() {
        let manager = BoxManager::new();
        let metadata = create_test_metadata("test-id", BoxState::Stopped);

        manager.register(metadata.clone()).unwrap();
        manager.remove(&metadata.id).unwrap();

        assert!(manager.get(&metadata.id).unwrap().is_none());
    }

    #[test]
    fn test_cannot_remove_running_box() {
        let manager = BoxManager::new();
        let metadata = create_test_metadata("test-id", BoxState::Running);

        manager.register(metadata.clone()).unwrap();
        let result = manager.remove(&metadata.id);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("active box"));
    }

    #[test]
    fn test_count() {
        let manager = BoxManager::new();

        assert_eq!(manager.count().unwrap(), 0);

        manager
            .register(create_test_metadata("id1", BoxState::Running))
            .unwrap();
        assert_eq!(manager.count().unwrap(), 1);

        manager
            .register(create_test_metadata("id2", BoxState::Running))
            .unwrap();
        assert_eq!(manager.count().unwrap(), 2);
    }
}
