//! IpcAgentHandle: typed handle for communicating with a spawned agent over IPC.
//!
//! Unlike [`AgentHandle`](crate::agent_types::AgentHandle) (which only holds an
//! event bus reference), `IpcAgentHandle` carries a [`SharedSender`] — a
//! mutex-wrapped optional mpsc sender — so callers can send request/response
//! messages directly to the agent's internal message loop.
//!
//! The [`SharedSender`] design enables **transparent hot-swap on restart**: when
//! the orchestrator restarts a failed agent it swaps the inner sender in-place,
//! so all cloned `IpcAgentHandle` instances automatically route future messages
//! to the new loop without the caller needing to obtain a fresh handle.
//!
//! # Usage
//!
//! ```rust,ignore
//! use claw_runtime::{IpcAgentHandle, AgentResponse};
//! use std::time::Duration;
//!
//! // Fire-and-forget
//! handle.send("hello").await?;
//!
//! // Wait for a response with a 30-second timeout
//! let response = handle.send_await("query", Duration::from_secs(30)).await?;
//! println!("{}", response.content);
//! ```

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::{agent_types::{AgentId, ResourceUsage}, error::RuntimeError};

/// Shared read-only view of the orchestrator's agent map, used by
/// [`IpcAgentHandle`] to query per-agent resource usage without holding a
/// reference to the full orchestrator.
pub(crate) type AgentsMap<S> = Arc<DashMap<AgentId, S>>;

// ─── Response types ───────────────────────────────────────────────────────────

/// The result returned by a completed agent run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    /// The agent's final text output.
    pub content: String,
    /// Why the agent stopped.
    pub finish_reason: FinishReason,
    /// Token usage statistics.
    pub usage: TokenUsage,
}

/// Reason why an agent loop terminated.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// Agent reached the max-turns limit.
    MaxTurns,
    /// Agent completed without any tool calls.
    NoToolCall,
    /// Token budget was exhausted.
    TokenBudget,
    /// Agent completed successfully with a natural end.
    Complete,
    /// Agent was terminated externally.
    Terminated,
}

/// Token usage statistics for a single agent run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

// ─── Internal message type ────────────────────────────────────────────────────

/// Message passed through the mpsc channel to the agent's message loop.
#[derive(Debug)]
pub(crate) enum AgentMessage {
    /// Fire-and-forget: send a message without waiting for a response.
    Send { content: String },
    /// Send and wait for the agent's response within `timeout`.
    SendAwait {
        content: String,
        timeout: Duration,
        reply_tx: tokio::sync::oneshot::Sender<Result<AgentResponse, RuntimeError>>,
    },
}

// ─── SharedSender ─────────────────────────────────────────────────────────────

/// Hot-swappable sender to an agent's IPC message loop.
///
/// Wrapped in `Arc<Mutex<Option<...>>>` so that:
/// - Multiple [`IpcAgentHandle`] clones share a single slot.
/// - The orchestrator can atomically replace the inner sender when the agent
///   loop is restarted, making the swap transparent to callers.
/// - `None` signals that the agent is not currently running (e.g. between
///   restart attempts); callers receive `AgentNotFound`.
pub(crate) type SharedSender =
    Arc<tokio::sync::Mutex<Option<tokio::sync::mpsc::Sender<AgentMessage>>>>;

// ─── IpcAgentHandle ──────────────────────────────────────────────────────────

/// A typed handle to a running agent for direct IPC communication.
///
/// Obtained from [`AgentOrchestrator::spawn_agent`](crate::orchestrator::AgentOrchestrator::spawn_agent).
/// All clones share the same [`SharedSender`] slot: when the orchestrator
/// restarts the underlying agent loop the sender is swapped in-place, so
/// existing handles automatically route future messages to the new loop.
#[derive(Debug, Clone)]
pub struct IpcAgentHandle {
    /// The unique ID of the target agent.
    pub agent_id: AgentId,
    /// Shared, hot-swappable sender to the agent's message loop.
    pub(crate) shared_tx: SharedSender,
    /// Shared view of the orchestrator's agent map, used for resource queries.
    pub(crate) agents: Arc<DashMap<AgentId, crate::orchestrator::AgentState>>,
}

