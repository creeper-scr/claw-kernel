use crate::{traits::StopCondition, types::LoopState};

/// Stop the loop after N turns have been executed.
pub struct MaxTurns(pub u32);

impl StopCondition for MaxTurns {
    fn should_stop(&self, state: &LoopState) -> bool {
        state.turn >= self.0
    }

    fn name(&self) -> &str {
        "max_turns"
    }
}

/// Stop the loop once cumulative token usage meets or exceeds the budget.
///
/// A budget of 0 means "no limit" and this condition will never trigger.
pub struct TokenBudget(pub u64);

impl StopCondition for TokenBudget {
    fn should_stop(&self, state: &LoopState) -> bool {
        self.0 > 0 && state.usage.total_tokens >= self.0
    }

    fn name(&self) -> &str {
        "token_budget"
    }
}

/// Marker condition that signals the loop should stop when the last LLM
/// response contained no tool calls (pure text reply).
///
/// The actual detection logic lives inside `AgentLoop::run` — this struct
/// exists so callers can reference the condition by name in logs / config.
/// Its `should_stop` implementation always returns `false`; the loop handles
/// the no-tool-call exit path directly.
pub struct NoToolCall;

impl StopCondition for NoToolCall {
    fn should_stop(&self, _state: &LoopState) -> bool {
        false
    }

    fn name(&self) -> &str {
        "no_tool_call"
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use claw_provider::types::TokenUsage;

    fn state_with(turn: u32, total_tokens: u64) -> LoopState {
        LoopState {
            turn,
            usage: TokenUsage { prompt_tokens: 0, completion_tokens: 0, total_tokens },
            history_len: 0,
        }
    }

    // ── MaxTurns ─────────────────────────────────────────────────────────────

    #[test]
    fn test_max_turns_not_stop() {
        let cond = MaxTurns(5);
        // Turns 0..=4 must not trigger stop.
        for t in 0..5u32 {
            assert!(!cond.should_stop(&state_with(t, 0)), "should not stop at turn {t}");
        }
    }

    #[test]
    fn test_max_turns_stop() {
        let cond = MaxTurns(5);
        // Turn 5 and beyond must stop.
        assert!(cond.should_stop(&state_with(5, 0)));
        assert!(cond.should_stop(&state_with(100, 0)));
        assert_eq!(cond.name(), "max_turns");
    }

    // ── TokenBudget ──────────────────────────────────────────────────────────

    #[test]
    fn test_token_budget_not_stop() {
        let cond = TokenBudget(1000);
        // 999 tokens used — must not stop.
        assert!(!cond.should_stop(&state_with(0, 999)));
    }

    #[test]
    fn test_token_budget_stop() {
        let cond = TokenBudget(1000);
        // Exactly at budget → stop.
        assert!(cond.should_stop(&state_with(0, 1000)));
        // Over budget → stop.
        assert!(cond.should_stop(&state_with(0, 1500)));
        assert_eq!(cond.name(), "token_budget");
    }

    #[test]
    fn test_token_budget_zero_budget_never_stops() {
        // Budget of 0 means "unlimited".
        let cond = TokenBudget(0);
        assert!(!cond.should_stop(&state_with(0, 0)));
        assert!(!cond.should_stop(&state_with(0, u64::MAX)));
    }

    // ── NoToolCall ───────────────────────────────────────────────────────────

    #[test]
    fn test_no_tool_call_never_stops_via_trait() {
        let cond = NoToolCall;
        // should_stop is always false; loop handles this exit path directly.
        assert!(!cond.should_stop(&state_with(0, 0)));
        assert!(!cond.should_stop(&state_with(100, 999_999)));
        assert_eq!(cond.name(), "no_tool_call");
    }
}
