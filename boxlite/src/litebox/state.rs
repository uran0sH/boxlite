//! Box lifecycle status and state machine.
//!
//! Defines the possible states of a box and valid transitions between them.

use crate::ContainerID;
use crate::lock::LockId;
use boxlite_shared::errors::{BoxliteError, BoxliteResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Lifecycle status of a box.
///
/// Represents the current operational state of a VM box.
/// Transitions between states are validated by the state machine.
///
/// State machine (two-layer model with runtime restart logic):
/// ```text
/// create() → Configured (persisted to DB, no VM)
/// start()  → Running (VM initialized)
/// stop()   → Stopping → Stopped (VM terminated, can restart)
///
/// With restart policy (runtime layer):
/// crash()  → Stopped (persisted)
///           → [runtime: evaluate policy]
///           → [runtime: backoff delay]
///           → Running (if restart allowed)
///           → Stopped (if restart denied or max exceeded)
/// ```
///
/// Exit reason is determined by BoxState fields:
/// - `last_exit_code == Some(0)`: normal exit
/// - `last_exit_code != Some(0)`: error exit
/// - `last_exit_code == None`: crashed (no exit code)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BoxStatus {
    /// Cannot determine box state (error recovery).
    Unknown,

    /// Box is created and persisted, but VM not yet started.
    /// No VM process allocated. Call start() or exec() to initialize.
    Configured,

    /// Box is running and guest server is accepting commands.
    Running,

    /// Box is shutting down gracefully (transient state).
    Stopping,

    /// Box is not running. VM process terminated.
    /// Rootfs is preserved, box can be restarted.
    ///
    /// Exit reason is determined by `last_exit_code`:
    /// - `Some(0)`: Normal exit
    /// - `Some(!0)`: Error exit
    /// - `None`: Crashed (process died without exit code)
    Stopped,
}

impl BoxStatus {
    /// Check if this status represents an active VM (process is running).
    pub fn is_active(&self) -> bool {
        matches!(self, BoxStatus::Running)
    }

    pub fn is_running(&self) -> bool {
        matches!(self, BoxStatus::Running)
    }

    pub fn is_configured(&self) -> bool {
        matches!(self, BoxStatus::Configured)
    }

    pub fn is_stopped(&self) -> bool {
        matches!(self, BoxStatus::Stopped)
    }

    /// Check if this status represents a transient state.
    /// Only Stopping is a transient state.
    pub fn is_transient(&self) -> bool {
        matches!(self, BoxStatus::Stopping)
    }

    /// Check if this status requires monitoring (only active boxes).
    pub fn requires_monitoring(&self) -> bool {
        matches!(self, BoxStatus::Running)
    }

    /// Check if this status is a terminal failure state.
    /// No terminal failure states - all can be recovered.
    pub fn is_terminal_failure(&self) -> bool {
        false
    }

    /// Check if restart is allowed from this state.
    pub fn can_restart(&self) -> bool {
        matches!(self, BoxStatus::Stopped)
    }

    /// Check if start() can be called from this state.
    /// Configured boxes need first start, Stopped boxes can restart.
    pub fn can_start(&self) -> bool {
        matches!(self, BoxStatus::Configured | BoxStatus::Stopped)
    }

    /// Check if stop() can be called from this state.
    /// Only running boxes can be stopped.
    pub fn can_stop(&self) -> bool {
        matches!(self, BoxStatus::Running)
    }

    /// Check if remove() can be called from this state.
    /// Configured, Stopped, and Unknown boxes can be removed.
    pub fn can_remove(&self) -> bool {
        matches!(
            self,
            BoxStatus::Configured | BoxStatus::Stopped | BoxStatus::Unknown
        )
    }

    /// Check if exec() can be called from this state.
    /// Configured and Stopped will trigger implicit start().
    pub fn can_exec(&self) -> bool {
        matches!(
            self,
            BoxStatus::Configured | BoxStatus::Running | BoxStatus::Stopped
        )
    }

    /// Check if transition to target state is valid.
    pub fn can_transition_to(&self, target: BoxStatus) -> bool {
        use BoxStatus::*;
        matches!(
            (self, target),
            // Unknown can transition to any state (recovery)
            (Unknown, _) |
            // Configured → Running (start success) or Stopped (start failed)
            (Configured, Running) |
            (Configured, Stopped) |
            (Configured, Unknown) |
            // Running → Stopping (graceful) or Stopped (crash/exit)
            (Running, Stopping) |
            (Running, Stopped) |
            (Running, Unknown) |
            // Stopping → Stopped (complete) or Unknown (error)
            (Stopping, Stopped) |
            (Stopping, Unknown) |
            // Stopped → Running (restart directly)
            (Stopped, Running) |
            (Stopped, Unknown)
        )
    }

