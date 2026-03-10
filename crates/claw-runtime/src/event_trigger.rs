//! EventTriggerRegistry — EventBus 条件转发（GAP-F6-06）。
//!
//! `EventTriggerRegistry` 订阅 [`EventBus`] 上的任意 [`Event`]，当事件的
//! `type_tag` 匹配规则中的 `watch_event` 字符串，且可选的 payload 过滤器
//! 全部通过时，自动发布 [`Event::TriggerFired`] 触发目标 Agent。
//!
//! # 设计边界
//!
//! - **内核层**：提供 `watch_event` 精确字符串匹配 + JSON key/value 等值过滤。
//! - **应用层**：正则、JMESPath 等复杂条件由应用自行在 EventBus 订阅后实现。
//!
//! # 支持的 watch_event 字符串
//!
//! | `watch_event`                     | 匹配的 `Event` 变体                      |
//! |----------------------------------|------------------------------------------|
//! | `"agent.started"`                | `Event::AgentStarted { .. }`             |
//! | `"agent.stopped"`                | `Event::AgentStopped { .. }`             |
//! | `"agent.restarted"`              | `Event::AgentRestarted { .. }`           |
//! | `"agent.failed"`                 | `Event::AgentFailed { .. }`              |
//! | `"llm.request.started"`          | `Event::LlmRequestStarted { .. }`        |
//! | `"llm.request.completed"`        | `Event::LlmRequestCompleted { .. }`      |
//! | `"message.received"`             | `Event::MessageReceived { .. }`          |
//! | `"tool.called"`                  | `Event::ToolCalled { .. }`               |
//! | `"tool.result"`                  | `Event::ToolResult { .. }`               |
//! | `"context.window.limit"`         | `Event::ContextWindowApproachingLimit { .. }` |
//! | `"memory.archive.complete"`      | `Event::MemoryArchiveComplete { .. }`    |
//! | `"mode.changed"`                 | `Event::ModeChanged { .. }`              |
//! | `"trigger.fired"`                | `Event::TriggerFired(..)`                |
//! | `"custom.<event_type>"`          | `Event::Custom { event_type, .. }` where `event_type == <event_type>` |
//!
//! # 示例
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use claw_runtime::{EventBus, event_trigger::{EventTriggerRegistry, EventTriggerRule}};
//!
//! # #[tokio::main]
//! # async fn main() {
//! let bus = Arc::new(EventBus::new());
//! let registry = EventTriggerRegistry::new(Arc::clone(&bus));
//!
//! // 当任意 Agent 停止时触发 "cleanup-agent"
//! registry.register(EventTriggerRule {
//!     trigger_id: "on-agent-stopped".into(),
//!     watch_event: "agent.stopped".into(),
//!     payload_filter: None,
//!     target_agent: None,
//! });
//!
//! tokio::spawn(registry.run());
//! # }
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::agent_types::AgentId;
use crate::event_bus::EventBus;
use crate::events::Event;
use crate::trigger_event::TriggerEvent;

// ─── EventTriggerRule ──────────────────────────────────────────────────────────

/// 单条 EventTrigger 规则。
///
/// 注册到 [`EventTriggerRegistry`] 后，每次 EventBus 上的事件满足条件时
/// 自动发布 `Event::TriggerFired`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventTriggerRule {
    /// 规则唯一 ID，同时作为生成的 `TriggerEvent::trigger_id`。
    pub trigger_id: String,

    /// 监听的事件类型标签（见模块文档中的映射表）。
    ///
    /// 对于 `custom.*` 事件使用 `"custom.<event_type>"` 格式，例如 `"custom.user.login"`。
    pub watch_event: String,

    /// 可选 JSON 等值过滤器。
    ///
    /// `Some(obj)` 时，只有当事件序列化后的 JSON payload 包含 `obj` 中所有
    /// key/value 对时才触发。对比使用 [`serde_json::Value`] 深度等值。
    ///
    /// `None` 表示无过滤，匹配所有该类型事件。
    pub payload_filter: Option<serde_json::Value>,

    /// 触发后路由的目标 Agent；`None` 表示广播到所有在线 Agent。
    pub target_agent: Option<AgentId>,
}

