//! Execution state registry.
//!
//! Manages the state of all active executions, providing thread-safe access
//! to execution metadata, I/O channels, and completion status.

use crate::service::exec::state::ExecutionState;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Registry of active executions.
///
/// Thread-safe registry that stores execution state and provides
/// methods for registration, lookup, and lifecycle management.
#[derive(Clone)]
pub(crate) struct ExecutionRegistry {
    executions: Arc<Mutex<HashMap<String, ExecutionState>>>,
}

impl ExecutionRegistry {
    /// Create new registry.
    pub fn new() -> Self {
        Self {
            executions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Check if execution exists.
    pub async fn exists(&self, exec_id: &str) -> bool {
        self.executions.lock().await.contains_key(exec_id)
    }

    /// Get execution state.
    pub async fn get(&self, exec_id: &str) -> Option<ExecutionState> {
        self.executions.lock().await.get(exec_id).cloned()
    }

    /// Register new execution state.
    pub async fn register(&self, exec_id: String, state: ExecutionState) {
        self.executions.lock().await.insert(exec_id, state);
    }
}
