use crate::traits::HistoryManager;
use claw_provider::types::Message;

/// In-memory conversation history with optional overflow callback.
///
/// When `token_estimate()` exceeds `overflow_threshold`, the overflow callback is invoked.
/// The callback receives `(current_tokens, overflow_threshold)`.
///
/// This implementation is intentionally decoupled from any event bus or runtime —
/// callers register a plain closure instead.
pub struct InMemoryHistory {
    messages: Vec<Message>,
    overflow_threshold: usize,
    overflow_callback: Option<Box<dyn Fn(usize, usize) + Send + Sync>>,
}

impl InMemoryHistory {
    /// Create with the given token overflow threshold.
    pub fn new(overflow_threshold: usize) -> Self {
        Self {
            messages: Vec::new(),
            overflow_threshold,
            overflow_callback: None,
        }
    }

    /// Check if we're at or above the limit and invoke the callback if so.
    fn check_overflow(&self) {
        if let Some(cb) = &self.overflow_callback {
            let est = self.token_estimate();
            if est >= self.overflow_threshold {
                cb(est, self.overflow_threshold);
            }
        }
    }
}

impl Default for InMemoryHistory {
    /// Default threshold is 100 000 tokens.
    fn default() -> Self {
        Self::new(100_000)
    }
}

impl HistoryManager for InMemoryHistory {
    fn append(&mut self, message: Message) {
        self.messages.push(message);
        self.check_overflow();
    }

    fn messages(&self) -> &[Message] {
        &self.messages
    }

    fn len(&self) -> usize {
        self.messages.len()
    }

    /// Rough estimate: ASCII characters count as 0.25 tokens, CJK characters count as 1.0 tokens.
    /// Each message has +1 overhead. Each tool_call adds 50 tokens overhead.
    fn token_estimate(&self) -> usize {
        self.messages
            .iter()
            .map(|m| {
                // Count tokens based on character types
                let content_tokens: f64 = m
                    .content
                    .chars()
                    .map(|c| {
                        if c.is_ascii() {
                            0.25
                        } else {
                            // CJK and other non-ASCII characters
                            1.0
                        }
                    })
                    .sum();

                // Tool calls overhead: 50 tokens per tool_call
                let tool_call_tokens = m.tool_calls.as_ref().map(|tc| tc.len() * 50).unwrap_or(0);

                // +1 per message for overhead
                (content_tokens as usize) + 1 + tool_call_tokens
            })
            .sum()
    }

    fn clear(&mut self) {
        self.messages.clear();
    }

    fn set_overflow_callback(&mut self, f: Box<dyn Fn(usize, usize) + Send + Sync>) {
        self.overflow_callback = Some(f);
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_in_memory_history_new() {
        let h = InMemoryHistory::new(50_000);
        assert!(h.is_empty());
        assert_eq!(h.len(), 0);
        assert_eq!(h.messages().len(), 0);
    }

    #[test]
    fn test_in_memory_history_append() {
        let mut h = InMemoryHistory::default();
        assert!(h.is_empty());

        h.append(Message::user("hello"));
        assert_eq!(h.len(), 1);
        assert!(!h.is_empty());

        h.append(Message::assistant("world"));
        assert_eq!(h.len(), 2);

        let msgs = h.messages();
        assert_eq!(msgs[0].content, "hello");
        assert_eq!(msgs[1].content, "world");
    }

    #[test]
    fn test_in_memory_history_clear() {
        let mut h = InMemoryHistory::default();
        h.append(Message::user("msg1"));
        h.append(Message::user("msg2"));
        h.append(Message::user("msg3"));
        assert_eq!(h.len(), 3);

        h.clear();
        assert!(h.is_empty());
        assert_eq!(h.len(), 0);
        assert_eq!(h.messages().len(), 0);
    }

    #[test]
    fn test_in_memory_history_token_estimate() {
        let mut h = InMemoryHistory::default();
        // Empty — still 0.
        assert_eq!(h.token_estimate(), 0);

        // "hello" = 5 chars → 5/4 + 1 = 2
        h.append(Message::user("hello"));
        assert_eq!(h.token_estimate(), 2);

        // Add "world" = 5 chars → 5/4 + 1 = 2 → total 4
        h.append(Message::assistant("world"));
        assert_eq!(h.token_estimate(), 4);
    }

    #[test]
    fn test_in_memory_history_overflow_callback() {
        // Set a very low threshold so a single message triggers it.
        let mut h = InMemoryHistory::new(1);

        let calls: Arc<Mutex<Vec<(usize, usize)>>> = Arc::new(Mutex::new(Vec::new()));
        let calls_clone = calls.clone();

        h.set_overflow_callback(Box::new(move |cur, lim| {
            calls_clone.lock().unwrap().push((cur, lim));
        }));

        // Any message whose estimate >= 1 triggers the callback.
        h.append(Message::user("x"));
        // "x" → 1/4 + 1 = 1 token  → exactly at threshold, callback fires
        let recorded = calls.lock().unwrap();
        assert!(
            !recorded.is_empty(),
            "overflow callback should have been called"
        );
        assert_eq!(recorded[0].1, 1, "limit should match threshold");
    }
}
