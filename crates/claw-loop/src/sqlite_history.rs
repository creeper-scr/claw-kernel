//! SQLite-backed conversation history.
//!
//! Thin wrapper around [`claw_memory::SqliteHistoryStore`] that converts
//! [`Message`] / [`Role`] to plain strings for storage and back on retrieval.
//! All rusqlite code lives inside `claw-memory`; this crate has zero direct
//! rusqlite dependency.

use std::path::Path;

use claw_memory::{HistoryRow, SqliteHistoryStore};
use claw_provider::types::{Message, Role};

use crate::{error::AgentError, traits::HistoryManager};

/// Persistent conversation history backed by SQLite via `claw-memory`.
///
/// Maintains an in-memory cache of `Message` values for the `&[Message]`
/// slice API while delegating all SQLite persistence to `SqliteHistoryStore`.
pub struct SqliteHistory {
    store: SqliteHistoryStore,
    /// In-memory cache mirroring the persisted rows. Rebuilt on open.
    messages: Vec<Message>,
    overflow_threshold: usize,
    overflow_callback: Option<Box<dyn Fn(usize, usize) + Send + Sync>>,
}

impl SqliteHistory {
    /// Open (or create) a SQLite history database at the given path.
    ///
    /// The namespace isolates this history from other agents sharing the same db.
    pub fn open(path: impl AsRef<Path>, namespace: &str) -> Result<Self, AgentError> {
        let store = SqliteHistoryStore::open(path, namespace)
            .map_err(|e| AgentError::Context(format!("sqlite history open failed: {e}")))?;

        // Reconstruct in-memory cache from persisted rows.
        let messages = store
            .rows()
            .iter()
            .map(row_to_message)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            store,
            messages,
            overflow_threshold: 100_000,
            overflow_callback: None,
        })
    }

    /// Create with a custom overflow threshold.
    pub fn with_overflow_threshold(mut self, threshold: usize) -> Self {
        self.overflow_threshold = threshold;
        self
    }

    fn check_overflow(&self) {
        if let Some(cb) = &self.overflow_callback {
            let estimate = self.token_estimate();
            if estimate >= self.overflow_threshold {
                cb(estimate, self.overflow_threshold);
            }
        }
    }
}

fn role_to_str(role: &Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
        Role::Tool => "tool",
    }
}

fn str_to_role(s: &str) -> Result<Role, AgentError> {
    match s {
        "user" => Ok(Role::User),
        "assistant" => Ok(Role::Assistant),
        "system" => Ok(Role::System),
        "tool" => Ok(Role::Tool),
        other => Err(AgentError::Context(format!(
            "unknown role in history: {other}"
        ))),
    }
}

fn row_to_message(row: &HistoryRow) -> Result<Message, AgentError> {
    let role = str_to_role(&row.role)?;
    let tool_calls = row
        .tool_calls_json
        .as_ref()
        .and_then(|j| serde_json::from_str(j).ok());
    Ok(Message {
        role,
        content: row.content.clone(),
        tool_calls,
        tool_call_id: None,
    })
}

impl HistoryManager for SqliteHistory {
    fn append(&mut self, message: Message) {
        let tool_calls_json = message
            .tool_calls
            .as_ref()
            .and_then(|tc| serde_json::to_string(tc).ok());

        let mut row = HistoryRow::new(role_to_str(&message.role), &message.content);
        row.tool_calls_json = tool_calls_json;

        self.store.append(row);
        self.messages.push(message);
        self.check_overflow();
    }

    fn messages(&self) -> &[Message] {
        &self.messages
    }

    fn len(&self) -> usize {
        self.messages.len()
    }

    fn token_estimate(&self) -> usize {
        self.messages
            .iter()
            .map(|m| {
                let content_tokens: f64 = m
                    .content
                    .chars()
                    .map(|c| if c.is_ascii() { 0.25 } else { 1.0 })
                    .sum();
                let tool_call_tokens = m.tool_calls.as_ref().map(|tc| tc.len() * 50).unwrap_or(0);
                (content_tokens as usize) + 1 + tool_call_tokens
            })
            .sum()
    }

    fn clear(&mut self) {
        self.messages.clear();
        self.store.clear();
    }

    fn set_overflow_callback(&mut self, f: Box<dyn Fn(usize, usize) + Send + Sync>) {
        self.overflow_callback = Some(f);
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqlite_history_open_in_memory() {
        let mut h =
            SqliteHistory::open(":memory:", "test-agent").expect("should open in-memory db");

        assert!(h.is_empty());
        h.append(Message::user("hello"));
        h.append(Message::assistant("world"));
        assert_eq!(h.len(), 2);

        let msgs = h.messages();
        assert_eq!(msgs[0].content, "hello");
        assert_eq!(msgs[1].content, "world");
    }

    #[test]
    fn test_sqlite_history_clear() {
        let mut h =
            SqliteHistory::open(":memory:", "test-agent").expect("should open in-memory db");

        h.append(Message::user("msg1"));
        h.append(Message::user("msg2"));
        assert_eq!(h.len(), 2);

        h.clear();
        assert!(h.is_empty());
    }

    #[test]
    fn test_sqlite_history_token_estimate() {
        let mut h =
            SqliteHistory::open(":memory:", "test-agent").expect("should open in-memory db");

        assert_eq!(h.token_estimate(), 0);
        // "hello" = 5 chars → 5*0.25 + 1 = 2
        h.append(Message::user("hello"));
        assert_eq!(h.token_estimate(), 2);
    }

    #[test]
    fn test_sqlite_history_persist_and_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.db");

        {
            let mut h = SqliteHistory::open(&path, "agent1").expect("open");
            h.append(Message::user("persisted message"));
            h.append(Message::assistant("persisted reply"));
        }

        let h2 = SqliteHistory::open(&path, "agent1").expect("reopen");
        assert_eq!(h2.len(), 2);
        assert_eq!(h2.messages()[0].content, "persisted message");
        assert_eq!(h2.messages()[1].content, "persisted reply");
    }
}
