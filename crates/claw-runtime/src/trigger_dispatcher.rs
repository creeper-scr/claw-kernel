//! TriggerDispatcher — 将 EventBus 上的 TriggerFired 事件路由到目标 Agent（GAP-F6-03）。
//!
//! `TriggerDispatcher` 在后台订阅 [`EventBus`]，监听所有
//! [`Event::TriggerFired`](crate::events::Event::TriggerFired) 事件，并将触发消息
//! 通过 [`AgentOrchestrator::steer`] 注入目标 Agent。
//!
//! # 消息来源优先级
//!
//! 1. `TriggerEvent::payload["message"]` — Webhook / Event 触发时由调用方在 payload 中
//!    携带自定义消息文本。
//! 2. 回退值 `"triggered"` — Cron 触发或 payload 中无 `message` 字段时使用。
//!
//! # 广播行为
//!
//! 当 [`TriggerEvent::target_agent`] 为 `None` 时，消息广播到编排器中所有已注册 Agent。
//!
//! # 示例
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use claw_runtime::{AgentOrchestrator, EventBus};
//! use claw_runtime::trigger_dispatcher::TriggerDispatcher;
//!
//! # #[tokio::main]
//! # async fn main() {
//! let bus = Arc::new(EventBus::new());
//! let pm = Arc::new(claw_runtime::TokioProcessManager::new());
//! let orchestrator = Arc::new(AgentOrchestrator::new(Arc::clone(&bus), pm));
//! let dispatcher = TriggerDispatcher::new(orchestrator, (*bus).clone());
//!
//! // 在独立 tokio 任务中运行（直到 EventBus 关闭）
//! tokio::spawn(dispatcher.run());
//! # }
//! ```

use std::sync::Arc;

use crate::{
    agent_types::AgentId,
    event_bus::EventBus,
    events::Event,
    orchestrator::{AgentOrchestrator, SteerCommand},
    trigger_event::TriggerEvent,
};

// ─── TriggerDispatcher ────────────────────────────────────────────────────────

/// 后台服务：订阅 EventBus 并将触发事件注入目标 Agent。
///
/// 通过 [`TriggerDispatcher::new`] 构造后，调用 [`TriggerDispatcher::run`]
/// 在 tokio 任务中启动循环。循环在 EventBus 关闭时自动退出。
pub struct TriggerDispatcher {
    orchestrator: Arc<AgentOrchestrator>,
    event_bus: EventBus,
}

impl TriggerDispatcher {
    /// 创建一个新的 `TriggerDispatcher`。
    pub fn new(orchestrator: Arc<AgentOrchestrator>, event_bus: EventBus) -> Self {
        Self {
            orchestrator,
            event_bus,
        }
    }

