//! Restart policy configuration for boxes.
//!
//! Defines when and how boxes are automatically restarted after crashes.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Maximum backoff delay cap.
///
/// The exponential backoff (1s, 2s, 4s, ...) grows quickly, so we cap it
/// to avoid excessively long delays between restart attempts.
/// 5 minutes = 300 seconds, which caps the sequence at restart count 9
/// (would be 512 seconds without the cap).
const MAX_BACKOFF: Duration = Duration::from_secs(5 * 60);

/// Base backoff delay (1 second).
const BASE_BACKOFF: Duration = Duration::from_secs(1);

/// Restart policy for a box.
///
/// Controls whether and how the box is automatically restarted when it crashes.
/// Similar to Docker's `--restart` flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RestartPolicy {
    /// No automatic restart.
    ///
    /// The box will not be restarted if it crashes.
    /// This is the default behavior.
    #[default]
    No,

    /// Always restart the box regardless of exit status.
    ///
    /// The box will be restarted indefinitely until manually stopped.
    Always,

    /// Restart only if the box crashes (non-zero exit code).
    ///
    /// If the box exits successfully (exit code 0), it will not be restarted.
    /// Takes an optional maximum number of restart attempts.
    OnFailure(Option<u32>),

    /// Restart only if the box crashes, but not if manually stopped.
    ///
    /// Similar to OnFailure, but respects manual stop requests.
    /// If the box is stopped via `stop()`, it will not be restarted.
    /// Takes an optional maximum number of restart attempts.
    UnlessStopped(Option<u32>),
}

/// Calculate exponential backoff delay for restart attempts.
///
/// Uses the formula: `min(base * 2^restart_count, max_delay)`
/// where `restart_count` is the number of restart attempts already made.
///
/// # Arguments
///
/// * `restart_count` - Number of restart attempts already made
///
/// # Returns
///
/// The duration to wait before the next restart attempt.
///
/// # Examples
///
/// ```
/// use boxlite::runtime::restart_policy::calculate_backoff;
/// use std::time::Duration;
///
/// // First restart (count=0): 1 second
/// assert_eq!(calculate_backoff(0), Duration::from_secs(1));
///
/// // Second restart (count=1): 2 seconds
/// assert_eq!(calculate_backoff(1), Duration::from_secs(2));
///
/// // Third restart (count=2): 4 seconds
/// assert_eq!(calculate_backoff(2), Duration::from_secs(4));
///
/// // After many restarts, caps at 5 minutes
/// assert!(calculate_backoff(100).as_secs() >= 300);
/// ```
pub fn calculate_backoff(restart_count: u32) -> Duration {
    // Calculate exponential backoff: base * 2^restart_count
    // Use saturating arithmetic to prevent overflow
    let delay_secs = BASE_BACKOFF
        .as_secs()
        .saturating_mul(2_u64.saturating_pow(restart_count));

    // Cap at maximum backoff
    Duration::from_secs(delay_secs.min(MAX_BACKOFF.as_secs()))
}

impl RestartPolicy {
    /// Check if restart is enabled for this policy.
    pub fn is_enabled(&self) -> bool {
        !matches!(self, RestartPolicy::No)
    }

    /// Get the maximum number of restart attempts.
    ///
    /// - None: Unlimited restart attempts
    /// - Some(n): Maximum of n restart attempts
    pub fn max_attempts(&self) -> Option<u32> {
        match self {
            RestartPolicy::No => None,
            RestartPolicy::Always => None,
            RestartPolicy::OnFailure(max) => *max,
            RestartPolicy::UnlessStopped(max) => *max,
        }
    }

    /// Check if restart should happen based on exit code.
    ///
    /// - No: Never restart
    /// - Always: Always restart
    /// - OnFailure: Restart only if exit_code != 0
    /// - UnlessStopped: Restart only if exit_code != 0 and not manually stopped
    pub fn should_restart(&self, exit_code: Option<i32>, manually_stopped: bool) -> bool {
        match self {
            RestartPolicy::No => false,
            RestartPolicy::Always => !manually_stopped,
            RestartPolicy::OnFailure(_) => {
                if manually_stopped {
                    return false;
                }
                // Restart if exit code indicates failure
                exit_code.unwrap_or(0) != 0 || exit_code.is_none()
            }
            RestartPolicy::UnlessStopped(_) => {
                !manually_stopped && (exit_code.unwrap_or(0) != 0 || exit_code.is_none())
            }
        }
    }

