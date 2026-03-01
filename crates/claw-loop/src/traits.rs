use crate::{error::AgentError, types::LoopState};
use async_trait::async_trait;
use claw_provider::types::Message;

/// Determines whether the agent loop should stop.
pub trait StopCondition: Send + Sync {
    /// Return true if the loop should stop given the current state.
    fn should_stop(&self, state: &LoopState) -> bool;

    /// Human-readable name for this condition (used in logs).
    fn name(&self) -> &str;
}

/// Manages the conversation history for an agent loop.
///
/// The overflow callback is a closure (not EventBus dependency)
/// to keep claw-loop decoupled from claw-runtime.
pub trait HistoryManager: Send + Sync {
    /// Append a message to history.
    fn append(&mut self, message: Message);

    /// Get all current messages.
    fn messages(&self) -> &[Message];

    /// Number of messages in history.
    fn len(&self) -> usize;

    /// Whether history is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Rough token estimate for the whole history.
    fn token_estimate(&self) -> usize;

    /// Clear all history.
    fn clear(&mut self);

    /// Set a callback invoked when history approaches the context limit.
    /// Callback receives (current_tokens: usize, limit: usize).
    fn set_overflow_callback(&mut self, f: Box<dyn Fn(usize, usize) + Send + Sync>);
}

/// Summarizes a set of messages into a shorter text.
#[async_trait]
pub trait Summarizer: Send + Sync {
    /// Summarize the given messages. Returns a concise summary string.
    async fn summarize(&self, messages: &[Message]) -> Result<String, AgentError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::LoopState;
    use claw_provider::types::{Message, TokenUsage};

    // ---------------------------------------------------------------------------
    // Mock implementations
    // ---------------------------------------------------------------------------

    struct MaxTurns(u32);

    impl StopCondition for MaxTurns {
        fn should_stop(&self, state: &LoopState) -> bool {
            state.turn >= self.0
        }

        fn name(&self) -> &str {
            "max_turns"
        }
    }

    struct SimpleHistory {
        msgs: Vec<Message>,
        cb: Option<Box<dyn Fn(usize, usize) + Send + Sync>>,
    }

    impl SimpleHistory {
        fn new() -> Self {
            Self {
                msgs: Vec::new(),
                cb: None,
            }
        }
    }

    impl HistoryManager for SimpleHistory {
        fn append(&mut self, msg: Message) {
            self.msgs.push(msg);
        }

        fn messages(&self) -> &[Message] {
            &self.msgs
        }

        fn len(&self) -> usize {
            self.msgs.len()
        }

        fn token_estimate(&self) -> usize {
            self.msgs.iter().map(|m| m.content.len() / 4).sum()
        }

        fn clear(&mut self) {
            self.msgs.clear();
        }

        fn set_overflow_callback(&mut self, f: Box<dyn Fn(usize, usize) + Send + Sync>) {
            self.cb = Some(f);
        }
    }

    // ---------------------------------------------------------------------------
    // StopCondition tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_stop_condition_max_turns() {
        let cond = MaxTurns(5);

        let mut state = LoopState::new();
        // Turn 0..=4 should NOT stop.
        for t in 0..5u32 {
            state.turn = t;
            assert!(!cond.should_stop(&state), "should not stop at turn {t}");
        }
        // Turn 5 and beyond should stop.
        state.turn = 5;
        assert!(cond.should_stop(&state), "should stop at turn 5");
        state.turn = 10;
        assert!(cond.should_stop(&state), "should stop at turn 10");
    }

    #[test]
    fn test_stop_condition_name() {
        let cond = MaxTurns(3);
        assert_eq!(cond.name(), "max_turns");
    }

    // ---------------------------------------------------------------------------
    // HistoryManager tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_history_manager_append_and_retrieve() {
        let mut history = SimpleHistory::new();
        assert!(history.is_empty());
        assert_eq!(history.len(), 0);

        history.append(Message::user("hello"));
        history.append(Message::assistant("world"));

        assert!(!history.is_empty());
        assert_eq!(history.len(), 2);

        let msgs = history.messages();
        assert_eq!(msgs[0].content, "hello");
        assert_eq!(msgs[1].content, "world");
    }

    #[test]
    fn test_history_manager_clear() {
        let mut history = SimpleHistory::new();
        history.append(Message::user("msg1"));
        history.append(Message::user("msg2"));
        history.append(Message::user("msg3"));
        assert_eq!(history.len(), 3);

        history.clear();
        assert_eq!(history.len(), 0);
        assert!(history.is_empty());
        assert_eq!(history.messages().len(), 0);
    }

    #[test]
    fn test_history_manager_overflow_callback() {
        use std::sync::{Arc, Mutex};

        let mut history = SimpleHistory::new();
        let calls: Arc<Mutex<Vec<(usize, usize)>>> = Arc::new(Mutex::new(Vec::new()));
        let calls_clone = calls.clone();

        history.set_overflow_callback(Box::new(move |current, limit| {
            calls_clone.lock().unwrap().push((current, limit));
        }));

        // Manually invoke the callback to verify it works.
        if let Some(ref cb) = history.cb {
            cb(1024, 4096);
            cb(3900, 4096);
        }

        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0], (1024, 4096));
        assert_eq!(recorded[1], (3900, 4096));
    }

    #[test]
    fn test_history_manager_token_estimate() {
        let mut history = SimpleHistory::new();
        // Empty history → 0 tokens.
        assert_eq!(history.token_estimate(), 0);

        // "hello world" = 11 chars → 11/4 = 2 tokens (integer division).
        history.append(Message::user("hello world"));
        assert_eq!(history.token_estimate(), 2);
    }
}
