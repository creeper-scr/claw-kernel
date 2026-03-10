//! Agent restart policy with exponential backoff.
//!
//! [`AgentRestartPolicy`] controls *how* (and *whether*) the orchestrator
//! restarts a failed agent.  The delay between successive restart attempts
//! grows exponentially, capped at [`AgentRestartPolicy::max_delay`].
//!
//! # Example
//!
//! ```rust
//! use claw_runtime::restart_policy::AgentRestartPolicy;
//! use std::time::Duration;
//!
//! let policy = AgentRestartPolicy::default();
//!
//! // First retry: 1s, second: 2s, third: 4s (capped at 60s).
//! assert_eq!(policy.delay_for_attempt(0), Duration::from_secs(1));
//! assert_eq!(policy.delay_for_attempt(1), Duration::from_secs(2));
//! assert_eq!(policy.delay_for_attempt(2), Duration::from_secs(4));
//! assert!(policy.should_restart(0));
//! assert!(!policy.should_restart(3)); // default max_retries = 3
//! ```

use std::time::Duration;

use crate::agent_types::AgentId;

// ─── AgentRestartPolicy ───────────────────────────────────────────────────────

/// Policy controlling how a failed agent is restarted, with exponential backoff.
///
/// This type is separate from the older [`RestartPolicy`](crate::orchestrator::RestartPolicy)
/// (which is stored per-orchestrator and governs the background task sweep).
/// `AgentRestartPolicy` is stored **per-agent** and drives precise per-agent
/// restart scheduling with a computed backoff delay.
#[derive(Debug, Clone)]
pub struct AgentRestartPolicy {
    /// Maximum number of restart attempts.  `0` means no automatic restart.
    pub max_retries: u32,
    /// Delay before the **first** restart attempt.
    pub initial_delay: Duration,
    /// Upper bound on the computed backoff delay.
    pub max_delay: Duration,
    /// Exponential multiplier applied each attempt (e.g. `2.0` doubles the delay).
    pub backoff_multiplier: f64,
}

impl Default for AgentRestartPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 2.0,
        }
    }
}

impl AgentRestartPolicy {
    /// Create a policy that never restarts the agent.
    pub fn never() -> Self {
        Self {
            max_retries: 0,
            ..Default::default()
        }
    }

    /// Create a policy with a custom retry limit and default backoff.
    pub fn with_max_retries(max_retries: u32) -> Self {
        Self {
            max_retries,
            ..Default::default()
        }
    }

    /// Calculate the delay to wait before the `attempt`-th restart (0-indexed).
    ///
    /// - `attempt = 0` → `initial_delay`
    /// - `attempt = 1` → `initial_delay * backoff_multiplier`
    /// - …capped at `max_delay`
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let multiplier = self.backoff_multiplier.powi(attempt as i32);
        let secs = (self.initial_delay.as_secs_f64() * multiplier)
            .min(self.max_delay.as_secs_f64());
        Duration::from_secs_f64(secs)
    }

    /// Returns `true` if the orchestrator should attempt another restart.
    ///
    /// `attempt` is the number of restart attempts **already made** (0 = no
    /// restarts yet).
    pub fn should_restart(&self, attempt: u32) -> bool {
        attempt < self.max_retries
    }
}

// ─── RestartState ─────────────────────────────────────────────────────────────

/// Per-agent restart tracking stored inside the orchestrator.
///
/// Created when an agent registers with a non-zero `AgentRestartPolicy` and
/// updated after each restart attempt.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct RestartState {
    /// The agent being tracked.
    pub agent_id: AgentId,
    /// Human-readable agent name (for logging).
    pub name: String,
    /// Number of restart attempts already made.
    pub attempt: u32,
    /// The policy governing restarts for this agent.
    pub policy: AgentRestartPolicy,
}

impl RestartState {
    /// Create a fresh restart state for an agent that has not yet been restarted.
    pub(crate) fn new(agent_id: AgentId, name: impl Into<String>, policy: AgentRestartPolicy) -> Self {
        Self {
            agent_id,
            name: name.into(),
            attempt: 0,
            policy,
        }
    }

    /// Whether another restart attempt is permitted.
    pub(crate) fn should_restart(&self) -> bool {
        self.policy.should_restart(self.attempt)
    }

    /// The delay to wait before the next restart.
    pub(crate) fn next_delay(&self) -> Duration {
        self.policy.delay_for_attempt(self.attempt)
    }

    /// Increment the attempt counter after scheduling a restart.
    pub(crate) fn record_attempt(&mut self) {
        self.attempt += 1;
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_policy_delays() {
        let p = AgentRestartPolicy::default();

        // attempt 0: 1s * 2^0 = 1s
        assert_eq!(p.delay_for_attempt(0), Duration::from_secs(1));
        // attempt 1: 1s * 2^1 = 2s
        assert_eq!(p.delay_for_attempt(1), Duration::from_secs(2));
        // attempt 2: 1s * 2^2 = 4s
        assert_eq!(p.delay_for_attempt(2), Duration::from_secs(4));
        // attempt 3: 1s * 2^3 = 8s
        assert_eq!(p.delay_for_attempt(3), Duration::from_secs(8));
    }

    #[test]
    fn test_delay_capped_at_max() {
        let p = AgentRestartPolicy {
            max_retries: 10,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(10),
            backoff_multiplier: 2.0,
        };

        // attempt 4: 1s * 2^4 = 16s → capped at 10s
        assert_eq!(p.delay_for_attempt(4), Duration::from_secs(10));
        assert_eq!(p.delay_for_attempt(10), Duration::from_secs(10));
    }

    #[test]
    fn test_should_restart() {
        let p = AgentRestartPolicy::default(); // max_retries = 3

        assert!(p.should_restart(0));
        assert!(p.should_restart(1));
        assert!(p.should_restart(2));
        assert!(!p.should_restart(3));
        assert!(!p.should_restart(100));
    }

    #[test]
    fn test_never_policy() {
        let p = AgentRestartPolicy::never();
        assert!(!p.should_restart(0));
    }

    #[test]
    fn test_restart_state_progression() {
        let id = AgentId::new("a1");
        let mut state = RestartState::new(id, "worker", AgentRestartPolicy::default());

        assert!(state.should_restart());
        assert_eq!(state.next_delay(), Duration::from_secs(1));

        state.record_attempt();
        assert!(state.should_restart());
        assert_eq!(state.next_delay(), Duration::from_secs(2));

        state.record_attempt();
        state.record_attempt();
        // 3 attempts made, max_retries = 3 → no more restarts
        assert!(!state.should_restart());
    }
}