impl IpcAgentHandle {
    /// Send a message to the agent without waiting for a response.
    ///
    /// Returns immediately after the message is queued.  Returns
    /// `Err(AgentNotFound)` if the agent's message loop has exited and the
    /// restart has not yet completed (slot is `None`) or if the channel is
    /// permanently closed.
    pub async fn send(&self, msg: impl Into<String>) -> Result<(), RuntimeError> {
        let guard = self.shared_tx.lock().await;
        match guard.as_ref() {
            Some(tx) => tx
                .send(AgentMessage::Send {
                    content: msg.into(),
                })
                .await
                .map_err(|_| RuntimeError::AgentNotFound(self.agent_id.0.clone())),
            None => Err(RuntimeError::AgentNotFound(self.agent_id.0.clone())),
        }
    }

    /// Send a message and wait for the agent to complete processing it.
    ///
    /// Returns the agent's [`AgentResponse`] or a [`RuntimeError::Timeout`]
    /// if neither the inner handler nor the outer guard fires in time.
    ///
    /// An extra 5-second grace window is added on top of `timeout` for the
    /// handler to actually deliver the result before the outer guard fires.
    pub async fn send_await(
        &self,
        msg: impl Into<String>,
        timeout: Duration,
    ) -> Result<AgentResponse, RuntimeError> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();

        {
            let guard = self.shared_tx.lock().await;
            match guard.as_ref() {
                Some(tx) => tx
                    .send(AgentMessage::SendAwait {
                        content: msg.into(),
                        timeout,
                        reply_tx,
                    })
                    .await
                    .map_err(|_| RuntimeError::AgentNotFound(self.agent_id.0.clone()))?,
                None => return Err(RuntimeError::AgentNotFound(self.agent_id.0.clone())),
            }
        }