    /// 启动事件循环（阻塞，直到 EventBus 关闭）。
    ///
    /// 建议在 `tokio::spawn` 中调用：
    ///
    /// ```rust,ignore
    /// tokio::spawn(dispatcher.run());
    /// ```
    ///
    /// 订阅所有事件，只处理 [`Event::TriggerFired`] 变体，其余跳过。
    /// 当 EventBus 关闭或发生不可恢复错误时退出。
    pub async fn run(self) {
        let mut rx = self.event_bus.subscribe();

        loop {
            match rx.recv().await {
                Ok(Event::TriggerFired(trigger_event)) => {
                    self.dispatch(trigger_event).await;
                }
                Ok(_) => {
                    // 非 TriggerFired 事件，跳过。
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "TriggerDispatcher: EventBus error, stopping"
                    );
                    break;
                }
            }
        }
    }

    /// 将单个触发事件注入目标 Agent（或广播到所有 Agent）。
    async fn dispatch(&self, ev: TriggerEvent) {
        // 消息内容：优先从 payload["message"] 读取，否则用 "triggered" 兜底。
        let message = ev
            .payload
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("triggered")
            .to_string();

        tracing::debug!(
            trigger_id = %ev.trigger_id,
            trigger_type = %ev.trigger_type,
            target_agent = ?ev.target_agent,
            message = %message,
            "TriggerDispatcher: dispatching trigger event"
        );

        match ev.target_agent {
            Some(agent_id) => {
                self.inject_message(&agent_id, message).await;
            }
            None => {
                // 广播到所有已注册 Agent
                let ids: Vec<AgentId> = self.orchestrator.agent_ids();
                if ids.is_empty() {
                    tracing::debug!(
                        trigger_id = %ev.trigger_id,
                        "TriggerDispatcher: broadcast trigger fired but no agents registered"
                    );
                }
                for id in ids {
                    self.inject_message(&id, message.clone()).await;
                }
            }
        }
    }

    /// 向单个 Agent 注入消息，记录失败但不中止循环。
    async fn inject_message(&self, agent_id: &AgentId, message: String) {
        if let Err(e) = self
            .orchestrator
            .steer(agent_id, SteerCommand::InjectMessage(message))
            .await
        {
            tracing::warn!(
                agent_id = %agent_id.0,
                error = %e,
                "TriggerDispatcher: failed to inject message into agent"
            );
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent_types::{AgentConfig, AgentId},
        event_bus::EventBus,
        events::Event,
        orchestrator::AgentOrchestrator,
        trigger_event::{TriggerEvent, TriggerType},
        TokioProcessManager,
    };
    use std::sync::Arc;

    fn make_orchestrator(bus: &Arc<EventBus>) -> Arc<AgentOrchestrator> {
        let pm: Arc<dyn crate::ProcessManager> = Arc::new(TokioProcessManager::new());
        Arc::new(AgentOrchestrator::new_for_test(Arc::clone(bus), pm))
    }

    // ── 1. dispatch 向已注册 Agent 注入消息 ───────────────────────────────────
    #[tokio::test]
    async fn test_dispatch_injects_message_to_target_agent() {
        let bus = Arc::new(EventBus::new());
        let orchestrator = make_orchestrator(&bus);

        let config = AgentConfig::new("dispatch-test");
        let agent_id = config.agent_id.clone();
        orchestrator.register(config).unwrap();

        let dispatcher = TriggerDispatcher::new(Arc::clone(&orchestrator), (*bus).clone());

        let ev = TriggerEvent {
            id: uuid::Uuid::new_v4(),
            trigger_id: "t1".into(),
            trigger_type: TriggerType::Cron,
            fired_at: chrono::Utc::now(),
            payload: serde_json::json!({ "message": "hello from cron" }),
            target_agent: Some(agent_id),
        };

        // dispatch 调用 steer(InjectMessage) — 注册 agent（无 ipc_tx）走 out-of-process
        // 路径，通过 EventBus 广播 Custom 事件，不会 panic。
        dispatcher.dispatch(ev).await;
    }

    // ── 2. dispatch 在 target_agent=None 时广播到所有 Agent ─────────────────
    #[tokio::test]
    async fn test_dispatch_broadcasts_when_no_target() {
        let bus = Arc::new(EventBus::new());
        let orchestrator = make_orchestrator(&bus);

        for i in 0..3 {
            let cfg = AgentConfig::new(format!("broadcast-{i}"));
            orchestrator.register(cfg).unwrap();
        }

        let dispatcher = TriggerDispatcher::new(Arc::clone(&orchestrator), (*bus).clone());

        let ev = TriggerEvent::cron("broadcast-trigger", None);
        dispatcher.dispatch(ev).await;
    }

    // ── 3. dispatch 在 payload 无 message 字段时使用 "triggered" 兜底 ───────
    #[tokio::test]
    async fn test_dispatch_fallback_message() {
        let bus = Arc::new(EventBus::new());
        let orchestrator = make_orchestrator(&bus);

        let cfg = AgentConfig::new("fallback");
        let agent_id = cfg.agent_id.clone();
        orchestrator.register(cfg).unwrap();

        let dispatcher = TriggerDispatcher::new(Arc::clone(&orchestrator), (*bus).clone());

        // payload 中没有 "message" 字段
        let ev = TriggerEvent {
            id: uuid::Uuid::new_v4(),
            trigger_id: "t-fallback".into(),
            trigger_type: TriggerType::Webhook,
            fired_at: chrono::Utc::now(),
            payload: serde_json::json!({ "action": "push" }),
            target_agent: Some(agent_id),
        };

        dispatcher.dispatch(ev).await;
    }

    // ── 4. dispatch 在 target_agent 不存在时记录警告但不 panic ────────────────
    #[tokio::test]
    async fn test_dispatch_nonexistent_agent_does_not_panic() {
        let bus = Arc::new(EventBus::new());
        let orchestrator = make_orchestrator(&bus);
        let dispatcher = TriggerDispatcher::new(Arc::clone(&orchestrator), (*bus).clone());

        let ev = TriggerEvent {
            id: uuid::Uuid::new_v4(),
            trigger_id: "ghost-trigger".into(),
            trigger_type: TriggerType::Event,
            fired_at: chrono::Utc::now(),
            payload: serde_json::Value::Null,
            target_agent: Some(AgentId::new("nonexistent-agent")),
        };

        // steer 会返回 AgentNotFound，inject_message 记录 warn，不 panic
        dispatcher.dispatch(ev).await;
    }

    // ── 5. run() 循环在收到 TriggerFired 后处理并继续 ───────────────────────
    #[tokio::test]
    async fn test_run_processes_trigger_fired_event() {
        let bus = Arc::new(EventBus::new());
        let orchestrator = make_orchestrator(&bus);

        let cfg = AgentConfig::new("run-test");
        let agent_id = cfg.agent_id.clone();
        orchestrator.register(cfg).unwrap();

        let dispatcher = TriggerDispatcher::new(Arc::clone(&orchestrator), (*bus).clone());

        // 在后台启动 dispatcher
        let handle = tokio::spawn(dispatcher.run());

        // 稍等确保 dispatcher 已订阅
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // 发布一个 TriggerFired 事件
        let ev = TriggerEvent {
            id: uuid::Uuid::new_v4(),
            trigger_id: "run-trigger".into(),
            trigger_type: TriggerType::Cron,
            fired_at: chrono::Utc::now(),
            payload: serde_json::json!({ "message": "cron fired" }),
            target_agent: Some(agent_id),
        };
        bus.publish(Event::TriggerFired(ev));

        // 稍等 dispatcher 处理
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // dispatcher 应仍在运行（等待下一个事件）
        assert!(!handle.is_finished(), "dispatcher should still be running");

        // 中止 task，验证不会 panic
        handle.abort();
        let result = handle.await;
        assert!(
            result.unwrap_err().is_cancelled(),
            "task should be cancelled, not panicked"
        );
    }
}
