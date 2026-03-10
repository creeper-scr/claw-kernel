//! CronScheduler — EventBus 驱动的 Cron 调度器（GAP-F6-01 / GAP-F6-02）。
//!
//! `CronScheduler` 是 F6"定时触发"能力的核心实现。每个注册的 [`CronJob`]
//! 使用 6 字段秒级 Cron 表达式（`s m h d M dow`），到期后向 [`EventBus`]
//! 发布 [`Event::TriggerFired`]，由 [`TriggerDispatcher`] 路由到目标 Agent。
//!
//! # 持久化（GAP-F6-02）
//!
//! 当构造时提供 `TriggerStore` 时，所有写操作均自动持久化：
//!
//! | 操作             | 持久化行为                          |
//! |----------------|--------------------------------------|
//! | `add()`        | `store.save(record)`                 |
//! | `remove()`     | `store.delete(trigger_id)`           |
//! | `run()` 启动   | `store.list_all()` → 恢复所有 job    |
//! | 每次触发       | `store.update_last_fired()`         |
//!
//! # 架构
//!
//! ```text
//! CronScheduler::add()
//!      │
//!      ├─► DashMap<trigger_id, CronJob>
//!      └─► TriggerStore::save()  (可选，启用时自动调用)
//!
//! CronScheduler::run()
//!      │  (启动时)
//!      ├─► TriggerStore::list_all() → 恢复所有持久化 job
//!      │  (每秒)
//!      ├─► 遍历 jobs，到期则：
//!      │      event_bus.publish(Event::TriggerFired(TriggerEvent::cron(...)))
//!      │      TriggerStore::update_last_fired()
//!      │          │
//!      │          ▼
//!      │    TriggerDispatcher  (已订阅 EventBus)
//!      └─► (表达式耗尽时) TriggerStore::delete()
//! ```
//!
//! # 示例（有持久化）
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use std::path::Path;
//! use claw_runtime::{EventBus, CronScheduler};
//! use claw_runtime::trigger_store::TriggerStore;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let bus = Arc::new(EventBus::new());
//! let store = Arc::new(TriggerStore::open(Path::new("/tmp/triggers.db"))?);
//! let scheduler = CronScheduler::with_store(Arc::clone(&bus), Arc::clone(&store));
//!
//! // 注册会同步写入 SQLite
//! scheduler.add("*/5 * * * * *", "report-trigger", None)?;
//!
//! // run() 启动时自动从 SQLite 恢复所有 job
//! tokio::spawn(scheduler.run());
//! # Ok(())
//! # }
//! ```

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use cron::Schedule;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::agent_types::AgentId;
use crate::event_bus::EventBus;
use crate::events::Event;
use crate::trigger_event::TriggerEvent;
use crate::trigger_store::{TriggerRecord, TriggerStore};

// ─── Error ─────────────────────────────────────────────────────────────────────

/// CronScheduler 错误类型。
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum CronError {
    /// Cron 表达式语法错误。
    #[error("invalid cron expression '{expr}': {reason}")]
    InvalidExpression {
        /// 非法表达式。
        expr: String,
        /// 来自解析器的原因。
        reason: String,
    },

    /// Cron 表达式语法正确但永远不会触发（如 `0 0 30 2 *`）。
    #[error("cron expression will never fire: {0}")]
    NeverFires(String),

    /// 相同 trigger_id 已存在。
    #[error("cron job already exists: {0}")]
    AlreadyExists(String),

    /// 指定的 trigger_id 不存在。
    #[error("cron job not found: {0}")]
    NotFound(String),
}

// ─── CronJob ───────────────────────────────────────────────────────────────────

/// 单个 Cron 定时任务的运行时状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    /// 对应 TriggerStore 中触发源的唯一 ID。
    pub trigger_id: String,
    /// 6 字段秒级 Cron 表达式（`s m h d M dow`）。
    pub expr: String,
    /// 指定目标 Agent；`None` 表示广播到所有在线 Agent。
    pub target_agent: Option<AgentId>,
    /// 下次预计触发时刻（UTC）。
    pub next_fire: DateTime<Utc>,
    /// 任务注册时刻（UTC）。
    pub created_at: DateTime<Utc>,
    /// 是否启用；`false` 时调度循环跳过此任务。
    pub enabled: bool,
}

