//! Persistent trigger store backed by SQLite.
//!
//! Saves cron and webhook triggers across daemon restarts.

use std::path::PathBuf;
use rusqlite::{Connection, params};

/// Kind of trigger.
#[derive(Debug, Clone, PartialEq)]
pub enum TriggerKind {
    /// Cron-based recurring trigger.
    Cron,
    /// HTTP webhook trigger.
    Webhook,
}

/// A persisted trigger record.
#[derive(Debug, Clone)]
pub struct PersistedTrigger {
    /// Unique trigger identifier.
    pub trigger_id: String,
    /// Kind of trigger.
    pub kind: TriggerKind,
    /// Cron expression (for Cron triggers).
    pub cron_expr: Option<String>,
    /// Target agent ID.
    pub target_agent: String,
    /// Optional message/prompt injected when the trigger fires.
    pub message: Option<String>,
    /// HTTP endpoint path (for Webhook triggers).
    pub endpoint: Option<String>,
}

/// SQLite-backed trigger persistence store.
pub struct TriggerStore {
    conn: std::sync::Mutex<Connection>,
}

impl TriggerStore {
    /// Opens (or creates) the trigger database at the given path.
    pub fn open(path: &PathBuf) -> Result<Self, String> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create trigger store dir: {}", e))?;
        }
        let conn = Connection::open(path)
            .map_err(|e| format!("Failed to open trigger DB: {}", e))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS triggers (
                trigger_id   TEXT PRIMARY KEY,
                kind         TEXT NOT NULL,
                cron_expr    TEXT,
                target_agent TEXT NOT NULL,
                message      TEXT,
                endpoint     TEXT
            );"
        ).map_err(|e| format!("Failed to create triggers table: {}", e))?;
        Ok(Self { conn: std::sync::Mutex::new(conn) })
    }

    /// Saves a cron trigger. Overwrites if trigger_id exists.
    pub fn save_cron(
        &self,
        trigger_id: &str,
        cron_expr: &str,
        target_agent: &str,
        message: Option<&str>,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO triggers \
             (trigger_id, kind, cron_expr, target_agent, message, endpoint) \
             VALUES (?1, 'cron', ?2, ?3, ?4, NULL)",
            params![trigger_id, cron_expr, target_agent, message],
        ).map_err(|e| format!("Failed to save cron trigger: {}", e))?;
        Ok(())
    }

    /// Saves a webhook trigger. Overwrites if trigger_id exists.
    pub fn save_webhook(
        &self,
        trigger_id: &str,
        target_agent: &str,
        endpoint: &str,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO triggers \
             (trigger_id, kind, cron_expr, target_agent, message, endpoint) \
             VALUES (?1, 'webhook', NULL, ?2, NULL, ?3)",
            params![trigger_id, target_agent, endpoint],
        ).map_err(|e| format!("Failed to save webhook trigger: {}", e))?;
        Ok(())
    }

    /// Removes a trigger by ID.
    pub fn remove(&self, trigger_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM triggers WHERE trigger_id = ?1",
            params![trigger_id],
        ).map_err(|e| format!("Failed to remove trigger: {}", e))?;
        Ok(())
    }

    /// Loads all persisted triggers.
    pub fn load_all(&self) -> Result<Vec<PersistedTrigger>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT trigger_id, kind, cron_expr, target_agent, message, endpoint FROM triggers"
        ).map_err(|e| format!("Failed to prepare query: {}", e))?;

        let rows = stmt.query_map([], |row| {
            let kind_str: String = row.get(1)?;
            let kind = if kind_str == "cron" { TriggerKind::Cron } else { TriggerKind::Webhook };
            Ok(PersistedTrigger {
                trigger_id: row.get(0)?,
                kind,
                cron_expr: row.get(2)?,
                target_agent: row.get(3)?,
                message: row.get(4)?,
                endpoint: row.get(5)?,
            })
        }).map_err(|e| format!("Failed to query triggers: {}", e))?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| format!("Row error: {}", e))?);
        }
        Ok(result)
    }
}

impl std::fmt::Debug for TriggerStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TriggerStore").finish_non_exhaustive()
    }
}
