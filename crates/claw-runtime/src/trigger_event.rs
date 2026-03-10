//! TriggerEvent — 触发系统的核心数据载体（GAP-F6-01）。
//!
//! 所有触发源（Cron、Webhook、Event）产生的触发事件均统一表示为 [`TriggerEvent`]，
//! 通过 [`EventBus`](crate::event_bus::EventBus) 以 [`Event::TriggerFired`](crate::events::Event) 形式广播。

use crate::agent_types::AgentId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 触发类型 — 区分触发事件的来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerType {
    /// 定时触发（Cron 表达式或 Interval）。
    Cron,
    /// 外部 HTTP Webhook 触发。
    Webhook,
    /// 内部事件总线事件触发。
    Event,
}

impl std::fmt::Display for TriggerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TriggerType::Cron => write!(f, "cron"),
            TriggerType::Webhook => write!(f, "webhook"),
            TriggerType::Event => write!(f, "event"),
        }
    }
}

/// 触发事件 — 触发系统在 EventBus 上传播的统一事件类型。
///
/// 每次触发器触发时生成一个新的 `TriggerEvent`，通过
/// [`Event::TriggerFired`](crate::events::Event) 发布到 [`EventBus`](crate::event_bus::EventBus)。
///
/// # 字段
///
/// - `id`: 本次触发的唯一 ID（UUID v4），用于审计和去重。
/// - `trigger_id`: 触发源配置的 ID（与 `TriggerStore` 中的记录对应）。
/// - `trigger_type`: 触发来源类别。
/// - `fired_at`: 触发时刻（UTC）。
/// - `payload`: 触发时携带的数据。Cron 触发为 `null`，Webhook 触发为请求体 JSON。
/// - `target_agent`: 指定目标 Agent；`None` 表示广播到所有在线 Agent。
///
/// # 示例
///
/// ```rust
/// use claw_runtime::trigger_event::{TriggerEvent, TriggerType};
/// use claw_runtime::agent_types::AgentId;
///
/// let event = TriggerEvent::new(
///     "my-cron-trigger",
///     TriggerType::Cron,
///     serde_json::Value::Null,
///     None,
/// );
/// assert_eq!(event.trigger_type, TriggerType::Cron);
/// assert!(event.target_agent.is_none());
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerEvent {
    /// 本次触发的唯一 ID（UUID v4）。
    pub id: Uuid,
    /// 触发源配置 ID（对应 TriggerStore 中的持久化记录）。
    pub trigger_id: String,
    /// 触发类型。
    pub trigger_type: TriggerType,
    /// 触发时刻（UTC）。
    pub fired_at: DateTime<Utc>,
    /// 触发携带的 payload。Cron 为 `null`，Webhook 为请求体 JSON。
    pub payload: serde_json::Value,
    /// 目标 Agent ID；`None` 表示广播到所有在线 Agent。
    pub target_agent: Option<AgentId>,
}

impl TriggerEvent {
    /// 创建一个新的触发事件，自动生成 UUID 并记录当前时刻。
    pub fn new(
        trigger_id: impl Into<String>,
        trigger_type: TriggerType,
        payload: serde_json::Value,
        target_agent: Option<AgentId>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            trigger_id: trigger_id.into(),
            trigger_type,
            fired_at: Utc::now(),
            payload,
            target_agent,
        }
    }

    /// 创建一个 Cron 触发事件（payload 为 null）。
    pub fn cron(trigger_id: impl Into<String>, target_agent: Option<AgentId>) -> Self {
        Self::new(trigger_id, TriggerType::Cron, serde_json::Value::Null, target_agent)
    }

    /// 创建一个 Webhook 触发事件。
    pub fn webhook(
        trigger_id: impl Into<String>,
        payload: serde_json::Value,
        target_agent: Option<AgentId>,
    ) -> Self {
        Self::new(trigger_id, TriggerType::Webhook, payload, target_agent)
    }

    /// 创建一个内部事件触发事件。
    pub fn event(
        trigger_id: impl Into<String>,
        payload: serde_json::Value,
        target_agent: Option<AgentId>,
    ) -> Self {
        Self::new(trigger_id, TriggerType::Event, payload, target_agent)
    }

    /// 返回是否为广播触发（无指定目标 Agent）。
    pub fn is_broadcast(&self) -> bool {
        self.target_agent.is_none()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trigger_event_new_generates_unique_ids() {
        let e1 = TriggerEvent::new("t1", TriggerType::Cron, serde_json::Value::Null, None);
        let e2 = TriggerEvent::new("t1", TriggerType::Cron, serde_json::Value::Null, None);
        assert_ne!(e1.id, e2.id, "每次创建应生成不同的 UUID");
    }

    #[test]
    fn test_trigger_event_cron_has_null_payload() {
        let e = TriggerEvent::cron("daily-job", None);
        assert_eq!(e.trigger_type, TriggerType::Cron);
        assert_eq!(e.payload, serde_json::Value::Null);
        assert!(e.is_broadcast());
    }

    #[test]
    fn test_trigger_event_webhook_carries_payload() {
        let payload = serde_json::json!({"action": "push", "ref": "refs/heads/main"});
        let agent = AgentId::new("agent-42");
        let e = TriggerEvent::webhook("gh-webhook", payload.clone(), Some(agent.clone()));
        assert_eq!(e.trigger_type, TriggerType::Webhook);
        assert_eq!(e.payload, payload);
        assert_eq!(e.target_agent, Some(agent));
        assert!(!e.is_broadcast());
    }

    #[test]
    fn test_trigger_event_serialization_roundtrip() {
        let e = TriggerEvent::event(
            "my-event-trigger",
            serde_json::json!({"key": "value"}),
            Some(AgentId::new("agent-1")),
        );
        let json = serde_json::to_string(&e).expect("序列化应成功");
        let decoded: TriggerEvent = serde_json::from_str(&json).expect("反序列化应成功");
        assert_eq!(decoded.id, e.id);
        assert_eq!(decoded.trigger_id, e.trigger_id);
        assert_eq!(decoded.trigger_type, TriggerType::Event);
        assert_eq!(decoded.target_agent, e.target_agent);
    }

    #[test]
    fn test_trigger_type_display() {
        assert_eq!(TriggerType::Cron.to_string(), "cron");
        assert_eq!(TriggerType::Webhook.to_string(), "webhook");
        assert_eq!(TriggerType::Event.to_string(), "event");
    }
}
