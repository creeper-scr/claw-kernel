//! Priority-aware A2A Message Routing
//!
//! Each registered agent gets 5 independent MPSC channels, one per
//! `MessagePriority` level (Critical=0 … Background=4).  `route_message`
//! writes into the channel that matches the message priority.  The consumer
//! side (`PriorityReceiver`) polls channels from highest priority to lowest,
//! falling back to an `await` on the Critical channel when all queues are
//! empty to avoid busy-looping.

use crate::a2a::protocol::{A2AMessage, MessagePriority};
use crate::agent_types::AgentId;
use crate::error::RuntimeError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

// ─── PriorityReceiver ─────────────────────────────────────────────────────────

/// Consumer-side handle returned to the owning agent when it registers.
///
/// Drain messages in priority order: `recv()` returns the highest-priority
/// available message, blocking only when all five queues are empty.
///
/// Fields are named (not a `[Receiver; 5]` array) so that `tokio::select!`
/// can borrow them independently — Rust does not allow multiple `&mut` borrows
/// into the same array within a single `select!` block.
pub struct PriorityReceiver {
    rx_critical:   mpsc::Receiver<A2AMessage>,
    rx_high:       mpsc::Receiver<A2AMessage>,
    rx_normal:     mpsc::Receiver<A2AMessage>,
    rx_low:        mpsc::Receiver<A2AMessage>,
    rx_background: mpsc::Receiver<A2AMessage>,
}

impl PriorityReceiver {
    /// Return the next message in priority order.
    ///
    /// Strategy:
    /// 1. **Fast path** — `try_recv` from Critical → Background.  If any
    ///    queue has a message waiting, return it immediately (no `.await`).
    /// 2. **Slow path** — `select! { biased; … }` awaits all five channels in
    ///    priority order.  When woken by a lower-priority arm, we do one final
    ///    `try_recv` sweep over all higher-priority channels before returning,
    ///    so a Critical message that arrived while we were suspended is never
    ///    delayed behind a Background message that woke us.
    pub async fn recv(&mut self) -> Option<A2AMessage> {
        loop {
            // ── fast path ──────────────────────────────────────────────────
            if let Ok(m) = self.rx_critical.try_recv()   { return Some(m); }
            if let Ok(m) = self.rx_high.try_recv()       { return Some(m); }
            if let Ok(m) = self.rx_normal.try_recv()     { return Some(m); }
            if let Ok(m) = self.rx_low.try_recv()        { return Some(m); }
            if let Ok(m) = self.rx_background.try_recv() { return Some(m); }

            // ── slow path ──────────────────────────────────────────────────
            tokio::select! {
                biased;
                msg = self.rx_critical.recv() => {
                    return msg;
                }
                msg = self.rx_high.recv() => {
                    let Some(m) = msg else { return None; };
                    if let Ok(hi) = self.rx_critical.try_recv() { return Some(hi); }
                    return Some(m);
                }
                msg = self.rx_normal.recv() => {
                    let Some(m) = msg else { return None; };
                    if let Ok(hi) = self.rx_critical.try_recv() { return Some(hi); }
                    if let Ok(hi) = self.rx_high.try_recv()     { return Some(hi); }
                    return Some(m);
                }
                msg = self.rx_low.recv() => {
                    let Some(m) = msg else { return None; };
                    if let Ok(hi) = self.rx_critical.try_recv() { return Some(hi); }
                    if let Ok(hi) = self.rx_high.try_recv()     { return Some(hi); }
                    if let Ok(hi) = self.rx_normal.try_recv()   { return Some(hi); }
                    return Some(m);
                }
                msg = self.rx_background.recv() => {
                    let Some(m) = msg else { return None; };
                    if let Ok(hi) = self.rx_critical.try_recv()   { return Some(hi); }
                    if let Ok(hi) = self.rx_high.try_recv()       { return Some(hi); }
                    if let Ok(hi) = self.rx_normal.try_recv()     { return Some(hi); }
                    if let Ok(hi) = self.rx_low.try_recv()        { return Some(hi); }
                    return Some(m);
                }
            }
        }
    }
}

