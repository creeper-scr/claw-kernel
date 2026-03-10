use crate::{error::AgentError, types::LoopState};
use async_trait::async_trait;
use claw_provider::types::{Message, TokenUsage};
use std::sync::Arc;

/// Determines whether the agent loop should stop.
///
/// Stop conditions are checked after each turn to determine if the agent
/// loop should continue or terminate. Multiple stop conditions can be
/// combined using logical OR semantics.
///
/// # Examples
///
/// Using built-in stop conditions:
///
/// ```rust
/// use claw_loop::{StopCondition, MaxTurns, TokenBudget, NoToolCall, LoopState};
/// use claw_provider::Message;
///
/// // Stop after 10 turns (tuple struct construction)
/// let max_turns = MaxTurns(10);
///
/// // Stop if token budget exceeds 100,000
/// let token_budget = TokenBudget(100_000);
///
/// // Stop when the LLM returns without making tool calls
/// let no_tool_call = NoToolCall;
///
/// // Check if a condition is met
/// let mut state = LoopState::new();
/// state.turn = 5;
/// assert!(!max_turns.should_stop(&state)); // Not yet at 10 turns
///
/// state.turn = 10;
/// assert!(max_turns.should_stop(&state)); // Now at 10 turns
///
/// // Token budget check
/// let mut state = LoopState::new();
/// state.usage.total_tokens = 50000;
/// assert!(!token_budget.should_stop(&state)); // Under budget
///
/// state.usage.total_tokens = 100_000;
/// assert!(token_budget.should_stop(&state)); // At budget
/// ```
///
/// Implementing a custom stop condition:
///
/// ```rust
/// use claw_loop::{StopCondition, LoopState};
///
/// /// Stop when a specific keyword appears in the conversation
/// struct KeywordStopCondition {
///     keyword: String,
/// }
///
/// impl KeywordStopCondition {
///     fn new(keyword: impl Into<String>) -> Self {
///         Self { keyword: keyword.into() }
///     }
/// }
///
/// impl StopCondition for KeywordStopCondition {
///     fn should_stop(&self, state: &LoopState) -> bool {
///         // In a real implementation, you might check the history
///         // for messages containing the keyword
///         false
///     }
///
///     fn name(&self) -> &str {
///         "keyword_stop"
///     }
/// }
/// ```
pub trait StopCondition: Send + Sync {
    /// Return true if the loop should stop given the current state.
    fn should_stop(&self, state: &LoopState) -> bool;

    /// Human-readable name for this condition (used in logs).
    fn name(&self) -> &str;

    /// Return true if the loop should stop given both the current state and
    /// the full conversation history.
    ///
    /// The default implementation ignores `history` and delegates to
    /// [`should_stop`](StopCondition::should_stop).  Override this method to
    /// implement history-aware stop conditions (e.g., stop when a specific
    /// keyword appears in the last assistant message).
    fn should_stop_with_history(&self, state: &LoopState, _history: &[Message]) -> bool {
        self.should_stop(state)
    }
}

