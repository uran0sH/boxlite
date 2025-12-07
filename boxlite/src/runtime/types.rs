//! Core data types for box lifecycle management.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use boxlite_shared::Transport;

/// Box identifier (ULID format for sortability).
///
/// ULIDs are 26-character strings that encode:
/// - 48-bit timestamp (millisecond precision)
/// - 80 bits of randomness
/// - Lexicographically sortable by creation time
///
/// Example: `01HJK4TNRPQSXYZ8WM6NCVT9R5`
pub type BoxID = String;

/// Generate a new ULID-based box ID.
pub fn generate_box_id() -> BoxID {
    ulid::Ulid::new().to_string()
}

/// Lifecycle state of a box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BoxState {
    /// Box subprocess is spawned but guest is not yet ready.
    Starting,

    /// Box is running and the guest server is accepting commands.
    Running,

    /// Box was shut down gracefully via `shutdown()`.
    Stopped,

    /// Box crashed, failed to start, or initialization timed out.
    Failed,
}

impl BoxState {
    /// Check if this state represents an active box.
    pub fn is_active(&self) -> bool {
        matches!(self, BoxState::Starting | BoxState::Running)
    }

    /// Check if this state represents a terminal state (no longer active).
    pub fn is_terminal(&self) -> bool {
        matches!(self, BoxState::Stopped | BoxState::Failed)
    }
}

/// Public metadata about a box (returned by list operations).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxInfo {
    /// Unique box identifier (ULID).
    pub id: BoxID,

    /// Current lifecycle state.
    pub state: BoxState,

    /// Creation timestamp (UTC).
    pub created_at: DateTime<Utc>,

    /// Process ID of the boxlite-shim subprocess (None if not started yet).
    pub pid: Option<u32>,

    /// Transport mechanism for guest communication.
    pub transport: Transport,

    /// Image reference or rootfs path.
    pub image: String,

    /// Allocated CPU count.
    pub cpus: u8,

    /// Allocated memory in MiB.
    pub memory_mib: u32,

    /// User-defined labels for filtering and organization.
    pub labels: HashMap<String, String>,
}

/// Internal metadata stored in the manager.
///
/// This contains all information needed to track a box,
/// including fields not exposed in the public API.
#[derive(Debug, Clone)]
#[allow(dead_code)] // engine_kind may be used in future phases
pub(crate) struct BoxMetadata {
    pub id: BoxID,
    pub state: BoxState,
    pub created_at: DateTime<Utc>,
    pub pid: Option<u32>,
    pub transport: Transport,

    // Original options used to create the box
    pub image: String,
    pub cpus: u8,
    pub memory_mib: u32,
    pub labels: HashMap<String, String>,

    // Internal tracking
    pub engine_kind: crate::vmm::VmmKind,
}

impl BoxMetadata {
    /// Convert internal metadata to public BoxInfo.
    pub fn to_info(&self) -> BoxInfo {
        BoxInfo {
            id: self.id.clone(),
            state: self.state,
            created_at: self.created_at,
            pid: self.pid,
            transport: self.transport.clone(),
            image: self.image.clone(),
            cpus: self.cpus,
            memory_mib: self.memory_mib,
            labels: self.labels.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_box_id() {
        let id1 = generate_box_id();
        let id2 = generate_box_id();

        // IDs should be 26 characters (ULID format)
        assert_eq!(id1.len(), 26);
        assert_eq!(id2.len(), 26);

        // IDs should be unique
        assert_ne!(id1, id2);

        // IDs should be sortable (later ID > earlier ID)
        assert!(id2 > id1);
    }

    #[test]
    fn test_box_state_is_active() {
        assert!(BoxState::Starting.is_active());
        assert!(BoxState::Running.is_active());
        assert!(!BoxState::Stopped.is_active());
        assert!(!BoxState::Failed.is_active());
    }

    #[test]
    fn test_box_state_is_terminal() {
        assert!(!BoxState::Starting.is_terminal());
        assert!(!BoxState::Running.is_terminal());
        assert!(BoxState::Stopped.is_terminal());
        assert!(BoxState::Failed.is_terminal());
    }

    #[test]
    fn test_metadata_to_info() {
        use std::path::PathBuf;

        let metadata = BoxMetadata {
            id: "01HJK4TNRPQSXYZ8WM6NCVT9R5".to_string(),
            state: BoxState::Running,
            created_at: Utc::now(),
            pid: Some(12345),
            transport: Transport::unix(PathBuf::from("/tmp/boxlite.sock")),
            image: "python:3.11".to_string(),
            cpus: 4,
            memory_mib: 1024,
            labels: HashMap::new(),
            engine_kind: crate::vmm::VmmKind::Libkrun,
        };

        let info = metadata.to_info();

        assert_eq!(info.id, metadata.id);
        assert_eq!(info.state, metadata.state);
        assert_eq!(info.pid, metadata.pid);
        assert_eq!(info.transport, metadata.transport);
        assert_eq!(info.image, metadata.image);
        assert_eq!(info.cpus, metadata.cpus);
        assert_eq!(info.memory_mib, metadata.memory_mib);
    }
}