impl EventTriggerRule {
    /// 创建一个无过滤的简单规则（匹配所有该类型事件并广播）。
    pub fn new(trigger_id: impl Into<String>, watch_event: impl Into<String>) -> Self {
        Self {
            trigger_id: trigger_id.into(),
            watch_event: watch_event.into(),
            payload_filter: None,
            target_agent: None,
        }
    }

    /// 设置 payload 过滤器（builder 风格）。
    pub fn with_filter(mut self, filter: serde_json::Value) -> Self {
        self.payload_filter = Some(filter);
        self
    }

    /// 设置目标 Agent（builder 风格）。
    pub fn with_target(mut self, agent_id: AgentId) -> Self {
        self.target_agent = Some(agent_id);
        self
    }
}

// ─── EventTriggerRegistry ─────────────────────────────────────────────────────

/// EventTrigger 规则注册表。
///
/// 订阅 [`EventBus`]，当事件匹配注册的规则时发布 `Event::TriggerFired`。
/// 通过 [`EventTriggerRegistry::run`] 在独立 tokio 任务中驱动。
pub struct EventTriggerRegistry {
    rules: Arc<DashMap<String, EventTriggerRule>>,
    event_bus: Arc<EventBus>,
}

impl EventTriggerRegistry {
    /// 创建一个新的 `EventTriggerRegistry`。
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            rules: Arc::new(DashMap::new()),
            event_bus,
        }
    }

    /// 注册一条 EventTrigger 规则。
    ///
    /// 若相同 `trigger_id` 已存在则覆盖。
    pub fn register(&self, rule: EventTriggerRule) {
        tracing::debug!(
            trigger_id = %rule.trigger_id,
            watch_event = %rule.watch_event,
            has_filter = rule.payload_filter.is_some(),
            "EventTriggerRegistry: registered rule"
        );
        self.rules.insert(rule.trigger_id.clone(), rule);
    }

    /// 注销一条规则。返回 `true` 表示规则存在并被移除。
    pub fn unregister(&self, trigger_id: &str) -> bool {
        let removed = self.rules.remove(trigger_id).is_some();
        if removed {
            tracing::debug!(trigger_id = %trigger_id, "EventTriggerRegistry: unregistered rule");
        }
        removed
    }

    /// 列出所有已注册的规则（快照）。
    pub fn list(&self) -> Vec<EventTriggerRule> {
        self.rules.iter().map(|r| r.value().clone()).collect()
    }

    /// 启动事件监听循环（阻塞，直到 EventBus 关闭）。
    ///
    /// 建议在 `tokio::spawn` 中调用：
    ///
    /// ```rust,ignore
    /// tokio::spawn(registry.run());
    /// ```
    pub async fn run(self) {
        let mut rx = self.event_bus.subscribe();

        loop {
            match rx.recv().await {
                Ok(event) => {
                    self.process_event(&event);
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "EventTriggerRegistry: EventBus error, stopping"
                    );
                    break;
                }
            }
        }
    }

    /// 对一个事件检查所有规则，匹配则发布 TriggerFired。
    fn process_event(&self, event: &Event) {
        let type_tag = event_type_tag(event);
        let event_payload = event_to_payload(event);

        for rule_ref in self.rules.iter() {
            let rule = rule_ref.value();

            // 1. type_tag 匹配
            if !type_tag_matches(&rule.watch_event, type_tag, event) {
                continue;
            }

            // 2. payload 过滤
            if let Some(filter) = &rule.payload_filter {
                if !payload_matches(filter, &event_payload) {
                    continue;
                }
            }

            // 3. 发布 TriggerFired
            let trigger_ev = TriggerEvent::event(
                rule.trigger_id.clone(),
                event_payload.clone(),
                rule.target_agent.clone(),
            );

            tracing::debug!(
                trigger_id = %rule.trigger_id,
                watch_event = %rule.watch_event,
                "EventTriggerRegistry: rule matched, firing trigger"
            );

            self.event_bus.publish(Event::TriggerFired(trigger_ev));
        }
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// 将 Event 映射为 type_tag 字符串。
fn event_type_tag(event: &Event) -> &'static str {
    match event {
        Event::AgentStarted { .. } => "agent.started",
        Event::AgentStopped { .. } => "agent.stopped",
        Event::AgentRestarted { .. } => "agent.restarted",
        Event::AgentFailed { .. } => "agent.failed",
        Event::LlmRequestStarted { .. } => "llm.request.started",
        Event::LlmRequestCompleted { .. } => "llm.request.completed",
        Event::MessageReceived { .. } => "message.received",
        Event::A2A(_) => "a2a",
        Event::ToolCalled { .. } => "tool.called",
        Event::ToolResult { .. } => "tool.result",
        Event::ContextWindowApproachingLimit { .. } => "context.window.limit",
        Event::MemoryArchiveComplete { .. } => "memory.archive.complete",
        Event::ModeChanged { .. } => "mode.changed",
        Event::Extension(_) => "extension",
        Event::TriggerFired(_) => "trigger.fired",
        Event::Custom { .. } => "custom",
        Event::Shutdown => "shutdown",
    }
}