    /// Convert to string for database storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            BoxStatus::Unknown => "unknown",
            BoxStatus::Configured => "configured",
            BoxStatus::Running => "running",
            BoxStatus::Stopping => "stopping",
            BoxStatus::Stopped => "stopped",
        }
    }
}

impl std::str::FromStr for BoxStatus {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "unknown" => Ok(BoxStatus::Unknown),
            "configured" => Ok(BoxStatus::Configured),
            // Legacy: support "starting" for backward compatibility with existing databases
            "starting" => Ok(BoxStatus::Configured),
            "running" => Ok(BoxStatus::Running),
            "stopping" => Ok(BoxStatus::Stopping),
            "stopped" => Ok(BoxStatus::Stopped),
            // Legacy: support old states for backward compatibility
            "crashed" | "restarting" | "failed" => Ok(BoxStatus::Stopped),
            _ => Err(()),
        }
    }
}

impl std::fmt::Display for BoxStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Dynamic box state (changes during lifecycle).
///
/// This is updated frequently and persisted to database.
/// State transitions are validated before applying.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoxState {
    /// Current lifecycle status.
    pub status: BoxStatus,
    pub pid: Option<u32>,
    pub container_id: Option<ContainerID>,
    /// Last state change timestamp (UTC).
    pub last_updated: DateTime<Utc>,
    /// Lock ID for multiprocess-safe locking.
    ///
    /// Allocated when the box is first initialized (not at creation time).
    /// Used to retrieve the lock across process restarts.
    pub lock_id: Option<LockId>,
    /// Number of times the box has been restarted.
    ///
    /// Incremented each time the box crashes and is restarted
    /// according to the restart policy. Reset to 0 when the box
    /// is explicitly stopped and started again.
    #[serde(default)]
    pub restart_count: u32,
    /// Timestamp of the last restart attempt (UTC).
    ///
    /// Set each time the box is restarted. Used for calculating
    /// exponential backoff delays between restart attempts.
    #[serde(default)]
    pub last_restarted_at: Option<DateTime<Utc>>,
    /// Reason for the last failure, if any.
    ///
    /// Set when the box fails after exceeding max restart attempts.
    /// Used for diagnostics and user feedback.
    #[serde(default)]
    pub failure_reason: Option<String>,
    /// Last process exit code.
    ///
    /// Used by restart policy to determine if restart is needed.
    /// OnFailure policy: restart only if exit_code != 0
    #[serde(default)]
    pub last_exit_code: Option<i32>,
    /// Whether the box was manually stopped.
    ///
    /// Used by UnlessStopped restart policy to prevent restart
    /// after manual stop. Reset to false on crash/restart.
    #[serde(default)]
    pub manually_stopped: bool,
}

impl BoxState {
    /// Create initial state for a new box.
    /// Box starts in Configured status (persisted, no VM yet).
    pub fn new() -> Self {
        Self {
            status: BoxStatus::Configured,
            pid: None,
            container_id: None,
            last_updated: Utc::now(),
            lock_id: None,
            restart_count: 0,
            last_restarted_at: None,
            failure_reason: None,
            last_exit_code: None,
            manually_stopped: false,
        }
    }

    /// Set lock ID and update timestamp.
    pub fn set_lock_id(&mut self, lock_id: LockId) {
        self.lock_id = Some(lock_id);
        self.last_updated = Utc::now();
    }

    /// Attempt state transition with validation.
    ///
    /// Returns error if the transition is not valid.
    pub fn transition_to(&mut self, new_status: BoxStatus) -> BoxliteResult<()> {
        if !self.status.can_transition_to(new_status) {
            return Err(BoxliteError::InvalidState(format!(
                "Cannot transition from {} to {}",
                self.status, new_status
            )));
        }

        self.status = new_status;
        self.last_updated = Utc::now();
        Ok(())
    }

    /// Force set status without validation (for recovery/internal use).
    pub fn force_status(&mut self, status: BoxStatus) {
        self.status = status;
        self.last_updated = Utc::now();
    }

    /// Set status directly (alias for force_status, used by manager).
    pub fn set_status(&mut self, status: BoxStatus) {
        self.force_status(status);
    }

    /// Set PID and update timestamp.
    pub fn set_pid(&mut self, pid: Option<u32>) {
        self.pid = pid;
        self.last_updated = Utc::now();
    }