// ─── AgentHandle ──────────────────────────────────────────────────────────────

/// Sender-side handle stored inside the router for each registered agent.
#[derive(Debug, Clone)]
pub struct AgentHandle {
    /// The agent's unique identifier.
    pub agent_id: AgentId,
    tx_critical:   mpsc::Sender<A2AMessage>,
    tx_high:       mpsc::Sender<A2AMessage>,
    tx_normal:     mpsc::Sender<A2AMessage>,
    tx_low:        mpsc::Sender<A2AMessage>,
    tx_background: mpsc::Sender<A2AMessage>,
}

impl AgentHandle {
    fn sender_for(&self, priority: MessagePriority) -> &mpsc::Sender<A2AMessage> {
        match priority {
            MessagePriority::Critical   => &self.tx_critical,
            MessagePriority::High       => &self.tx_high,
            MessagePriority::Normal     => &self.tx_normal,
            MessagePriority::Low        => &self.tx_low,
            MessagePriority::Background => &self.tx_background,
        }
    }

    /// Send a message to this agent, routing to the appropriate priority channel.
    pub async fn send(&self, message: A2AMessage) -> Result<(), RuntimeError> {
        self.sender_for(message.priority)
            .send(message)
            .await
            .map_err(|_| RuntimeError::IpcError("agent channel closed".to_string()))
    }

    /// Try to send without blocking.
    pub fn try_send(&self, message: A2AMessage) -> Result<(), RuntimeError> {
        self.sender_for(message.priority)
            .try_send(message)
            .map_err(|_| RuntimeError::IpcError("agent channel full or closed".to_string()))
    }
}

// ─── SimpleRouter ─────────────────────────────────────────────────────────────

/// Priority-aware A2A message router.
///
/// Maintains a registry of local agents with 5-level priority channels.
/// Messages are routed to the channel matching their `MessagePriority`,
/// ensuring Critical messages are processed before Background ones.
#[derive(Debug)]
pub struct SimpleRouter {
    agents: Arc<RwLock<HashMap<AgentId, AgentHandle>>>,
}

impl SimpleRouter {
    /// Create a new priority router.
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new agent with the router.
    ///
    /// Returns:
    /// - `AgentHandle` — cloneable sender-side reference stored in the router
    /// - `PriorityReceiver` — unique consumer-side handle to give to the agent
    ///
    /// `buffer_size` is applied **per priority level** (5 × buffer_size total).
    pub async fn register_agent(
        &self,
        agent_id: AgentId,
        buffer_size: usize,
    ) -> (AgentHandle, PriorityReceiver) {
        let (tx_critical,   rx_critical)   = mpsc::channel::<A2AMessage>(buffer_size);
        let (tx_high,       rx_high)       = mpsc::channel::<A2AMessage>(buffer_size);
        let (tx_normal,     rx_normal)     = mpsc::channel::<A2AMessage>(buffer_size);
        let (tx_low,        rx_low)        = mpsc::channel::<A2AMessage>(buffer_size);
        let (tx_background, rx_background) = mpsc::channel::<A2AMessage>(buffer_size);

        let handle = AgentHandle {
            agent_id: agent_id.clone(),
            tx_critical,
            tx_high,
            tx_normal,
            tx_low,
            tx_background,
        };

        let receiver = PriorityReceiver {
            rx_critical,
            rx_high,
            rx_normal,
            rx_low,
            rx_background,
        };

        let mut agents = self.agents.write().await;
        agents.insert(agent_id, handle.clone());

        (handle, receiver)
    }

    /// Unregister an agent from the router.
    pub async fn unregister_agent(&self, agent_id: &AgentId) -> Result<(), RuntimeError> {
        let mut agents = self.agents.write().await;
        if agents.remove(agent_id).is_none() {
            return Err(RuntimeError::AgentNotFound(agent_id.0.clone()));
        }
        Ok(())
    }

    /// Check if an agent is registered.
    pub async fn is_agent_registered(&self, agent_id: &AgentId) -> bool {
        let agents = self.agents.read().await;
        agents.contains_key(agent_id)
    }

