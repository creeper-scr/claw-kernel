//! TriggerStore — SQLite 持久化触发器记录（GAP-F6-02）。
//!
//! 将所有注册的触发器（Cron、Webhook、Event）写入 SQLite 文件，
//! 保证重启后可通过 [`TriggerStore::list_all`] 完整恢复。
//!
//! 存储路径：`~/.local/share/claw-kernel/triggers.db`
//!
//! # 设计决策
//!
//! - 单表、单文件，数据量极小，无需连接池；
//! - 使用 `std::sync::Mutex` 保护 `rusqlite::Connection`（非 async 驱动）；
//! - 调用方（`TokioScheduler`）在 tokio 线程池中以 `spawn_blocking` 包裹写操作；
//! - `config` 字段以 JSON 文本存储，保持模式演化灵活性。
//!
//! # 示例
//!
//! ```rust,no_run
//! use claw_runtime::trigger_store::{TriggerStore, TriggerRecord};
//! use std::path::Path;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let store = TriggerStore::open(Path::new("/tmp/triggers.db"))?;
//!
//! let record = TriggerRecord {
//!     trigger_id: "daily-report".to_string(),
//!     trigger_type: "cron".to_string(),
//!     config: serde_json::json!({ "expr": "0 9 * * *" }),
//!     target_agent: None,
//!     created_at: 0,
//!     enabled: true,
//!     last_fired_at: None,
//!     fire_count: 0,
//! };
//!
//! store.save(&record)?;
//!
//! let all = store.list_all()?;
//! assert!(!all.is_empty());
//! # Ok(())
//! # }
//! ```

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};
use thiserror::Error;

// ─── Error ────────────────────────────────────────────────────────────────────

/// 触发器持久化错误。
#[derive(Debug, Error)]
pub enum TriggerStoreError {
    /// SQLite 操作失败。
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// JSON 序列化/反序列化失败。
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// 锁中毒（内部 panic 导致）。
    #[error("mutex poisoned")]
    Poisoned,
}

// ─── TriggerRecord ────────────────────────────────────────────────────────────

/// 持久化存储的触发器记录。
///
/// 每条记录对应一个已注册的触发器配置，由 [`TriggerStore`] 负责读写。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TriggerRecord {
    /// 触发器唯一 ID（主键），与调度器中的 `TaskId` 一致。
    pub trigger_id: String,

    /// 触发类型字符串：`"cron"` | `"webhook"` | `"event"`。
    pub trigger_type: String,

    /// 触发器配置（JSON）。
    /// - Cron：`{ "expr": "0 9 * * *" }`
    /// - Webhook：`{ "path": "/hooks/gh" }`
    /// - Event：`{ "event_name": "agent.completed" }`
    pub config: serde_json::Value,

    /// 目标 Agent ID；`None` 表示广播到所有 Agent。
    pub target_agent: Option<String>,

    /// 创建时间戳（Unix 秒）。
    pub created_at: i64,

    /// 是否启用（未来用于软删除 / 临时禁用）。
    pub enabled: bool,

    /// 最近一次触发的时间戳（Unix 秒）；首次触发前为 `None`。
    pub last_fired_at: Option<i64>,

    /// 累计触发次数。
    pub fire_count: i64,
}

// ─── TriggerStore ─────────────────────────────────────────────────────────────

/// SQLite 持久化触发器仓库。
///
/// 内部使用 `Mutex<Connection>` 保护单一 SQLite 连接。
/// 多线程访问安全，但写操作会短暂阻塞。
///
/// 调用方在 async 上下文中应通过 `tokio::task::spawn_blocking` 包裹写操作：
///
/// ```rust,ignore
/// let store = Arc::clone(&store);
/// tokio::task::spawn_blocking(move || store.save(&record)).await??;
/// ```
pub struct TriggerStore {
    conn: Mutex<Connection>,
}