// ─── CronScheduler ─────────────────────────────────────────────────────────────

/// EventBus 驱动的 Cron 调度器，支持可选的 SQLite 持久化（GAP-F6-02）。
///
/// - 无持久化：使用 [`CronScheduler::new`]，数据驻留内存，重启后丢失。
/// - 有持久化：使用 [`CronScheduler::with_store`]，数据写入 `TriggerStore`，
///   `run()` 启动时自动恢复，触发时更新触发记录。
pub struct CronScheduler {
    /// 注册的任务集合（trigger_id → CronJob）。
    jobs: Arc<DashMap<String, CronJob>>,
    /// 用于发布 `Event::TriggerFired` 的事件总线。
    event_bus: Arc<EventBus>,
    /// 可选的持久化仓库（GAP-F6-02）。
    store: Option<Arc<TriggerStore>>,
}

impl CronScheduler {
    /// 创建一个无持久化的 `CronScheduler`（纯内存模式）。
    ///
    /// 数据仅驻留内存，进程重启后全部丢失。
    /// 生产环境建议使用 [`with_store`](CronScheduler::with_store)。
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            jobs: Arc::new(DashMap::new()),
            event_bus,
            store: None,
        }
    }

    /// 创建一个带 SQLite 持久化的 `CronScheduler`（GAP-F6-02）。
    ///
    /// `run()` 启动时会自动调用 `store.list_all()` 恢复所有已持久化的 job。
    pub fn with_store(event_bus: Arc<EventBus>, store: Arc<TriggerStore>) -> Self {
        Self {
            jobs: Arc::new(DashMap::new()),
            event_bus,
            store: Some(store),
        }
    }

    /// 注册一个新的 Cron 定时任务。
    ///
    /// 若 `TriggerStore` 存在，会同步写入持久化层。
    ///
    /// # 参数
    ///
    /// - `expr`       — 6 字段秒级 Cron 表达式，例如 `"0 */5 * * * *"`（每 5 分钟）。
    /// - `trigger_id` — 触发源 ID，必须全局唯一；对应 TriggerStore 中的记录。
    /// - `target`     — 目标 Agent；`None` 表示广播。
    ///
    /// # 错误
    ///
    /// - [`CronError::InvalidExpression`] — 表达式语法错误。
    /// - [`CronError::NeverFires`]        — 表达式永远不会触发。
    /// - [`CronError::AlreadyExists`]     — 相同 `trigger_id` 已注册。
    pub fn add(
        &self,
        expr: &str,
        trigger_id: &str,
        target: Option<AgentId>,
    ) -> Result<(), CronError> {
        // 1. 验证并解析表达式（立即返回错误，无延迟）
        let schedule = Schedule::from_str(expr).map_err(|e| CronError::InvalidExpression {
            expr: expr.to_string(),
            reason: e.to_string(),
        })?;

        // 2. 确保至少有一个未来触发点
        let next_fire = schedule
            .upcoming(Utc)
            .next()
            .ok_or_else(|| CronError::NeverFires(expr.to_string()))?;

        // 3. 检查重复注册
        if self.jobs.contains_key(trigger_id) {
            return Err(CronError::AlreadyExists(trigger_id.to_string()));
        }

        let now_ts = Utc::now().timestamp();
        let created_at = Utc::now();

        // 4. 写入 DashMap
        self.jobs.insert(
            trigger_id.to_string(),
            CronJob {
                trigger_id: trigger_id.to_string(),
                expr: expr.to_string(),
                target_agent: target.clone(),
                next_fire,
                created_at,
                enabled: true,
            },
        );

        // 5. 持久化（GAP-F6-02）
        if let Some(store) = &self.store {
            let record = TriggerRecord {
                trigger_id: trigger_id.to_string(),
                trigger_type: "cron".to_string(),
                config: serde_json::json!({ "expr": expr }),
                target_agent: target.map(|a| a.0),
                created_at: now_ts,
                enabled: true,
                last_fired_at: None,
                fire_count: 0,
            };
            if let Err(e) = store.save(&record) {
                tracing::warn!(
                    trigger_id = %trigger_id,
                    error = %e,
                    "CronScheduler: failed to persist job to store"
                );
            }
        }

        tracing::debug!(
            trigger_id = %trigger_id,
            expr = %expr,
            next_fire = %next_fire,
            "CronScheduler: job registered"
        );

        Ok(())
    }

    /// 移除一个已注册的 Cron 定时任务。
    ///
    /// 若 `TriggerStore` 存在，会同步从持久化层删除。
    ///
    /// 返回 `true` 表示成功移除，`false` 表示 `trigger_id` 不存在。
    pub fn remove(&self, trigger_id: &str) -> bool {
        let removed = self.jobs.remove(trigger_id).is_some();
        if removed {
            // 持久化删除（GAP-F6-02）
            if let Some(store) = &self.store {
                if let Err(e) = store.delete(trigger_id) {
                    tracing::warn!(
                        trigger_id = %trigger_id,
                        error = %e,
                        "CronScheduler: failed to delete job from store"
                    );
                }
            }
            tracing::debug!(trigger_id = %trigger_id, "CronScheduler: job removed");
        }
        removed
    }

    /// 列出所有已注册的 Cron 任务（快照）。
    ///
    /// 返回按 `trigger_id` 字母顺序排序的列表。
    pub fn list(&self) -> Vec<CronJob> {
        let mut jobs: Vec<CronJob> = self.jobs.iter().map(|e| e.value().clone()).collect();
        jobs.sort_by(|a, b| a.trigger_id.cmp(&b.trigger_id));
        jobs
    }

    /// 返回当前注册的任务数量。
    pub fn job_count(&self) -> usize {
        self.jobs.len()
    }

    /// 从 `TriggerStore` 恢复所有已持久化的 Cron job（GAP-F6-02）。
    ///
    /// 仅在有 store 时调用。重复的 trigger_id 会跳过（防止恢复与已注册 job 冲突）。
    fn restore_from_store(&self) {
        let store = match &self.store {
            Some(s) => s,
            None => return,
        };

        let records = match store.list_all() {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "CronScheduler: failed to list records from store");
                return;
            }
        };

        let mut restored = 0usize;

        for record in records {
            // 只恢复 cron 类型
            if record.trigger_type != "cron" {
                continue;
            }

            // 跳过已在内存中的 job
            if self.jobs.contains_key(&record.trigger_id) {
                continue;
            }

            // 从 config JSON 中取回 expr
            let expr = match record.config.get("expr").and_then(|v| v.as_str()) {
                Some(e) => e.to_string(),
                None => {
                    tracing::warn!(
                        trigger_id = %record.trigger_id,
                        "CronScheduler: restore skipped — missing 'expr' in config"
                    );
                    continue;
                }
            };

            // 解析表达式，验证可用性
            let schedule = match Schedule::from_str(&expr) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(
                        trigger_id = %record.trigger_id,
                        expr = %expr,
                        error = %e,
                        "CronScheduler: restore skipped — invalid cron expr"
                    );
                    continue;
                }
            };

            let next_fire = match schedule.upcoming(Utc).next() {
                Some(t) => t,
                None => {
                    tracing::warn!(
                        trigger_id = %record.trigger_id,
                        "CronScheduler: restore skipped — cron expr will never fire"
                    );
                    continue;
                }
            };

            let target_agent = record.target_agent.map(|s| AgentId::new(s));
            let created_at = DateTime::from_timestamp(record.created_at, 0)
                .unwrap_or_else(Utc::now);

            self.jobs.insert(
                record.trigger_id.clone(),
                CronJob {
                    trigger_id: record.trigger_id.clone(),
                    expr,
                    target_agent,
                    next_fire,
                    created_at,
                    enabled: record.enabled,
                },
            );

            restored += 1;
        }

        if restored > 0 {
            tracing::info!(count = restored, "CronScheduler: restored jobs from TriggerStore");
        }
    }

    /// 启动调度循环（阻塞，直到外部中止）。
    ///
    /// 若 `TriggerStore` 存在，启动前先调用 `restore_from_store()` 恢复持久化 job。
    ///
    /// 每秒唤醒一次，遍历所有 enabled 任务：
    /// - 若当前时刻 ≥ `next_fire`，则向 EventBus 发布 `Event::TriggerFired`
    ///   并更新 `next_fire`；同时调用 `store.update_last_fired()`。
    /// - 若表达式耗尽（极罕见），则自动移除任务并从 store 删除。
    ///
    /// 建议在 `tokio::spawn` 中调用：
    ///
    /// ```rust,ignore
    /// tokio::spawn(scheduler.run());
    /// ```
    pub async fn run(self) {
        // GAP-F6-02：启动时从 TriggerStore 恢复所有持久化 job
        self.restore_from_store();

        let mut interval = tokio::time::interval(Duration::from_secs(1));

        loop {
            interval.tick().await;

            let now = Utc::now();

            // 收集需要触发或移除的 trigger_id（避免在 DashMap iter 中持有写锁）
            let mut to_fire: Vec<(String, Option<AgentId>)> = Vec::new();
            let mut exhausted: Vec<String> = Vec::new();

            for entry in self.jobs.iter() {
                let job = entry.value();
                if !job.enabled {
                    continue;
                }
                if now >= job.next_fire {
                    to_fire.push((job.trigger_id.clone(), job.target_agent.clone()));
                }
            }

            // 触发并更新 next_fire
            for (trigger_id, target_agent) in to_fire {
                // 发布 TriggerFired 事件
                let ev = TriggerEvent::cron(&trigger_id, target_agent);
                self.event_bus.publish(Event::TriggerFired(ev));

                tracing::debug!(trigger_id = %trigger_id, "CronScheduler: fired");

                // GAP-F6-02：更新持久化层的触发记录
                if let Some(store) = &self.store {
                    let fired_ts = now.timestamp();
                    if let Err(e) = store.update_last_fired(&trigger_id, fired_ts) {
                        tracing::warn!(
                            trigger_id = %trigger_id,
                            error = %e,
                            "CronScheduler: failed to update last_fired in store"
                        );
                    }
                }

                // 更新 next_fire（获取写锁）
                if let Some(mut job_entry) = self.jobs.get_mut(&trigger_id) {
                    let schedule = match Schedule::from_str(&job_entry.expr) {
                        Ok(s) => s,
                        Err(_) => {
                            // 表达式解析失败（不应发生，已在 add() 验证）
                            tracing::error!(
                                trigger_id = %trigger_id,
                                expr = %job_entry.expr,
                                "CronScheduler: failed to re-parse cron expr; removing job"
                            );
                            exhausted.push(trigger_id.clone());
                            continue;
                        }
                    };

                    match schedule.upcoming(Utc).next() {
                        Some(next) => {
                            job_entry.next_fire = next;
                        }
                        None => {
                            // 表达式已耗尽
                            exhausted.push(trigger_id.clone());
                        }
                    }
                }
            }

            // 移除已耗尽的任务（同步从 store 删除）
            for trigger_id in exhausted {
                self.jobs.remove(&trigger_id);
                if let Some(store) = &self.store {
                    if let Err(e) = store.delete(&trigger_id) {
                        tracing::warn!(
                            trigger_id = %trigger_id,
                            error = %e,
                            "CronScheduler: failed to delete exhausted job from store"
                        );
                    }
                }
                tracing::info!(
                    trigger_id = %trigger_id,
                    "CronScheduler: job exhausted, removed"
                );
            }
        }
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{event_bus::EventBus, events::Event};
    use std::sync::Arc;
    use tempfile::NamedTempFile;

    fn make_scheduler() -> CronScheduler {
        let bus = Arc::new(EventBus::new());
        CronScheduler::new(bus)
    }

    fn make_scheduler_with_store() -> (CronScheduler, Arc<TriggerStore>, NamedTempFile) {
        let bus = Arc::new(EventBus::new());
        let f = NamedTempFile::new().unwrap();
        let store = Arc::new(TriggerStore::open(f.path()).unwrap());
        let scheduler = CronScheduler::with_store(bus, Arc::clone(&store));
        (scheduler, store, f)
    }

    // ── 1. add / remove / list ─────────────────────────────────────────────────

    #[test]
    fn test_add_valid_job() {
        let s = make_scheduler();
        // 每分钟整秒触发（秒级 6-field：0 * * * * *）
        s.add("0 * * * * *", "job-1", None).expect("should succeed");
        assert_eq!(s.job_count(), 1);
        let jobs = s.list();
        assert_eq!(jobs[0].trigger_id, "job-1");
        assert!(jobs[0].enabled);
    }

    #[test]
    fn test_add_duplicate_fails() {
        let s = make_scheduler();
        s.add("0 * * * * *", "dup", None).unwrap();
        let err = s.add("0 * * * * *", "dup", None).unwrap_err();
        assert!(matches!(err, CronError::AlreadyExists(_)));
    }

    #[test]
    fn test_add_invalid_expr_fails() {
        let s = make_scheduler();
        let err = s.add("not-a-cron-expr", "bad", None).unwrap_err();
        assert!(matches!(err, CronError::InvalidExpression { .. }));
    }

    #[test]
    fn test_remove_existing_job() {
        let s = make_scheduler();
        s.add("0 * * * * *", "to-remove", None).unwrap();
        assert_eq!(s.job_count(), 1);
        assert!(s.remove("to-remove"));
        assert_eq!(s.job_count(), 0);
    }

    #[test]
    fn test_remove_nonexistent_returns_false() {
        let s = make_scheduler();
        assert!(!s.remove("ghost"));
    }

    #[test]
    fn test_list_sorted_by_trigger_id() {
        let s = make_scheduler();
        s.add("0 * * * * *", "bravo", None).unwrap();
        s.add("0 * * * * *", "alpha", None).unwrap();
        s.add("0 * * * * *", "charlie", None).unwrap();
        let ids: Vec<_> = s.list().iter().map(|j| j.trigger_id.clone()).collect();
        assert_eq!(ids, vec!["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn test_job_has_correct_expr_and_target() {
        let s = make_scheduler();
        let agent = AgentId::new("my-agent");
        s.add("0 */5 * * * *", "five-min", Some(agent.clone()))
            .unwrap();
        let job = s.list().into_iter().next().unwrap();
        assert_eq!(job.expr, "0 */5 * * * *");
        assert_eq!(job.target_agent, Some(agent));
    }

    // ── 2. TriggerStore 集成（GAP-F6-02） ──────────────────────────────────────

    #[test]
    fn test_add_persists_to_store() {
        let (s, store, _f) = make_scheduler_with_store();
        s.add("0 * * * * *", "persist-job", None).unwrap();

        let records = store.list_all().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].trigger_id, "persist-job");
        assert_eq!(records[0].trigger_type, "cron");
        assert_eq!(records[0].config["expr"], "0 * * * * *");
    }

    #[test]
    fn test_remove_deletes_from_store() {
        let (s, store, _f) = make_scheduler_with_store();
        s.add("0 * * * * *", "to-delete", None).unwrap();
        assert_eq!(store.list_all().unwrap().len(), 1);

        s.remove("to-delete");
        assert_eq!(store.list_all().unwrap().len(), 0);
    }

    #[test]
    fn test_restore_from_store() {
        let f = NamedTempFile::new().unwrap();
        let store = Arc::new(TriggerStore::open(f.path()).unwrap());
        let bus = Arc::new(EventBus::new());

        // 第一个调度器：写入持久化数据
        {
            let s1 = CronScheduler::with_store(Arc::clone(&bus), Arc::clone(&store));
            s1.add("0 * * * * *", "restored-job", None).unwrap();
            assert_eq!(s1.job_count(), 1);
        }

        // 第二个调度器（模拟重启）：从 store 恢复
        let s2 = CronScheduler::with_store(Arc::clone(&bus), Arc::clone(&store));
        assert_eq!(s2.job_count(), 0, "重启前内存中没有任务");
        s2.restore_from_store();
        assert_eq!(s2.job_count(), 1, "恢复后应有 1 个任务");
        assert_eq!(s2.list()[0].trigger_id, "restored-job");
    }

    #[test]
    fn test_restore_skips_non_cron_records() {
        let f = NamedTempFile::new().unwrap();
        let store = Arc::new(TriggerStore::open(f.path()).unwrap());

        // 直接写入一条 webhook 类型的记录
        store.save(&crate::trigger_store::TriggerRecord {
            trigger_id: "webhook-trigger".to_string(),
            trigger_type: "webhook".to_string(),
            config: serde_json::json!({ "path": "/hooks/gh" }),
            target_agent: None,
            created_at: 0,
            enabled: true,
            last_fired_at: None,
            fire_count: 0,
        }).unwrap();

        let bus = Arc::new(EventBus::new());
        let s = CronScheduler::with_store(bus, Arc::clone(&store));
        s.restore_from_store();

        // webhook 记录不应被恢复为 cron job
        assert_eq!(s.job_count(), 0);
    }

    #[test]
    fn test_restore_skips_already_registered_jobs() {
        let f = NamedTempFile::new().unwrap();
        let store = Arc::new(TriggerStore::open(f.path()).unwrap());
        let bus = Arc::new(EventBus::new());

        // 在 store 中写一条记录
        store.save(&crate::trigger_store::TriggerRecord {
            trigger_id: "existing-job".to_string(),
            trigger_type: "cron".to_string(),
            config: serde_json::json!({ "expr": "0 * * * * *" }),
            target_agent: None,
            created_at: 0,
            enabled: true,
            last_fired_at: None,
            fire_count: 0,
        }).unwrap();

        let s = CronScheduler::with_store(bus, Arc::clone(&store));
        // 先手动注册同名 job
        s.add("0 * * * * *", "existing-job", None).unwrap();
        assert_eq!(s.job_count(), 1);

        // 恢复时应跳过已注册的 job，不导致重复
        s.restore_from_store();
        assert_eq!(s.job_count(), 1, "不应产生重复 job");
    }

    #[test]
    fn test_add_with_target_agent_persists_correctly() {
        let (s, store, _f) = make_scheduler_with_store();
        let agent = AgentId::new("agent-99");
        s.add("0 * * * * *", "targeted-job", Some(agent)).unwrap();

        let records = store.list_all().unwrap();
        assert_eq!(records[0].target_agent, Some("agent-99".to_string()));
    }

    // ── 3. run() 调度循环 ───────────────────────────────────────────────────────

    /// 注册一个每秒触发的 job，运行调度循环并验证 EventBus 收到 TriggerFired 事件。
    #[tokio::test]
    async fn test_run_fires_trigger_event_on_event_bus() {
        let bus = Arc::new(EventBus::new());
        let scheduler = CronScheduler::new(Arc::clone(&bus));
        let mut rx = bus.subscribe();

        // "* * * * * *" = 每秒触发
        scheduler.add("* * * * * *", "every-second", None).unwrap();

        let handle = tokio::spawn(scheduler.run());

        // 等待最多 3 秒，期间应能收到 TriggerFired 事件
        let deadline = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match rx.recv().await {
                    Ok(Event::TriggerFired(ev)) => {
                        assert_eq!(ev.trigger_id, "every-second");
                        assert_eq!(ev.trigger_type, crate::trigger_event::TriggerType::Cron);
                        assert!(ev.target_agent.is_none());
                        break;
                    }
                    Ok(_) => continue,
                    Err(e) => panic!("EventBus recv error: {e}"),
                }
            }
        });

        deadline.await.expect("should receive TriggerFired within 3 seconds");

        handle.abort();
        let r = handle.await;
        assert!(r.unwrap_err().is_cancelled());
    }

    /// 注册后移除 job，调度循环运行时不应触发任何事件。
    #[tokio::test]
    async fn test_run_does_not_fire_removed_job() {
        let bus = Arc::new(EventBus::new());
        let scheduler = CronScheduler::new(Arc::clone(&bus));
        let mut rx = bus.subscribe();

        scheduler.add("* * * * * *", "removable", None).unwrap();
        // 立即移除
        scheduler.remove("removable");

        let handle = tokio::spawn(scheduler.run());

        // 等待 2 秒，不应收到任何 TriggerFired
        let result = tokio::time::timeout(Duration::from_millis(2000), async {
            loop {
                match rx.recv().await {
                    Ok(Event::TriggerFired(_)) => return true, // 不应到这里
                    Ok(_) => continue,
                    Err(_) => return false,
                }
            }
        })
        .await;

        // timeout = 没收到 TriggerFired，符合预期
        assert!(result.is_err(), "should not have received TriggerFired after remove");

        handle.abort();
    }

    /// 多个 job 同时运行时，事件 trigger_id 与注册的 id 匹配。
    #[tokio::test]
    async fn test_run_multiple_jobs_fire_correctly() {
        let bus = Arc::new(EventBus::new());
        let scheduler = CronScheduler::new(Arc::clone(&bus));
        let mut rx = bus.subscribe();

        scheduler.add("* * * * * *", "job-a", None).unwrap();
        scheduler.add("* * * * * *", "job-b", None).unwrap();

        let handle = tokio::spawn(scheduler.run());

        let mut seen = std::collections::HashSet::new();
        let deadline = tokio::time::timeout(Duration::from_secs(3), async {
            while seen.len() < 2 {
                match rx.recv().await {
                    Ok(Event::TriggerFired(ev)) => {
                        seen.insert(ev.trigger_id.clone());
                    }
                    Ok(_) => continue,
                    Err(e) => panic!("recv error: {e}"),
                }
            }
        });

        deadline.await.expect("both jobs should fire within 3 seconds");
        assert!(seen.contains("job-a"));
        assert!(seen.contains("job-b"));

        handle.abort();
    }

    /// target_agent 正确传递到 TriggerEvent。
    #[tokio::test]
    async fn test_run_sets_target_agent_in_event() {
        let bus = Arc::new(EventBus::new());
        let scheduler = CronScheduler::new(Arc::clone(&bus));
        let mut rx = bus.subscribe();

        let agent = AgentId::new("target-007");
        scheduler
            .add("* * * * * *", "targeted", Some(agent.clone()))
            .unwrap();

        let handle = tokio::spawn(scheduler.run());

        let deadline = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match rx.recv().await {
                    Ok(Event::TriggerFired(ev)) if ev.trigger_id == "targeted" => {
                        assert_eq!(ev.target_agent, Some(agent.clone()));
                        break;
                    }
                    Ok(_) => continue,
                    Err(e) => panic!("recv error: {e}"),
                }
            }
        });

        deadline.await.expect("targeted job should fire within 3 seconds");

        handle.abort();
    }

    /// run() 触发后，TriggerStore 中的 last_fired_at 和 fire_count 被更新。
    #[tokio::test]
    async fn test_run_updates_last_fired_in_store() {
        let f = NamedTempFile::new().unwrap();
        let store = Arc::new(TriggerStore::open(f.path()).unwrap());
        let bus = Arc::new(EventBus::new());

        let scheduler = CronScheduler::with_store(Arc::clone(&bus), Arc::clone(&store));
        scheduler.add("* * * * * *", "store-update-test", None).unwrap();

        let mut rx = bus.subscribe();
        let handle = tokio::spawn(scheduler.run());

        // 等待至少一次触发
        let deadline = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match rx.recv().await {
                    Ok(Event::TriggerFired(ev)) if ev.trigger_id == "store-update-test" => break,
                    Ok(_) => continue,
                    Err(e) => panic!("recv error: {e}"),
                }
            }
        });
        deadline.await.expect("should fire within 3 seconds");

        // 等一点时间让 store.update_last_fired 写完
        tokio::time::sleep(Duration::from_millis(100)).await;
        handle.abort();
        let _ = handle.await;

        let records = store.list_all().unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0].last_fired_at.is_some(), "last_fired_at 应已更新");
        assert!(records[0].fire_count >= 1, "fire_count 应 >= 1");
    }
}