    /// Check if max attempts have been exceeded.
    ///
    /// Returns true if the restart count has reached or exceeded the maximum.
    ///
    /// # Arguments
    ///
    /// * `current_count` - Current restart count
    ///
    /// # Examples
    ///
    /// ```
    /// use boxlite::runtime::restart_policy::RestartPolicy;
    ///
    /// // Unlimited policy
    /// let policy = RestartPolicy::OnFailure(None);
    /// assert!(!policy.has_exceeded_max_attempts(100));
    ///
    /// // Limited to 5 attempts
    /// let policy = RestartPolicy::OnFailure(Some(5));
    /// assert!(!policy.has_exceeded_max_attempts(4));
    /// assert!(policy.has_exceeded_max_attempts(5));
    /// assert!(policy.has_exceeded_max_attempts(6));
    /// ```
    pub fn has_exceeded_max_attempts(&self, current_count: u32) -> bool {
        match self.max_attempts() {
            None => false, // Unlimited
            Some(max) => current_count >= max,
        }
    }
}

impl std::fmt::Display for RestartPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestartPolicy::No => write!(f, "no"),
            RestartPolicy::Always => write!(f, "always"),
            RestartPolicy::OnFailure(None) => write!(f, "on-failure"),
            RestartPolicy::OnFailure(Some(n)) => write!(f, "on-failure:{}", n),
            RestartPolicy::UnlessStopped(None) => write!(f, "unless-stopped"),
            RestartPolicy::UnlessStopped(Some(n)) => write!(f, "unless-stopped:{}", n),
        }
    }
}