/// 检查规则的 `watch_event` 是否匹配给定事件。
///
/// `custom.*` 格式支持对 `Custom { event_type }` 的精确子类型匹配。
fn type_tag_matches(watch_event: &str, type_tag: &'static str, event: &Event) -> bool {
    // 精确等值匹配（非 custom 情形）
    if watch_event == type_tag {
        return true;
    }

    // custom.<sub_type> 格式：精确匹配 Custom.event_type
    if let Some(sub) = watch_event.strip_prefix("custom.") {
        if let Event::Custom { event_type, .. } = event {
            return event_type == sub;
        }
    }

    false
}

/// 将 Event 序列化为 JSON payload（用于 payload_filter 比较）。
///
/// 序列化失败时返回 `Value::Null`（不影响正常事件处理）。
fn event_to_payload(event: &Event) -> serde_json::Value {
    serde_json::to_value(event).unwrap_or(serde_json::Value::Null)
}

/// 检查事件 payload 是否包含 filter 中的所有 key/value 对（浅层等值）。
///
/// - `filter` 必须是 `Value::Object`；若非 Object 则始终返回 `false`。
/// - `payload` 中对应 key 的值必须与 filter 中的值深度相等。
fn payload_matches(filter: &serde_json::Value, payload: &serde_json::Value) -> bool {
    let filter_obj = match filter.as_object() {
        Some(o) => o,
        None => return false,
    };

    // 将 payload 展平为 map（支持顶层和嵌套一层）
    let payload_map: HashMap<String, &serde_json::Value> = match payload.as_object() {
        Some(outer) => {
            // 展开一层嵌套（如 {"AgentStopped": {"agent_id": ..., "reason": ...}}）
            let mut map = HashMap::new();
            for (k, v) in outer {
                map.insert(k.clone(), v);
                if let Some(inner_obj) = v.as_object() {
                    for (ik, iv) in inner_obj {
                        map.insert(ik.clone(), iv);
                    }
                }
            }
            map
        }
        None => return false,
    };

    filter_obj
        .iter()
        .all(|(k, fv)| payload_map.get(k).map_or(false, |pv| *pv == fv))
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent_types::AgentId,
        event_bus::EventBus,
        events::Event,
        trigger_event::TriggerType,
    };
    use std::sync::Arc;
    use tokio::time::{sleep, Duration};

    // ── 辅助 ───────────────────────────────────────────────────────────────────

    fn make_registry() -> (Arc<EventBus>, EventTriggerRegistry) {
        let bus = Arc::new(EventBus::new());
        let registry = EventTriggerRegistry::new(Arc::clone(&bus));
        (bus, registry)
    }

    // ── 1. type_tag 映射 ───────────────────────────────────────────────────────

    #[test]
    fn test_event_type_tag_all_variants() {
        let agent_id = AgentId::new("a1");
        let cases: &[(Event, &str)] = &[
            (Event::AgentStarted { agent_id: agent_id.clone() }, "agent.started"),
            (
                Event::AgentStopped { agent_id: agent_id.clone(), reason: "ok".into() },
                "agent.stopped",
            ),
            (
                Event::AgentRestarted { agent_id: agent_id.clone(), attempt: 1, delay_ms: 0 },
                "agent.restarted",
            ),
            (
                Event::AgentFailed { agent_id: agent_id.clone(), attempts: 3, reason: "err".into() },
                "agent.failed",
            ),
            (
                Event::LlmRequestStarted { agent_id: agent_id.clone(), provider: "openai".into() },
                "llm.request.started",
            ),
            (
                Event::LlmRequestCompleted {
                    agent_id: agent_id.clone(),
                    prompt_tokens: 10,
                    completion_tokens: 20,
                },
                "llm.request.completed",
            ),
            (
                Event::MessageReceived {
                    agent_id: agent_id.clone(),
                    channel: "webhook".into(),
                    message_type: "text".into(),
                },
                "message.received",
            ),
            (
                Event::ToolCalled {
                    agent_id: agent_id.clone(),
                    tool_name: "search".into(),
                    call_id: "c1".into(),
                },
                "tool.called",
            ),
            (
                Event::ToolResult {
                    agent_id: agent_id.clone(),
                    tool_name: "search".into(),
                    call_id: "c1".into(),
                    success: true,
                },
                "tool.result",
            ),
            (
                Event::ContextWindowApproachingLimit {
                    agent_id: agent_id.clone(),
                    token_count: 900,
                    token_limit: 1000,
                },
                "context.window.limit",
            ),
            (
                Event::MemoryArchiveComplete { agent_id: agent_id.clone(), archived_count: 5 },
                "memory.archive.complete",
            ),
            (Event::ModeChanged { agent_id: agent_id.clone(), to_power_mode: true }, "mode.changed"),
            (Event::Shutdown, "shutdown"),
            (
                Event::TriggerFired(TriggerEvent::cron("t1", None)),
                "trigger.fired",
            ),
            (
                Event::Custom { event_type: "user.login".into(), data: serde_json::Value::Null },
                "custom",
            ),
        ];

        for (event, expected_tag) in cases {
            assert_eq!(
                event_type_tag(event),
                *expected_tag,
                "event_type_tag mismatch for {:?}",
                event
            );
        }
    }

    // ── 2. type_tag_matches: custom.* 子类型 ──────────────────────────────────

    #[test]
    fn test_type_tag_matches_custom_subtype() {
        let event = Event::Custom { event_type: "user.login".into(), data: serde_json::Value::Null };
        // "custom.user.login" 精确匹配 Custom { event_type: "user.login" }
        assert!(type_tag_matches("custom.user.login", "custom", &event));
        // "custom.user.logout" 不匹配 event_type: "user.login"
        assert!(!type_tag_matches("custom.user.logout", "custom", &event));
        // "custom"（无子类型）精确匹配 type_tag == "custom"，即匹配所有 Custom 事件
        assert!(type_tag_matches("custom", "custom", &event));
    }

    #[test]
    fn test_type_tag_matches_exact() {
        let event = Event::AgentStarted { agent_id: AgentId::new("a1") };
        assert!(type_tag_matches("agent.started", "agent.started", &event));
        assert!(!type_tag_matches("agent.stopped", "agent.started", &event));
    }

    // ── 3. payload_matches ────────────────────────────────────────────────────

    #[test]
    fn test_payload_matches_empty_filter_always_true() {
        let filter = serde_json::json!({});
        let payload = serde_json::json!({"AgentStopped": {"agent_id": {"0": "a1"}, "reason": "ok"}});
        assert!(payload_matches(&filter, &payload));
    }

    #[test]
    fn test_payload_matches_nested_key() {
        let filter = serde_json::json!({"reason": "crashed"});
        let payload = serde_json::json!({"AgentStopped": {"agent_id": {"0": "a1"}, "reason": "crashed"}});
        assert!(payload_matches(&filter, &payload));

        let filter_no = serde_json::json!({"reason": "ok"});
        assert!(!payload_matches(&filter_no, &payload));
    }

    #[test]
    fn test_payload_matches_non_object_filter_returns_false() {
        let filter = serde_json::json!("not-an-object");
        let payload = serde_json::json!({"key": "val"});
        assert!(!payload_matches(&filter, &payload));
    }

    // ── 4. register / unregister / list ───────────────────────────────────────

    #[test]
    fn test_register_and_list() {
        let (bus, registry) = make_registry();
        drop(bus);

        registry.register(EventTriggerRule::new("r1", "agent.started"));
        registry.register(EventTriggerRule::new("r2", "agent.stopped"));

        let list = registry.list();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_unregister_existing() {
        let (bus, registry) = make_registry();
        drop(bus);

        registry.register(EventTriggerRule::new("r1", "agent.started"));
        assert!(registry.unregister("r1"));
        assert!(!registry.unregister("r1")); // 第二次返回 false
        assert!(registry.list().is_empty());
    }

    #[test]
    fn test_register_overwrites_same_id() {
        let (bus, registry) = make_registry();
        drop(bus);

        registry.register(EventTriggerRule::new("r1", "agent.started"));
        registry.register(EventTriggerRule::new("r1", "agent.stopped")); // 覆盖
        let list = registry.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].watch_event, "agent.stopped");
    }

    // ── 5. run() 匹配并发布 TriggerFired ──────────────────────────────────────

    #[tokio::test]
    async fn test_run_fires_trigger_on_matching_event() {
        let bus = Arc::new(EventBus::new());
        let registry = EventTriggerRegistry::new(Arc::clone(&bus));

        registry.register(EventTriggerRule::new("on-start", "agent.started"));

        // 订阅 EventBus 监听 TriggerFired
        let mut rx = bus.subscribe();

        // 启动 registry 循环
        tokio::spawn(registry.run());
        sleep(Duration::from_millis(10)).await;

        // 发布一个 AgentStarted 事件
        bus.publish(Event::AgentStarted { agent_id: AgentId::new("test-agent") });

        // 等待 TriggerFired 出现
        let timeout = tokio::time::timeout(Duration::from_millis(200), async {
            loop {
                if let Ok(event) = rx.recv().await {
                    if let Event::TriggerFired(te) = event {
                        return te;
                    }
                }
            }
        })
        .await
        .expect("TriggerFired 应在 200ms 内发布");

        assert_eq!(timeout.trigger_id, "on-start");
        assert_eq!(timeout.trigger_type, TriggerType::Event);
    }

    // ── 6. run() 不匹配时不触发 ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_run_no_fire_on_non_matching_event() {
        let bus = Arc::new(EventBus::new());
        let registry = EventTriggerRegistry::new(Arc::clone(&bus));

        // 只监听 agent.stopped
        registry.register(EventTriggerRule::new("on-stop", "agent.stopped"));

        let mut rx = bus.subscribe();
        tokio::spawn(registry.run());
        sleep(Duration::from_millis(10)).await;

        // 发布一个 AgentStarted（不匹配）
        bus.publish(Event::AgentStarted { agent_id: AgentId::new("a1") });

        // 100ms 内不应出现 TriggerFired
        let result = tokio::time::timeout(Duration::from_millis(100), async {
            loop {
                if let Ok(Event::TriggerFired(_)) = rx.recv().await {
                    return true;
                }
            }
        })
        .await;

        assert!(result.is_err(), "不匹配的事件不应触发 TriggerFired");
    }

    // ── 7. payload_filter 过滤 ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_run_payload_filter_blocks_non_matching() {
        let bus = Arc::new(EventBus::new());
        let registry = EventTriggerRegistry::new(Arc::clone(&bus));

        // 只有 reason == "crashed" 才触发
        let rule = EventTriggerRule::new("on-crash", "agent.stopped")
            .with_filter(serde_json::json!({"reason": "crashed"}));
        registry.register(rule);

        let mut rx = bus.subscribe();
        tokio::spawn(registry.run());
        sleep(Duration::from_millis(10)).await;

        // 发布 reason = "ok"，不匹配
        bus.publish(Event::AgentStopped {
            agent_id: AgentId::new("a1"),
            reason: "ok".into(),
        });

        let result = tokio::time::timeout(Duration::from_millis(100), async {
            loop {
                if let Ok(Event::TriggerFired(_)) = rx.recv().await {
                    return true;
                }
            }
        })
        .await;

        assert!(result.is_err(), "reason=ok 不应触发");
    }

    #[tokio::test]
    async fn test_run_payload_filter_allows_matching() {
        let bus = Arc::new(EventBus::new());
        let registry = EventTriggerRegistry::new(Arc::clone(&bus));

        let rule = EventTriggerRule::new("on-crash", "agent.stopped")
            .with_filter(serde_json::json!({"reason": "crashed"}));
        registry.register(rule);

        let mut rx = bus.subscribe();
        tokio::spawn(registry.run());
        sleep(Duration::from_millis(10)).await;

        // 发布 reason = "crashed"，匹配
        bus.publish(Event::AgentStopped {
            agent_id: AgentId::new("a2"),
            reason: "crashed".into(),
        });

        let te = tokio::time::timeout(Duration::from_millis(200), async {
            loop {
                if let Ok(Event::TriggerFired(te)) = rx.recv().await {
                    return te;
                }
            }
        })
        .await
        .expect("TriggerFired 应在 200ms 内发布");

        assert_eq!(te.trigger_id, "on-crash");
    }

    // ── 8. custom.* 子类型匹配 ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_run_custom_subtype_matching() {
        let bus = Arc::new(EventBus::new());
        let registry = EventTriggerRegistry::new(Arc::clone(&bus));

        registry.register(EventTriggerRule::new("on-login", "custom.user.login"));

        let mut rx = bus.subscribe();
        tokio::spawn(registry.run());
        sleep(Duration::from_millis(10)).await;

        // 匹配
        bus.publish(Event::Custom {
            event_type: "user.login".into(),
            data: serde_json::json!({"user": "alice"}),
        });

        let te = tokio::time::timeout(Duration::from_millis(200), async {
            loop {
                if let Ok(Event::TriggerFired(te)) = rx.recv().await {
                    return te;
                }
            }
        })
        .await
        .expect("TriggerFired 应在 200ms 内发布");

        assert_eq!(te.trigger_id, "on-login");

        // 不匹配的 custom 子类型
        bus.publish(Event::Custom {
            event_type: "user.logout".into(),
            data: serde_json::Value::Null,
        });

        let result = tokio::time::timeout(Duration::from_millis(100), async {
            loop {
                if let Ok(Event::TriggerFired(_)) = rx.recv().await {
                    return true;
                }
            }
        })
        .await;
        assert!(result.is_err(), "user.logout 不应触发 on-login 规则");
    }

    // ── 9. target_agent 传递到 TriggerEvent ──────────────────────────────────

    #[tokio::test]
    async fn test_run_target_agent_propagated() {
        let bus = Arc::new(EventBus::new());
        let registry = EventTriggerRegistry::new(Arc::clone(&bus));

        let target = AgentId::new("handler-agent");
        let rule = EventTriggerRule::new("on-start", "agent.started")
            .with_target(target.clone());
        registry.register(rule);

        let mut rx = bus.subscribe();
        tokio::spawn(registry.run());
        sleep(Duration::from_millis(10)).await;

        bus.publish(Event::AgentStarted { agent_id: AgentId::new("a1") });

        let te = tokio::time::timeout(Duration::from_millis(200), async {
            loop {
                if let Ok(Event::TriggerFired(te)) = rx.recv().await {
                    return te;
                }
            }
        })
        .await
        .expect("TriggerFired 应发布");

        assert_eq!(te.target_agent, Some(target));
    }
}