    /// Mark box as stopped (VM process terminated).
    ///
    /// Called when the box stops (either gracefully or due to crash).
    /// PID is cleared since the process is no longer alive.
    /// Exit reason should be set via `last_exit_code` field.
    pub fn mark_stop(&mut self) {
        self.status = BoxStatus::Stopped;
        self.pid = None;
        self.last_updated = Utc::now();
    }

    /// Increment restart counter and update timestamp.
    ///
    /// Called each time a box is restarted according to its restart policy.
    pub fn increment_restart_count(&mut self) {
        self.restart_count += 1;
        self.last_restarted_at = Some(Utc::now());
        self.last_updated = Utc::now();
    }

    /// Reset restart counter.
    ///
    /// Called when a box is explicitly stopped and started again,
    /// clearing the restart history.
    pub fn reset_restart_count(&mut self) {
        self.restart_count = 0;
        self.last_restarted_at = None;
        self.last_updated = Utc::now();
    }

    /// Check if the box stopped cleanly (exit code 0).
    pub fn stopped_cleanly(&self) -> bool {
        self.status == BoxStatus::Stopped && self.last_exit_code == Some(0)
    }

    /// Check if the box stopped with error (non-zero exit code).
    pub fn stopped_with_error(&self) -> bool {
        self.status == BoxStatus::Stopped && self.last_exit_code.unwrap_or(0) != 0
    }

    /// Check if the box crashed (no exit code).
    pub fn crashed(&self) -> bool {
        self.status == BoxStatus::Stopped && self.last_exit_code.is_none()
    }

    /// Check if the box has exceeded max restart attempts.
    ///
    /// This is determined by comparing `restart_count` with the policy's max attempts.
    /// A box in this state cannot be auto-restarted without manual intervention.
    pub fn is_permanently_failed(&self) -> bool {
        self.status == BoxStatus::Stopped && self.restart_count > 0 && self.failure_reason.is_some()
    }

    /// Reset state after system reboot.
    ///
    /// Active boxes become Stopped since VM rootfs is preserved.
    /// PID is cleared since all processes are gone after reboot.
    /// Restart count and failure state are preserved.
    pub fn reset_for_reboot(&mut self) {
        if self.status.is_active() {
            self.status = BoxStatus::Stopped;
        }
        self.pid = None;
        self.last_updated = Utc::now();
    }
}