        // Give the handler `timeout + 5s` to respond before the outer guard fires.
        tokio::time::timeout(timeout + Duration::from_secs(5), reply_rx)
            .await
            .map_err(|_| RuntimeError::Timeout)?
            .map_err(|_| RuntimeError::AgentNotFound(self.agent_id.0.clone()))?
    }

    /// Return the latest resource usage snapshot for this agent.
    ///
    /// Reads directly from the shared orchestrator agent map — no async IPC
    /// round-trip is needed.  Returns `None` if the agent is no longer
    /// registered or if no resource data has been collected yet.
    pub fn resource_usage(&self) -> Option<ResourceUsage> {
        self.agents
            .get(&self.agent_id)
            .and_then(|state| state.value().to_resource_usage())
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_handle(tx: tokio::sync::mpsc::Sender<AgentMessage>, id: &str) -> IpcAgentHandle {
        IpcAgentHandle {
            agent_id: AgentId::new(id),
            shared_tx: Arc::new(tokio::sync::Mutex::new(Some(tx))),
            agents: Arc::new(dashmap::DashMap::new()),
        }
    }

    #[tokio::test]
    async fn test_send_fire_and_forget() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentMessage>(8);
        let handle = make_handle(tx, "test-agent");

        handle.send("hello").await.expect("send should succeed");

        match rx.recv().await.unwrap() {
            AgentMessage::Send { content } => assert_eq!(content, "hello"),
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_send_fails_when_receiver_dropped() {
        let (tx, _rx) = tokio::sync::mpsc::channel::<AgentMessage>(8);
        // Drop _rx immediately — the channel is closed.
        drop(_rx);

        let handle = make_handle(tx, "gone-agent");

        let result = handle.send("hello").await;
        assert!(
            matches!(result, Err(RuntimeError::AgentNotFound(_))),
            "expected AgentNotFound, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_send_fails_when_slot_is_none() {
        // Simulate the between-restart window: slot is None.
        let handle = IpcAgentHandle {
            agent_id: AgentId::new("restarting-agent"),
            shared_tx: Arc::new(tokio::sync::Mutex::new(None)),
            agents: Arc::new(dashmap::DashMap::new()),
        };

        let result = handle.send("hello").await;
        assert!(
            matches!(result, Err(RuntimeError::AgentNotFound(_))),
            "expected AgentNotFound when slot is None, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_shared_sender_hot_swap() {
        // First channel (old loop)
        let (tx1, _rx1) = tokio::sync::mpsc::channel::<AgentMessage>(8);
        let shared: SharedSender = Arc::new(tokio::sync::Mutex::new(Some(tx1)));
        let handle = IpcAgentHandle {
            agent_id: AgentId::new("swap-agent"),
            shared_tx: Arc::clone(&shared),
            agents: Arc::new(dashmap::DashMap::new()),
        };

        // Drop rx1 — old loop is dead.
        drop(_rx1);
        // Send fails because old channel is broken.
        assert!(handle.send("should fail").await.is_err());

        // Orchestrator restarts: swap in a new sender.
        let (tx2, mut rx2) = tokio::sync::mpsc::channel::<AgentMessage>(8);
        *shared.lock().await = Some(tx2);

        // Same handle now routes to new loop.
        handle.send("after restart").await.expect("send after hot-swap should succeed");
        match rx2.recv().await.unwrap() {
            AgentMessage::Send { content } => assert_eq!(content, "after restart"),
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_send_await_receives_response() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentMessage>(8);
        let handle = make_handle(tx, "echo-agent");

        // Spawn a fake handler that echoes back.
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let AgentMessage::SendAwait {
                    content,
                    reply_tx,
                    ..
                } = msg
                {
                    let response = AgentResponse {
                        content: format!("echo:{content}"),
                        finish_reason: FinishReason::Complete,
                        usage: TokenUsage::default(),
                    };
                    let _ = reply_tx.send(Ok(response));
                }
            }
        });

        let resp = handle
            .send_await("ping", Duration::from_secs(5))
            .await
            .expect("send_await should succeed");

        assert_eq!(resp.content, "echo:ping");
        assert!(matches!(resp.finish_reason, FinishReason::Complete));
    }

    #[tokio::test]
    async fn test_send_await_timeout() {
        let (tx, _rx) = tokio::sync::mpsc::channel::<AgentMessage>(8);
        // Do NOT spawn a handler — reply will never arrive.

        let handle = make_handle(tx, "slow-agent");

        let result = handle
            .send_await("stuck", Duration::from_millis(50))
            .await;

        // Channel is alive but no handler replies — outer guard fires.
        // However because _rx is still in scope, the send succeeds but the
        // oneshot never fires, so we get Timeout from the outer guard.
        // If the channel happens to be dropped first we get AgentNotFound.
        assert!(
            matches!(
                result,
                Err(RuntimeError::Timeout) | Err(RuntimeError::AgentNotFound(_))
            ),
            "expected Timeout or AgentNotFound, got {:?}",
            result
        );
    }

    // ── resource_usage tests ─────────────────────────────────────────────────

    #[test]
    fn test_resource_usage_returns_none_when_agent_not_in_map() {
        let agents: Arc<DashMap<AgentId, crate::orchestrator::AgentState>> =
            Arc::new(DashMap::new());
        let (tx, _rx) = std::sync::mpsc::channel::<()>();
        let _ = tx; // suppress unused warning
        let handle = IpcAgentHandle {
            agent_id: AgentId::new("ghost"),
            shared_tx: Arc::new(tokio::sync::Mutex::new(None)),
            agents,
        };
        assert!(handle.resource_usage().is_none());
    }

    #[test]
    fn test_resource_usage_returns_none_before_health_check() {
        use crate::agent_types::AgentConfig;

        let agent_id = AgentId::new("no-health");
        let agents: Arc<DashMap<AgentId, crate::orchestrator::AgentState>> =
            Arc::new(DashMap::new());
        agents.insert(
            agent_id.clone(),
            crate::orchestrator::AgentState::new(AgentConfig::new("no-health"), None),
        );

        let handle = IpcAgentHandle {
            agent_id,
            shared_tx: Arc::new(tokio::sync::Mutex::new(None)),
            agents,
        };
        // No health check run yet → resource_usage is None.
        assert!(handle.resource_usage().is_none());
    }

    #[test]
    fn test_resource_usage_reflects_health_memory() {
        use crate::agent_types::AgentConfig;
        use crate::orchestrator::{AgentState, HealthStatus};

        let agent_id = AgentId::new("healthy");
        let agents: Arc<DashMap<AgentId, AgentState>> = Arc::new(DashMap::new());
        let mut state = AgentState::new(AgentConfig::new("healthy"), None);

        // Inject a health snapshot with 2048 KB of memory.
        let mut health = HealthStatus::new(agent_id.clone());
        health.memory_usage_kb = Some(2048);
        state.health = Some(health);
        agents.insert(agent_id.clone(), state);

        let handle = IpcAgentHandle {
            agent_id,
            shared_tx: Arc::new(tokio::sync::Mutex::new(None)),
            agents,
        };

        let usage = handle.resource_usage().expect("should have resource usage");
        assert_eq!(usage.memory_bytes, Some(2048 * 1024));
        assert_eq!(usage.cpu_ms, None);
    }
}