impl std::str::FromStr for RestartPolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s_lower = s.to_lowercase();
        let (policy, count) = match s_lower.split_once(':') {
            Some((p, n)) => {
                let count = n.parse::<u32>().map_err(|_| {
                    format!("invalid restart count '{}': must be a positive integer", n)
                })?;
                (p, Some(count))
            }
            None => (s_lower.as_str(), None),
        };

        Ok(match policy {
            "no" => RestartPolicy::No,
            "always" => RestartPolicy::Always,
            "on-failure" | "on_failure" => RestartPolicy::OnFailure(count),
            "unless-stopped" | "unless_stopped" => RestartPolicy::UnlessStopped(count),
            _ => {
                return Err(format!("invalid restart policy: {}", s));
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let policy = RestartPolicy::default();
        assert_eq!(policy, RestartPolicy::No);
    }

    #[test]
    fn test_is_enabled() {
        assert!(!RestartPolicy::No.is_enabled());
        assert!(RestartPolicy::Always.is_enabled());
        assert!(RestartPolicy::OnFailure(None).is_enabled());
        assert!(RestartPolicy::UnlessStopped(None).is_enabled());
    }

    #[test]
    fn test_max_attempts() {
        // No and Always should have unlimited restarts
        assert!(RestartPolicy::No.max_attempts().is_none());
        assert!(RestartPolicy::Always.max_attempts().is_none());

        // OnFailure and UnlessStopped can have optional limits
        assert!(RestartPolicy::OnFailure(None).max_attempts().is_none());
        assert_eq!(RestartPolicy::OnFailure(Some(5)).max_attempts(), Some(5));
        assert_eq!(RestartPolicy::OnFailure(Some(10)).max_attempts(), Some(10));

        assert!(RestartPolicy::UnlessStopped(None).max_attempts().is_none());
        assert_eq!(
            RestartPolicy::UnlessStopped(Some(3)).max_attempts(),
            Some(3)
        );
    }

    #[test]
    fn test_should_restart_no() {
        let policy = RestartPolicy::No;

        // Never restart
        assert!(!policy.should_restart(Some(0), false));
        assert!(!policy.should_restart(Some(1), false));
        assert!(!policy.should_restart(None, false));
        assert!(!policy.should_restart(Some(1), true));
    }

    #[test]
    fn test_should_restart_always() {
        let policy = RestartPolicy::Always;

        // Always restart unless manually stopped
        assert!(policy.should_restart(Some(0), false));
        assert!(policy.should_restart(Some(1), false));
        assert!(policy.should_restart(None, false));
        assert!(!policy.should_restart(Some(0), true)); // Manually stopped
        assert!(!policy.should_restart(Some(1), true)); // Manually stopped
    }

    #[test]
    fn test_should_restart_on_failure() {
        let policy = RestartPolicy::OnFailure(None);

        // Restart on failure (non-zero exit)
        assert!(!policy.should_restart(Some(0), false)); // Success
        assert!(policy.should_restart(Some(1), false)); // Failure
        assert!(policy.should_restart(Some(255), false)); // Failure
        assert!(policy.should_restart(None, false)); // Unknown = failure

        // Don't restart if manually stopped
        assert!(!policy.should_restart(Some(1), true));
        assert!(!policy.should_restart(Some(0), true));

        // Test with max attempts
        let policy_limited = RestartPolicy::OnFailure(Some(5));
        // Behavior should be the same, just max_attempts different
        assert!(!policy_limited.should_restart(Some(0), false));
        assert!(policy_limited.should_restart(Some(1), false));
    }

    #[test]
    fn test_should_restart_unless_stopped() {
        let policy = RestartPolicy::UnlessStopped(None);

        // Restart on failure, unless manually stopped
        assert!(!policy.should_restart(Some(0), false)); // Success
        assert!(policy.should_restart(Some(1), false)); // Failure
        assert!(policy.should_restart(None, false)); // Unknown = failure

        // Don't restart if manually stopped
        assert!(!policy.should_restart(Some(1), true));
        assert!(!policy.should_restart(Some(0), true));

        // Test with max attempts
        let policy_limited = RestartPolicy::UnlessStopped(Some(3));
        assert!(!policy_limited.should_restart(Some(0), false));
        assert!(policy_limited.should_restart(Some(1), false));
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", RestartPolicy::No), "no");
        assert_eq!(format!("{}", RestartPolicy::Always), "always");
        assert_eq!(format!("{}", RestartPolicy::OnFailure(None)), "on-failure");
        assert_eq!(
            format!("{}", RestartPolicy::OnFailure(Some(5))),
            "on-failure:5"
        );
        assert_eq!(
            format!("{}", RestartPolicy::UnlessStopped(None)),
            "unless-stopped"
        );
        assert_eq!(
            format!("{}", RestartPolicy::UnlessStopped(Some(10))),
            "unless-stopped:10"
        );
    }

    #[test]
    fn test_from_str() {
        assert_eq!("no".parse::<RestartPolicy>().unwrap(), RestartPolicy::No);
        assert_eq!(
            "always".parse::<RestartPolicy>().unwrap(),
            RestartPolicy::Always
        );
        assert_eq!(
            "on-failure".parse::<RestartPolicy>().unwrap(),
            RestartPolicy::OnFailure(None)
        );
        assert_eq!(
            "on-failure:5".parse::<RestartPolicy>().unwrap(),
            RestartPolicy::OnFailure(Some(5))
        );
        assert_eq!(
            "unless-stopped".parse::<RestartPolicy>().unwrap(),
            RestartPolicy::UnlessStopped(None)
        );
        assert_eq!(
            "unless-stopped:10".parse::<RestartPolicy>().unwrap(),
            RestartPolicy::UnlessStopped(Some(10))
        );

        // Underscore variants
        assert_eq!(
            "on_failure".parse::<RestartPolicy>().unwrap(),
            RestartPolicy::OnFailure(None)
        );
        assert_eq!(
            "unless_stopped:3".parse::<RestartPolicy>().unwrap(),
            RestartPolicy::UnlessStopped(Some(3))
        );
    }

    #[test]
    fn test_from_str_case_insensitive() {
        assert_eq!("NO".parse::<RestartPolicy>().unwrap(), RestartPolicy::No);
        assert_eq!(
            "Always".parse::<RestartPolicy>().unwrap(),
            RestartPolicy::Always
        );
        assert_eq!(
            "ON-FAILURE".parse::<RestartPolicy>().unwrap(),
            RestartPolicy::OnFailure(None)
        );
    }

    #[test]
    fn test_from_str_invalid() {
        assert!("invalid".parse::<RestartPolicy>().is_err());
        assert!("".parse::<RestartPolicy>().is_err());
        // Invalid count
        assert!("on-failure:abc".parse::<RestartPolicy>().is_err());
        assert!("unless-stopped:-1".parse::<RestartPolicy>().is_err());
    }

    #[test]
    fn test_serde_roundtrip() {
        let policies = vec![
            RestartPolicy::No,
            RestartPolicy::Always,
            RestartPolicy::OnFailure(None),
            RestartPolicy::OnFailure(Some(5)),
            RestartPolicy::UnlessStopped(None),
            RestartPolicy::UnlessStopped(Some(10)),
        ];

        for policy in policies {
            let json = serde_json::to_string(&policy).unwrap();
            let deserialized: RestartPolicy = serde_json::from_str(&json).unwrap();
            assert_eq!(policy, deserialized);
        }
    }

    // ========================================================================
    // calculate_backoff tests
    // ========================================================================

    #[test]
    fn test_backoff_first_restart() {
        // First restart (count=0): 1 second
        assert_eq!(calculate_backoff(0), Duration::from_secs(1));
    }

    #[test]
    fn test_backoff_second_restart() {
        // Second restart (count=1): 2 seconds
        assert_eq!(calculate_backoff(1), Duration::from_secs(2));
    }

    #[test]
    fn test_backoff_third_restart() {
        // Third restart (count=2): 4 seconds
        assert_eq!(calculate_backoff(2), Duration::from_secs(4));
    }

    #[test]
    fn test_backoff_fourth_restart() {
        // Fourth restart (count=3): 8 seconds
        assert_eq!(calculate_backoff(3), Duration::from_secs(8));
    }

    #[test]
    fn test_backoff_fifth_restart() {
        // Fifth restart (count=4): 16 seconds
        assert_eq!(calculate_backoff(4), Duration::from_secs(16));
    }

    #[test]
    fn test_backoff_sixth_restart() {
        // Sixth restart (count=5): 32 seconds
        assert_eq!(calculate_backoff(5), Duration::from_secs(32));
    }

    #[test]
    fn test_backoff_seventh_restart() {
        // Seventh restart (count=6): 64 seconds
        assert_eq!(calculate_backoff(6), Duration::from_secs(64));
    }

    #[test]
    fn test_backoff_eighth_restart() {
        // Eighth restart (count=7): 128 seconds (2 min 8 sec)
        assert_eq!(calculate_backoff(7), Duration::from_secs(128));
    }

    #[test]
    fn test_backoff_ninth_restart() {
        // Ninth restart (count=8): 256 seconds (4 min 16 sec)
        assert_eq!(calculate_backoff(8), Duration::from_secs(256));
    }

    #[test]
    fn test_backoff_tenth_restart() {
        // Tenth restart (count=9): Would be 512 seconds, but caps at 300 seconds (5 minutes)
        assert_eq!(calculate_backoff(9), Duration::from_secs(300));
    }

    #[test]
    fn test_backoff_max_cap() {
        // After many restarts, should cap at 5 minutes
        let delay = calculate_backoff(100);
        assert_eq!(delay, Duration::from_secs(300));
    }

    #[test]
    fn test_backoff_sequence() {
        // Test the full sequence up to the cap
        let expected = vec![
            Duration::from_secs(1),   // 0
            Duration::from_secs(2),   // 1
            Duration::from_secs(4),   // 2
            Duration::from_secs(8),   // 3
            Duration::from_secs(16),  // 4
            Duration::from_secs(32),  // 5
            Duration::from_secs(64),  // 6
            Duration::from_secs(128), // 7
            Duration::from_secs(256), // 8
            Duration::from_secs(300), // 9 (capped)
            Duration::from_secs(300), // 10 (capped)
        ];

        for (i, expected_delay) in expected.iter().enumerate() {
            let delay = calculate_backoff(i as u32);
            assert_eq!(
                delay, *expected_delay,
                "Restart count {}: expected {:?}, got {:?}",
                i, expected_delay, delay
            );
        }
    }

    // ========================================================================
    // has_exceeded_max_attempts tests
    // ========================================================================

    #[test]
    fn test_has_exceeded_max_attempts_unlimited() {
        // Unlimited policy should never exceed
        let policy = RestartPolicy::OnFailure(None);
        assert!(!policy.has_exceeded_max_attempts(0));
        assert!(!policy.has_exceeded_max_attempts(100));
        assert!(!policy.has_exceeded_max_attempts(u32::MAX));
    }

    #[test]
    fn test_has_exceeded_max_attempts_limited() {
        // Limited to 5 attempts
        let policy = RestartPolicy::OnFailure(Some(5));
        assert!(!policy.has_exceeded_max_attempts(4));
        assert!(policy.has_exceeded_max_attempts(5));
        assert!(policy.has_exceeded_max_attempts(6));
    }

    #[test]
    fn test_has_exceeded_max_attempts_always() {
        // Always policy has no limit
        let policy = RestartPolicy::Always;
        assert!(!policy.has_exceeded_max_attempts(0));
        assert!(!policy.has_exceeded_max_attempts(1000));
    }

    #[test]
    fn test_has_exceeded_max_attempts_no() {
        // No policy has no limit (but won't restart anyway)
        let policy = RestartPolicy::No;
        assert!(!policy.has_exceeded_max_attempts(0));
        assert!(!policy.has_exceeded_max_attempts(1000));
    }
}