impl Default for BoxState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_is_active() {
        // Only Running is active (VM process running)
        assert!(!BoxStatus::Configured.is_active());
        assert!(BoxStatus::Running.is_active());
        assert!(!BoxStatus::Stopping.is_active());
        assert!(!BoxStatus::Stopped.is_active());
        assert!(!BoxStatus::Unknown.is_active());
    }

    #[test]
    fn test_status_is_configured() {
        assert!(BoxStatus::Configured.is_configured());
        assert!(!BoxStatus::Running.is_configured());
        assert!(!BoxStatus::Stopping.is_configured());
        assert!(!BoxStatus::Stopped.is_configured());
        assert!(!BoxStatus::Unknown.is_configured());
    }

    #[test]
    fn test_status_requires_monitoring() {
        // Only Running requires monitoring
        assert!(!BoxStatus::Configured.requires_monitoring());
        assert!(BoxStatus::Running.requires_monitoring());
        assert!(!BoxStatus::Stopping.requires_monitoring());
        assert!(!BoxStatus::Stopped.requires_monitoring());
        assert!(!BoxStatus::Unknown.requires_monitoring());
    }

    #[test]
    fn test_status_is_terminal_failure() {
        // No terminal failure states
        assert!(!BoxStatus::Configured.is_terminal_failure());
        assert!(!BoxStatus::Running.is_terminal_failure());
        assert!(!BoxStatus::Stopping.is_terminal_failure());
        assert!(!BoxStatus::Stopped.is_terminal_failure());
        assert!(!BoxStatus::Unknown.is_terminal_failure());
    }

    #[test]
    fn test_status_can_restart() {
        // Only Stopped can restart
        assert!(!BoxStatus::Configured.can_restart());
        assert!(!BoxStatus::Running.can_restart());
        assert!(!BoxStatus::Stopping.can_restart());
        assert!(BoxStatus::Stopped.can_restart());
        assert!(!BoxStatus::Unknown.can_restart());
    }

    #[test]
    fn test_status_can_start() {
        // Configured and Stopped can be started
        assert!(BoxStatus::Configured.can_start());
        assert!(!BoxStatus::Running.can_start());
        assert!(!BoxStatus::Stopping.can_start());
        assert!(BoxStatus::Stopped.can_start());
        assert!(!BoxStatus::Unknown.can_start());
    }

    #[test]
    fn test_status_can_stop() {
        // Only Running boxes can be stopped
        assert!(!BoxStatus::Configured.can_stop());
        assert!(BoxStatus::Running.can_stop());
        assert!(!BoxStatus::Stopping.can_stop());
        assert!(!BoxStatus::Stopped.can_stop());
        assert!(!BoxStatus::Unknown.can_stop());
    }

    #[test]
    fn test_status_can_exec() {
        // Configured, Running, and Stopped can exec
        assert!(BoxStatus::Configured.can_exec());
        assert!(BoxStatus::Running.can_exec());
        assert!(!BoxStatus::Stopping.can_exec());
        assert!(BoxStatus::Stopped.can_exec());
        assert!(!BoxStatus::Unknown.can_exec());
    }

    #[test]
    fn test_status_can_remove() {
        // Configured, Stopped, and Unknown can be removed
        assert!(BoxStatus::Configured.can_remove());
        assert!(!BoxStatus::Running.can_remove());
        assert!(!BoxStatus::Stopping.can_remove());
        assert!(BoxStatus::Stopped.can_remove());
        assert!(BoxStatus::Unknown.can_remove());
    }

    #[test]
    fn test_valid_transitions() {
        // Configured transitions
        assert!(BoxStatus::Configured.can_transition_to(BoxStatus::Running));
        assert!(BoxStatus::Configured.can_transition_to(BoxStatus::Stopped));
        assert!(!BoxStatus::Configured.can_transition_to(BoxStatus::Stopping));

        // Running transitions
        assert!(BoxStatus::Running.can_transition_to(BoxStatus::Stopping));
        assert!(BoxStatus::Running.can_transition_to(BoxStatus::Stopped));
        assert!(!BoxStatus::Running.can_transition_to(BoxStatus::Configured));

        // Stopping transitions
        assert!(BoxStatus::Stopping.can_transition_to(BoxStatus::Stopped));
        assert!(!BoxStatus::Stopping.can_transition_to(BoxStatus::Running));
        assert!(!BoxStatus::Stopping.can_transition_to(BoxStatus::Configured));

        // Stopped transitions - can go directly to Running
        assert!(BoxStatus::Stopped.can_transition_to(BoxStatus::Running));
        assert!(!BoxStatus::Stopped.can_transition_to(BoxStatus::Configured));
        assert!(!BoxStatus::Stopped.can_transition_to(BoxStatus::Stopping));

        // Unknown can go anywhere (recovery)
        assert!(BoxStatus::Unknown.can_transition_to(BoxStatus::Configured));
        assert!(BoxStatus::Unknown.can_transition_to(BoxStatus::Running));
        assert!(BoxStatus::Unknown.can_transition_to(BoxStatus::Stopped));
    }

    #[test]
    fn test_state_transition() {
        let mut state = BoxState::new();
        assert_eq!(state.status, BoxStatus::Configured);

        // Valid: Configured → Running
        assert!(state.transition_to(BoxStatus::Running).is_ok());
        assert_eq!(state.status, BoxStatus::Running);

        // Valid: Running → Stopping
        assert!(state.transition_to(BoxStatus::Stopping).is_ok());
        assert_eq!(state.status, BoxStatus::Stopping);

        // Valid: Stopping → Stopped
        assert!(state.transition_to(BoxStatus::Stopped).is_ok());
        assert_eq!(state.status, BoxStatus::Stopped);

        // Valid: Stopped → Running (direct restart)
        assert!(state.transition_to(BoxStatus::Running).is_ok());
        assert_eq!(state.status, BoxStatus::Running);
    }

    #[test]
    fn test_invalid_transition() {
        let mut state = BoxState::new();
        state.status = BoxStatus::Configured;

        // Invalid: Configured → Stopping (must go through Running)
        let result = state.transition_to(BoxStatus::Stopping);
        assert!(result.is_err());
        assert_eq!(state.status, BoxStatus::Configured); // Unchanged
    }

    #[test]
    fn test_reset_for_reboot() {
        let mut state = BoxState::new();
        state.status = BoxStatus::Running;
        state.pid = Some(12345);

        state.reset_for_reboot();

        assert_eq!(state.status, BoxStatus::Stopped);
        assert_eq!(state.pid, None);
    }

    #[test]
    fn test_reset_for_reboot_stopped() {
        let mut state = BoxState::new();
        state.status = BoxStatus::Stopped;
        state.pid = None;

        state.reset_for_reboot();

        // Stopped stays stopped
        assert_eq!(state.status, BoxStatus::Stopped);
    }

    #[test]
    fn test_reset_for_reboot_configured() {
        let mut state = BoxState::new();
        // Configured is not active, should stay configured
        assert_eq!(state.status, BoxStatus::Configured);

        state.reset_for_reboot();

        // Configured stays configured (no VM was running)
        assert_eq!(state.status, BoxStatus::Configured);
    }

    #[test]
    fn test_restart_count_increment() {
        let mut state = BoxState::new();
        assert_eq!(state.restart_count, 0);
        assert!(state.last_restarted_at.is_none());

        // Increment restart count
        state.increment_restart_count();

        assert_eq!(state.restart_count, 1);
        assert!(state.last_restarted_at.is_some());
    }

    #[test]
    fn test_restart_count_multiple_increments() {
        let mut state = BoxState::new();

        for i in 1..=5 {
            state.increment_restart_count();
            assert_eq!(state.restart_count, i);
        }

        assert_eq!(state.restart_count, 5);
    }

    #[test]
    fn test_restart_count_reset() {
        let mut state = BoxState::new();
        state.increment_restart_count();
        state.increment_restart_count();
        assert_eq!(state.restart_count, 2);

        state.reset_restart_count();

        assert_eq!(state.restart_count, 0);
        assert!(state.last_restarted_at.is_none());
    }

    #[test]
    fn test_reset_for_reboot_preserves_restart_state() {
        let mut state = BoxState::new();
        state.status = BoxStatus::Running;
        state.pid = Some(12345);
        state.increment_restart_count();
        state.increment_restart_count();
        assert_eq!(state.restart_count, 2);

        state.reset_for_reboot();

        // Restart count and failure state should be preserved
        assert_eq!(state.status, BoxStatus::Stopped);
        assert_eq!(state.restart_count, 2);
        assert!(state.last_restarted_at.is_some());
    }

    #[test]
    fn test_status_as_str() {
        assert_eq!(BoxStatus::Unknown.as_str(), "unknown");
        assert_eq!(BoxStatus::Configured.as_str(), "configured");
        assert_eq!(BoxStatus::Running.as_str(), "running");
        assert_eq!(BoxStatus::Stopping.as_str(), "stopping");
        assert_eq!(BoxStatus::Stopped.as_str(), "stopped");
    }

    #[test]
    fn test_status_from_str() {
        assert_eq!("unknown".parse(), Ok(BoxStatus::Unknown));
        assert_eq!("configured".parse(), Ok(BoxStatus::Configured));
        // Legacy support: "starting" maps to Configured
        assert_eq!("starting".parse(), Ok(BoxStatus::Configured));
        assert_eq!("running".parse(), Ok(BoxStatus::Running));
        assert_eq!("stopping".parse(), Ok(BoxStatus::Stopping));
        assert_eq!("stopped".parse(), Ok(BoxStatus::Stopped));
        // Legacy: old states map to Stopped
        assert_eq!("crashed".parse(), Ok(BoxStatus::Stopped));
        assert_eq!("restarting".parse(), Ok(BoxStatus::Stopped));
        assert_eq!("failed".parse(), Ok(BoxStatus::Stopped));
        assert!("invalid".parse::<BoxStatus>().is_err());
    }

    #[test]
    fn test_stopped_cleanly() {
        let mut state = BoxState::new();
        state.status = BoxStatus::Stopped;
        state.last_exit_code = Some(0);

        assert!(state.stopped_cleanly());
        assert!(!state.stopped_with_error());
        assert!(!state.crashed());
    }

    #[test]
    fn test_stopped_with_error() {
        let mut state = BoxState::new();
        state.status = BoxStatus::Stopped;
        state.last_exit_code = Some(1);

        assert!(!state.stopped_cleanly());
        assert!(state.stopped_with_error());
        assert!(!state.crashed());
    }

    #[test]
    fn test_crashed() {
        let mut state = BoxState::new();
        state.status = BoxStatus::Stopped;
        state.last_exit_code = None;

        assert!(!state.stopped_cleanly());
        assert!(!state.stopped_with_error());
        assert!(state.crashed());
    }

    #[test]
    fn test_is_permanently_failed() {
        let mut state = BoxState::new();
        state.status = BoxStatus::Stopped;
        state.restart_count = 5;
        state.failure_reason = Some("Exceeded max restart attempts".to_string());

        assert!(state.is_permanently_failed());

        // Not permanently failed if no failure reason
        state.failure_reason = None;
        assert!(!state.is_permanently_failed());

        // Not permanently failed if restart_count is 0
        state.restart_count = 0;
        state.failure_reason = Some("Test".to_string());
        assert!(!state.is_permanently_failed());
    }
}
