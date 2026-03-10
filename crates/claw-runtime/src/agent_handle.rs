//! IpcAgentHandle: typed handle for communicating with a spawned agent over IPC.
//!
//! Unlike [`AgentHandle`](crate::agent_types::AgentHandle) (which only holds an
//! event bus reference), `IpcAgentHandle` carries a dedicated
//! `tokio::sync::mpsc` channel so callers can send request/response messages
//! directly to the agent's internal message loop.
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

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{agent_types::AgentId, error::RuntimeError};

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

// ─── IpcAgentHandle ──────────────────────────────────────────────────────────

/// A typed handle to a running agent for direct IPC communication.
///
/// Obtained from [`AgentOrchestrator::spawn_agent`](crate::orchestrator::AgentOrchestrator::spawn_agent).
/// All clones share the same underlying channel to the agent.
#[derive(Debug, Clone)]
pub struct IpcAgentHandle {
    /// The unique ID of the target agent.
    pub agent_id: AgentId,
    /// Sender side of the agent's message channel.
    pub(crate) tx: tokio::sync::mpsc::Sender<AgentMessage>,
}

impl IpcAgentHandle {
    /// Send a message to the agent without waiting for a response.
    ///
    /// Returns immediately after the message is queued.  Returns
    /// `Err(AgentNotFound)` if the agent's message loop has already exited.
    pub async fn send(&self, msg: impl Into<String>) -> Result<(), RuntimeError> {
        self.tx
            .send(AgentMessage::Send {
                content: msg.into(),
            })
            .await
            .map_err(|_| RuntimeError::AgentNotFound(self.agent_id.0.clone()))
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

        self.tx
            .send(AgentMessage::SendAwait {
                content: msg.into(),
                timeout,
                reply_tx,
            })
            .await
            .map_err(|_| RuntimeError::AgentNotFound(self.agent_id.0.clone()))?;

        // Give the handler `timeout + 5s` to respond before the outer guard fires.
        tokio::time::timeout(timeout + Duration::from_secs(5), reply_rx)
            .await
            .map_err(|_| RuntimeError::Timeout)?
            .map_err(|_| RuntimeError::AgentNotFound(self.agent_id.0.clone()))?
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_send_fire_and_forget() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentMessage>(8);
        let handle = IpcAgentHandle {
            agent_id: AgentId::new("test-agent"),
            tx,
        };

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

        let handle = IpcAgentHandle {
            agent_id: AgentId::new("gone-agent"),
            tx,
        };

        let result = handle.send("hello").await;
        assert!(
            matches!(result, Err(RuntimeError::AgentNotFound(_))),
            "expected AgentNotFound, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_send_await_receives_response() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentMessage>(8);
        let handle = IpcAgentHandle {
            agent_id: AgentId::new("echo-agent"),
            tx,
        };

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

        let handle = IpcAgentHandle {
            agent_id: AgentId::new("slow-agent"),
            tx,
        };

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
}