impl TriggerStore {
    /// 打开（或创建）指定路径的 SQLite 数据库，并初始化表结构。
    ///
    /// 如果父目录不存在，会自动创建。
    pub fn open(path: &Path) -> Result<Self, TriggerStoreError> {
        // 确保父目录存在
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let conn = Connection::open(path)?;

        // WAL 模式提升并发读性能，减少写锁争用
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        // 初始化表（幂等）
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS triggers (
                trigger_id   TEXT    PRIMARY KEY,
                trigger_type TEXT    NOT NULL,
                config       TEXT    NOT NULL,
                target_agent TEXT,
                created_at   INTEGER NOT NULL,
                enabled      INTEGER NOT NULL DEFAULT 1,
                last_fired_at INTEGER,
                fire_count   INTEGER NOT NULL DEFAULT 0
            );",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 保存（upsert）一条触发器记录。
    ///
    /// 若 `trigger_id` 已存在则覆盖全部字段（`INSERT OR REPLACE`）。
    pub fn save(&self, record: &TriggerRecord) -> Result<(), TriggerStoreError> {
        let config_str = serde_json::to_string(&record.config)?;
        let conn = self.conn.lock().map_err(|_| TriggerStoreError::Poisoned)?;
        conn.execute(
            "INSERT OR REPLACE INTO triggers
                (trigger_id, trigger_type, config, target_agent, created_at,
                 enabled, last_fired_at, fire_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                record.trigger_id,
                record.trigger_type,
                config_str,
                record.target_agent,
                record.created_at,
                record.enabled as i64,
                record.last_fired_at,
                record.fire_count,
            ],
        )?;
        Ok(())
    }

    /// 删除指定触发器记录。若记录不存在则为空操作（幂等）。
    pub fn delete(&self, trigger_id: &str) -> Result<(), TriggerStoreError> {
        let conn = self.conn.lock().map_err(|_| TriggerStoreError::Poisoned)?;
        conn.execute(
            "DELETE FROM triggers WHERE trigger_id = ?1",
            params![trigger_id],
        )?;
        Ok(())
    }

    /// 返回所有已启用的触发器记录（`enabled = 1`）。
    ///
    /// 在调度器启动时调用以恢复持久化的触发器。
    pub fn list_all(&self) -> Result<Vec<TriggerRecord>, TriggerStoreError> {
        let conn = self.conn.lock().map_err(|_| TriggerStoreError::Poisoned)?;
        let mut stmt = conn.prepare(
            "SELECT trigger_id, trigger_type, config, target_agent,
                    created_at, enabled, last_fired_at, fire_count
             FROM triggers
             WHERE enabled = 1
             ORDER BY created_at ASC",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,   // trigger_id
                row.get::<_, String>(1)?,   // trigger_type
                row.get::<_, String>(2)?,   // config (JSON text)
                row.get::<_, Option<String>>(3)?, // target_agent
                row.get::<_, i64>(4)?,      // created_at
                row.get::<_, i64>(5)?,      // enabled
                row.get::<_, Option<i64>>(6)?, // last_fired_at
                row.get::<_, i64>(7)?,      // fire_count
            ))
        })?;

        let mut records = Vec::new();
        for row in rows {
            let (trigger_id, trigger_type, config_str, target_agent,
                 created_at, enabled, last_fired_at, fire_count) = row?;

            let config: serde_json::Value = serde_json::from_str(&config_str)
                .map_err(TriggerStoreError::Json)?;

            records.push(TriggerRecord {
                trigger_id,
                trigger_type,
                config,
                target_agent,
                created_at,
                enabled: enabled != 0,
                last_fired_at,
                fire_count,
            });
        }

        Ok(records)
    }

    /// 更新触发器的最近触发时间和累计触发次数。
    ///
    /// 在每次触发器触发后调用，记录触发历史。
    /// 若 `trigger_id` 不存在则为空操作（幂等）。
    pub fn update_last_fired(
        &self,
        trigger_id: &str,
        fired_at: i64,
    ) -> Result<(), TriggerStoreError> {
        let conn = self.conn.lock().map_err(|_| TriggerStoreError::Poisoned)?;
        conn.execute(
            "UPDATE triggers
             SET last_fired_at = ?1, fire_count = fire_count + 1
             WHERE trigger_id = ?2",
            params![fired_at, trigger_id],
        )?;
        Ok(())
    }

    /// 设置触发器的启用状态（软禁用 / 重新启用）。
    pub fn set_enabled(&self, trigger_id: &str, enabled: bool) -> Result<(), TriggerStoreError> {
        let conn = self.conn.lock().map_err(|_| TriggerStoreError::Poisoned)?;
        conn.execute(
            "UPDATE triggers SET enabled = ?1 WHERE trigger_id = ?2",
            params![enabled as i64, trigger_id],
        )?;
        Ok(())
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn make_store() -> (TriggerStore, NamedTempFile) {
        let f = NamedTempFile::new().unwrap();
        let store = TriggerStore::open(f.path()).unwrap();
        (store, f)
    }

    fn cron_record(id: &str) -> TriggerRecord {
        TriggerRecord {
            trigger_id: id.to_string(),
            trigger_type: "cron".to_string(),
            config: serde_json::json!({ "expr": "0 9 * * *" }),
            target_agent: None,
            created_at: 1_700_000_000,
            enabled: true,
            last_fired_at: None,
            fire_count: 0,
        }
    }

    // ── 1. open 创建表结构 ────────────────────────────────────────────────────
    #[test]
    fn test_open_creates_table() {
        let (_store, _f) = make_store();
        // 能打开且无 panic 即表结构创建成功
    }

    // ── 2. save + list_all 基本 roundtrip ────────────────────────────────────
    #[test]
    fn test_save_and_list_all() {
        let (store, _f) = make_store();

        store.save(&cron_record("t1")).unwrap();
        store.save(&cron_record("t2")).unwrap();

        let all = store.list_all().unwrap();
        assert_eq!(all.len(), 2);

        let ids: Vec<_> = all.iter().map(|r| r.trigger_id.as_str()).collect();
        assert!(ids.contains(&"t1"));
        assert!(ids.contains(&"t2"));
    }

    // ── 3. save 是幂等的 upsert ───────────────────────────────────────────────
    #[test]
    fn test_save_upsert() {
        let (store, _f) = make_store();

        let mut rec = cron_record("t1");
        store.save(&rec).unwrap();

        rec.fire_count = 5;
        store.save(&rec).unwrap();

        let all = store.list_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].fire_count, 5);
    }

    // ── 4. delete 删除记录 ────────────────────────────────────────────────────
    #[test]
    fn test_delete() {
        let (store, _f) = make_store();

        store.save(&cron_record("t1")).unwrap();
        store.save(&cron_record("t2")).unwrap();

        store.delete("t1").unwrap();

        let all = store.list_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].trigger_id, "t2");
    }

    // ── 5. delete 不存在的 id 是幂等的 ───────────────────────────────────────
    #[test]
    fn test_delete_nonexistent_is_noop() {
        let (store, _f) = make_store();
        // 不 panic，不报错
        store.delete("ghost").unwrap();
    }

    // ── 6. update_last_fired 更新时间戳和计数 ────────────────────────────────
    #[test]
    fn test_update_last_fired() {
        let (store, _f) = make_store();

        store.save(&cron_record("t1")).unwrap();
        store.update_last_fired("t1", 1_700_001_000).unwrap();

        let all = store.list_all().unwrap();
        assert_eq!(all[0].last_fired_at, Some(1_700_001_000));
        assert_eq!(all[0].fire_count, 1);

        // 再次触发
        store.update_last_fired("t1", 1_700_002_000).unwrap();
        let all = store.list_all().unwrap();
        assert_eq!(all[0].fire_count, 2);
    }

    // ── 7. set_enabled 软禁用后 list_all 不再返回该记录 ─────────────────────
    #[test]
    fn test_set_enabled_disables_record() {
        let (store, _f) = make_store();

        store.save(&cron_record("t1")).unwrap();
        store.save(&cron_record("t2")).unwrap();

        store.set_enabled("t1", false).unwrap();

        let all = store.list_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].trigger_id, "t2");
    }

    // ── 8. target_agent 字段正确持久化 ───────────────────────────────────────
    #[test]
    fn test_target_agent_persistence() {
        let (store, _f) = make_store();

        let mut rec = cron_record("t1");
        rec.target_agent = Some("agent-42".to_string());
        store.save(&rec).unwrap();

        let all = store.list_all().unwrap();
        assert_eq!(all[0].target_agent, Some("agent-42".to_string()));
    }

    // ── 9. config JSON 正确往返 ───────────────────────────────────────────────
    #[test]
    fn test_config_json_roundtrip() {
        let (store, _f) = make_store();

        let mut rec = cron_record("t1");
        rec.config = serde_json::json!({ "expr": "*/5 * * * *", "tz": "UTC" });
        store.save(&rec).unwrap();

        let all = store.list_all().unwrap();
        assert_eq!(all[0].config["expr"], "*/5 * * * *");
        assert_eq!(all[0].config["tz"], "UTC");
    }

    // ── 10. 空库 list_all 返回空 Vec ──────────────────────────────────────────
    #[test]
    fn test_list_all_empty() {
        let (store, _f) = make_store();
        let all = store.list_all().unwrap();
        assert!(all.is_empty());
    }
}