    /// Get a clone of the sender-side handle for a registered agent.
    pub async fn get_agent(&self, agent_id: &AgentId) -> Option<AgentHandle> {
        let agents = self.agents.read().await;
        agents.get(agent_id).cloned()
    }

    /// Get list of all registered agent IDs.
    pub async fn get_agent_ids(&self) -> Vec<AgentId> {
        let agents = self.agents.read().await;
        agents.keys().cloned().collect()
    }

    /// Route a message to its target using the message's priority channel.
    ///
    /// - If `target` is set, attempts direct priority-aware delivery.
    /// - If `target` is `None`, broadcasts to all agents (excluding sender).
    /// - Expired messages (GAP-F7-02) are silently dropped before routing.
    pub async fn route_message(&self, message: A2AMessage) -> Result<(), RuntimeError> {
        if message.is_expired() {
            tracing::debug!(
                id = %message.id,
                priority = ?message.priority,
                "Dropping expired A2A message"
            );
            return Ok(());
        }

        let target = match &message.target {
            Some(t) => t.clone(),
            None => return self.broadcast_message(message).await,
        };

        if let Some(handle) = self.get_agent(&target).await {
            handle.send(message).await?;
            Ok(())
        } else {
            Err(RuntimeError::AgentNotFound(target.0.clone()))
        }
    }

    /// Send a message directly to a specific agent (priority-aware).
    pub async fn send(&self, target: &AgentId, message: A2AMessage) -> Result<(), RuntimeError> {
        if let Some(handle) = self.get_agent(target).await {
            handle.send(message).await
        } else {
            Err(RuntimeError::AgentNotFound(target.0.clone()))
        }
    }

    /// Broadcast a message to all registered agents except the sender.
    async fn broadcast_message(&self, message: A2AMessage) -> Result<(), RuntimeError> {
        let agents = self.agents.read().await;
        for (agent_id, handle) in agents.iter() {
            if message.source == *agent_id {
                continue;
            }
            let _ = handle.try_send(message.clone());
        }
        Ok(())
    }
}

impl Default for SimpleRouter {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a2a::protocol::{A2AMessagePayload, A2AMessageType, MessagePriority};

    fn make_msg(id: &str, source: &str, target: Option<&str>, priority: MessagePriority) -> A2AMessage {
        let mut msg = A2AMessage::new(
            id,
            AgentId::new(source),
            A2AMessageType::Event,
            A2AMessagePayload::Event {
                event_type: "test".to_string(),
                data: Default::default(),
            },
        )
        .with_priority(priority);

        if let Some(t) = target {
            msg = msg.with_target(AgentId::new(t));
        }

        msg
    }

    #[tokio::test]
    async fn test_register_unregister() {
        let router = SimpleRouter::new();
        let agent_id = AgentId::new("test-agent");

        let (_handle, _rx) = router.register_agent(agent_id.clone(), 100).await;
        assert!(router.is_agent_registered(&agent_id).await);

        router.unregister_agent(&agent_id).await.unwrap();
        assert!(!router.is_agent_registered(&agent_id).await);
    }

    #[tokio::test]
    async fn test_route_to_registered() {
        let router = SimpleRouter::new();
        let target_id = AgentId::new("target");
        let (_handle, _rx) = router.register_agent(target_id.clone(), 100).await;

        let msg = make_msg("msg-1", "source", Some("target"), MessagePriority::Normal);
        router.route_message(msg).await.unwrap();
    }

    #[tokio::test]
    async fn test_route_to_unregistered_fails() {
        let router = SimpleRouter::new();
        let msg = make_msg("msg-1", "source", Some("unregistered"), MessagePriority::Normal);
        assert!(router.route_message(msg).await.is_err());
    }

    #[tokio::test]
    async fn test_broadcast() {
        let router = SimpleRouter::new();
        let (_h1, _r1) = router.register_agent(AgentId::new("agent-1"), 100).await;
        let (_h2, _r2) = router.register_agent(AgentId::new("agent-2"), 100).await;

        let msg = make_msg("broadcast", "source", None, MessagePriority::Normal);
        router.route_message(msg).await.unwrap();
    }