/// Manages the conversation history for an agent loop.
///
/// The overflow callback is a closure (not EventBus dependency)
/// to keep claw-loop decoupled from claw-runtime.
///
/// # Examples
///
/// Using the built-in in-memory history:
///
/// ```rust
/// use claw_loop::{HistoryManager, InMemoryHistory};
/// use claw_provider::Message;
///
/// // Create a new history manager
/// let mut history = InMemoryHistory::new(4096); // 4K token limit
///
/// // Append messages
/// history.append(Message::user("Hello!"));
/// history.append(Message::assistant("Hi there!"));
///
/// // Check history state
/// assert_eq!(history.len(), 2);
/// assert!(!history.is_empty());
///
/// // Access messages
/// let messages = history.messages();
/// assert_eq!(messages[0].content, "Hello!");
/// assert_eq!(messages[1].content, "Hi there!");
///
/// // Get token estimate
/// let tokens = history.token_estimate();
///
/// // Clear history when needed
/// history.clear();
/// assert!(history.is_empty());
/// ```
///
/// Setting up overflow callback:
///
/// ```rust
/// use claw_loop::{HistoryManager, InMemoryHistory};
/// use std::sync::atomic::{AtomicUsize, Ordering};
///
/// let mut history = InMemoryHistory::new(1000);
///
/// // Set up a callback to handle approaching context limit
/// history.set_overflow_callback(Box::new(|current, limit| {
///     eprintln!("Warning: History at {}/{} tokens", current, limit);
/// }));
/// ```
///
/// Implementing a custom history manager:
///
/// ```rust
/// use claw_loop::HistoryManager;
/// use claw_provider::Message;
///
/// /// Simple history that keeps last N messages
/// struct RingBufferHistory {
///     messages: Vec<Message>,
///     max_messages: usize,
///     overflow_cb: Option<Box<dyn Fn(usize, usize) + Send + Sync>>,
/// }
///
/// impl RingBufferHistory {
///     fn new(max_messages: usize) -> Self {
///         Self {
///             messages: Vec::new(),
///             max_messages,
///             overflow_cb: None,
///         }
///     }
/// }
///
/// impl HistoryManager for RingBufferHistory {
///     fn append(&mut self, message: Message) {
///         if self.messages.len() >= self.max_messages {
///             self.messages.remove(0); // Remove oldest
///         }
///         self.messages.push(message);
///     }
///
///     fn messages(&self) -> &[Message] {
///         &self.messages
///     }
///
///     fn len(&self) -> usize {
///         self.messages.len()
///     }
///
///     fn token_estimate(&self) -> usize {
///         // Simple estimation: 4 characters per token
///         self.messages.iter()
///             .map(|m| m.content.len() / 4)
///             .sum()
///     }
///
///     fn clear(&mut self) {
///         self.messages.clear();
///     }
///
///     fn set_overflow_callback(&mut self, f: Box<dyn Fn(usize, usize) + Send + Sync>) {
///         self.overflow_cb = Some(f);
///     }
/// }
/// ```
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
///
/// Summarizers are used to condense conversation history when it grows
/// too large for the context window. The `SimpleSummarizer` uses the
/// LLM itself to generate summaries.
///
/// # Examples
///
/// Using the built-in summarizer:
///
/// ```rust,ignore
/// use claw_loop::{Summarizer, SimpleSummarizer, Message};
/// use claw_provider::OllamaProvider;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Create a summarizer with a provider
/// let provider = OllamaProvider::new("llama3");
/// let summarizer = SimpleSummarizer::new(provider);
///
/// // Summarize a batch of messages
/// let messages = vec![
///     Message::user("What's the weather?"),
///     Message::assistant("It's sunny today."),
///     Message::user("Thanks!"),
/// ];
///
/// let summary = summarizer.summarize(&messages).await?;
/// println!("Summary: {}", summary);
/// # Ok(())
/// # }
/// ```
///
/// Implementing a custom summarizer:
///
/// ```rust
/// use claw_loop::{Summarizer, AgentError};
/// use claw_provider::Message;
/// use async_trait::async_trait;
///
/// /// A simple summarizer that extracts the first sentence of each message
/// struct ExtractiveSummarizer;
///
/// #[async_trait]
/// impl Summarizer for ExtractiveSummarizer {
///     async fn summarize(&self, messages: &[Message]) -> Result<String, AgentError> {
///         let mut summary_parts = Vec::new();
///
///         for msg in messages {
///             // Take first sentence or first 50 chars
///             let excerpt: String = msg.content
///                 .split('.')
///                 .next()
///                 .unwrap_or(&msg.content)
///                 .chars()
///                 .take(50)
///                 .collect();
///
///             summary_parts.push(format!("[{:?}]: {}", msg.role, excerpt));
///         }
///
///         Ok(summary_parts.join("; "))
///     }
/// }
///
/// # async fn example() -> Result<(), AgentError> {
/// let summarizer = ExtractiveSummarizer;
/// let messages = vec![
///     Message::user("Hello world. This is a test."),
///     Message::assistant("Hi! Nice to meet you."),
/// ];
///
/// let summary = summarizer.summarize(&messages).await?;
/// assert!(summary.contains("Hello world"));
/// # Ok(())
/// # }
/// ```
#[async_trait]
pub trait Summarizer: Send + Sync {
    /// Summarize the given messages. Returns a concise summary string.
    async fn summarize(&self, messages: &[Message]) -> Result<String, AgentError>;
}

/// Event publisher for agent loop lifecycle events.
///
/// This trait allows Layer 1 (claw-runtime) to inject EventBus capabilities
/// into Layer 2 (claw-loop) without creating a circular dependency.
/// The runtime implements this trait and passes it to the AgentLoopBuilder.
///
/// # Example
///
/// ```rust,ignore
/// use claw_loop::EventPublisher;
/// use claw_runtime::{EventBus, events::Event, agent_types::AgentId};
///
/// struct RuntimeEventPublisher {
///     event_bus: Arc<EventBus>,
/// }
///
/// impl EventPublisher for RuntimeEventPublisher {
///     fn publish_turn_started(&self, agent_id: &str, turn: u32) {
///         let _ = self.event_bus.publish(Event::Custom {
///             event_type: "agent.turn.started".to_string(),
///             data: serde_json::json!({ "agent_id": agent_id, "turn": turn }),
///         });
///     }
///     // ... other methods
/// }
/// ```
pub trait EventPublisher: Send + Sync {
    /// Publish event when agent loop starts a new turn.
    fn publish_turn_started(&self, agent_id: &str, turn: u32);

    /// Publish event when LLM request is sent.
    fn publish_llm_request(&self, agent_id: &str, provider: &str, model: &str, message_count: usize);

    /// Publish event when LLM response is received.
    fn publish_llm_response(
        &self,
        agent_id: &str,
        provider: &str,
        usage: TokenUsage,
        finish_reason: &str,
    );

    /// Publish event when a tool is called.
    fn publish_tool_called(&self, agent_id: &str, tool_name: &str, call_id: &str);

    /// Publish event when a tool result is received.
    fn publish_tool_result(&self, agent_id: &str, tool_name: &str, call_id: &str, success: bool);

    /// Publish event when agent loop completes.
    fn publish_loop_completed(&self, agent_id: &str, reason: &str, total_turns: u32);
}

/// No-op event publisher for testing or when event publishing is not needed.
pub struct NoopEventPublisher;

impl EventPublisher for NoopEventPublisher {
    fn publish_turn_started(&self, _agent_id: &str, _turn: u32) {}
    fn publish_llm_request(&self, _agent_id: &str, _provider: &str, _model: &str, _message_count: usize) {}
    fn publish_llm_response(&self, _agent_id: &str, _provider: &str, _usage: TokenUsage, _finish_reason: &str) {}
    fn publish_tool_called(&self, _agent_id: &str, _tool_name: &str, _call_id: &str) {}
    fn publish_tool_result(&self, _agent_id: &str, _tool_name: &str, _call_id: &str, _success: bool) {}
    fn publish_loop_completed(&self, _agent_id: &str, _reason: &str, _total_turns: u32) {}
}

impl NoopEventPublisher {
    /// Create a new no-op event publisher wrapped in Arc.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> Arc<dyn EventPublisher> {
        Arc::new(NoopEventPublisher)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::LoopState;
    use claw_provider::types::Message;

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
