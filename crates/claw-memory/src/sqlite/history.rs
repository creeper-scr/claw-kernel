//! SQLite-backed conversation history storage.
//!
//! Provides low-level row storage for conversation history without any
//! dependency on higher-level crates. Role and content are stored as plain
//! strings; callers are responsible for type conversion.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::error::MemoryError;

/// A single stored conversation row (role/content as plain strings).
#[derive(Debug, Clone)]
pub struct HistoryRow {
    pub role: String,
    pub content: String,
    pub tool_calls_json: Option<String>,
}

impl HistoryRow {
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_calls_json: None,
        }
    }
}

/// SQLite-backed row store for conversation history.
///
/// Messages are stored in a namespaced table and cached in memory for fast
/// access. Use [`SqliteHistoryStore::open`] to create or open an existing database.
pub struct SqliteHistoryStore {
    conn: Mutex<Connection>,
    namespace: String,
    rows: Vec<HistoryRow>,
}

impl SqliteHistoryStore {
    /// Open (or create) a SQLite history database at the given path.
    ///
    /// The namespace isolates this history from other agents sharing the same db.
    pub fn open(path: impl AsRef<Path>, namespace: &str) -> Result<Self, MemoryError> {
        let conn = Connection::open(path.as_ref())
            .map_err(|e| MemoryError::Storage(format!("sqlite open failed: {e}")))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS history (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                namespace   TEXT    NOT NULL,
                role        TEXT    NOT NULL,
                content     TEXT    NOT NULL,
                tool_calls  TEXT,
                timestamp_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_history_ns_time
                ON history(namespace, timestamp_ms);",
        )
        .map_err(|e| MemoryError::Storage(format!("sqlite init failed: {e}")))?;

        // Load existing rows for this namespace.
        let rows = {
            let mut stmt = conn
                .prepare(
                    "SELECT role, content, tool_calls FROM history
                     WHERE namespace = ?1
                     ORDER BY timestamp_ms ASC",
                )
                .map_err(|e| MemoryError::Storage(format!("sqlite prepare failed: {e}")))?;

            let raw = stmt
                .query_map(params![namespace], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .map_err(|e| MemoryError::Storage(format!("sqlite query failed: {e}")))?;

            let mut rows = Vec::new();
            for r in raw {
                let (role, content, tool_calls_json) =
                    r.map_err(|e| MemoryError::Storage(format!("sqlite row error: {e}")))?;
                rows.push(HistoryRow { role, content, tool_calls_json });
            }
            rows
        };

        Ok(Self {
            conn: Mutex::new(conn),
            namespace: namespace.to_string(),
            rows,
        })
    }

    /// Append a row and persist it to SQLite.
    pub fn append(&mut self, row: HistoryRow) {
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT INTO history (namespace, role, content, tool_calls, timestamp_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    self.namespace,
                    row.role,
                    row.content,
                    row.tool_calls_json,
                    timestamp_ms
                ],
            );
        }
        self.rows.push(row);
    }

    /// All stored rows in insertion order.
    pub fn rows(&self) -> &[HistoryRow] {
        &self.rows
    }

    /// Number of stored rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Delete all rows for this namespace.
    pub fn clear(&mut self) {
        self.rows.clear();
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "DELETE FROM history WHERE namespace = ?1",
                params![self.namespace],
            );
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqlite_history_store_open_in_memory() {
        let mut store = SqliteHistoryStore::open(":memory:", "agent-1")
            .expect("should open in-memory db");

        assert!(store.is_empty());
        store.append(HistoryRow::new("user", "hello"));
        store.append(HistoryRow::new("assistant", "world"));
        assert_eq!(store.len(), 2);

        let rows = store.rows();
        assert_eq!(rows[0].content, "hello");
        assert_eq!(rows[1].content, "world");
    }

    #[test]
    fn test_sqlite_history_store_clear() {
        let mut store = SqliteHistoryStore::open(":memory:", "agent-1")
            .expect("should open");

        store.append(HistoryRow::new("user", "msg1"));
        store.append(HistoryRow::new("user", "msg2"));
        assert_eq!(store.len(), 2);

        store.clear();
        assert!(store.is_empty());
    }

    #[test]
    fn test_sqlite_history_store_persist_and_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.db");

        {
            let mut store = SqliteHistoryStore::open(&path, "agent-1").unwrap();
            store.append(HistoryRow::new("user", "persisted message"));
            store.append(HistoryRow::new("assistant", "persisted reply"));
        }

        let store2 = SqliteHistoryStore::open(&path, "agent-1").unwrap();
        assert_eq!(store2.len(), 2);
        assert_eq!(store2.rows()[0].content, "persisted message");
        assert_eq!(store2.rows()[1].content, "persisted reply");
    }

    #[test]
    fn test_history_row_tool_calls_json() {
        let mut row = HistoryRow::new("assistant", "result");
        row.tool_calls_json = Some(r#"[{"id":"1"}]"#.to_string());

        let mut store = SqliteHistoryStore::open(":memory:", "ns").unwrap();
        store.append(row);

        let stored = &store.rows()[0];
        assert_eq!(stored.tool_calls_json, Some(r#"[{"id":"1"}]"#.to_string()));
    }
}