    #[tokio::test]
    async fn test_send_direct() {
        let router = SimpleRouter::new();
        let target_id = AgentId::new("target");
        let (_handle, _rx) = router.register_agent(target_id.clone(), 100).await;

        let msg = make_msg("msg-1", "source", Some("target"), MessagePriority::Normal);
        router.send(&target_id, msg).await.unwrap();
    }

    #[tokio::test]
    async fn test_get_agent_ids() {
        let router = SimpleRouter::new();
        router.register_agent(AgentId::new("agent-1"), 100).await;
        router.register_agent(AgentId::new("agent-2"), 100).await;

        let ids = router.get_agent_ids().await;
        assert_eq!(ids.len(), 2);
    }

    /// GAP-F7-01 核心验证：Critical 消息必须先于 Background 消息被消费。
    #[tokio::test]
    async fn test_priority_order_critical_before_background() {
        let router = SimpleRouter::new();
        let target_id = AgentId::new("target");
        let (_handle, mut rx) = router.register_agent(target_id.clone(), 100).await;

        // 先发送 Background，再发送 Critical
        let bg_msg   = make_msg("bg",   "source", Some("target"), MessagePriority::Background);
        let crit_msg = make_msg("crit", "source", Some("target"), MessagePriority::Critical);

        router.route_message(bg_msg).await.unwrap();
        router.route_message(crit_msg).await.unwrap();

        // PriorityReceiver 应先返回 Critical
        let first = rx.recv().await.unwrap();
        assert_eq!(first.priority, MessagePriority::Critical, "Critical must come first");

        let second = rx.recv().await.unwrap();
        assert_eq!(second.priority, MessagePriority::Background, "Background comes second");
    }

    /// 验证所有 5 个优先级通道都被正确路由。
    #[tokio::test]
    async fn test_all_five_priority_levels_routed() {
        let router = SimpleRouter::new();
        let target_id = AgentId::new("target");
        let (_handle, mut rx) = router.register_agent(target_id.clone(), 100).await;

        // 按从低到高顺序发送
        for (i, &p) in [
            MessagePriority::Background,
            MessagePriority::Low,
            MessagePriority::Normal,
            MessagePriority::High,
            MessagePriority::Critical,
        ].iter().enumerate() {
            let msg = make_msg(&format!("msg-{i}"), "source", Some("target"), p);
            router.route_message(msg).await.unwrap();
        }

        // 收取时应该从高到低
        for expected_priority in [
            MessagePriority::Critical,
            MessagePriority::High,
            MessagePriority::Normal,
            MessagePriority::Low,
            MessagePriority::Background,
        ] {
            let msg = rx.recv().await.unwrap();
            assert_eq!(
                msg.priority, expected_priority,
                "Expected {:?}, got {:?}", expected_priority, msg.priority
            );
        }
    }

    /// GAP-F7-02 顺带验证：过期消息被静默丢弃。
    #[tokio::test]
    async fn test_expired_message_dropped() {
        let router = SimpleRouter::new();
        let target_id = AgentId::new("target");
        let (_handle, _rx) = router.register_agent(target_id.clone(), 100).await;

        let mut msg = make_msg("expired", "source", Some("target"), MessagePriority::Critical);
        msg.timestamp = 0;
        msg.ttl_secs = Some(1);

        let result = router.route_message(msg).await;
        assert!(result.is_ok(), "Expired message should be silently dropped");
    }

    /// try_send 写入正确的优先级通道。
    #[tokio::test]
    async fn test_try_send_priority_channel() {
        let router = SimpleRouter::new();
        let target_id = AgentId::new("target");
        let (handle, mut rx) = router.register_agent(target_id.clone(), 100).await;

        let msg = make_msg("hi-prio", "source", None, MessagePriority::High);
        handle.try_send(msg).unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received.priority, MessagePriority::High);
        assert_eq!(received.id, "hi-prio");
    }
}
