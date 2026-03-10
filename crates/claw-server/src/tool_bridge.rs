//! External tool bridge — routes agent tool calls to the IPC client.
//!
//! When the LLM requests a tool that is registered as external, this bridge:
//! 1. Sends an `agent/toolCall` notification to the client via `notify_tx`
//! 2. Waits up to 30 seconds for a `toolResult` response via a oneshot channel
//! 3. Returns the result to the agent loop
//!
//! The bridge avoids holding `Arc<Session>` directly to prevent a circular
//! dependency (Session owns AgentLoop, AgentLoop owns ToolRegistry, ToolRegistry
//! would own ExternalToolBridge which would own Session).  Instead the bridge
//! holds only the components it actually needs:
//! - `notify_tx`: to send notifications to the client
//! - `session_id`: included in every notification payload
//! - `pending_tool_calls`: shared with the Session so that `handle_tool_result`
//!   can route the client response back to the waiting bridge

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use claw_tools::{
    traits::Tool,
    types::{PermissionSet, ToolContext, ToolError, ToolResult, ToolSchema},
};
use dashmap::DashMap;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::protocol::{Notification, ToolCallParams};

/// Bridge that routes tool calls from the agent loop to the IPC client.
///
/// One instance is created per external tool per session.
pub struct ExternalToolBridge {
    tool_name: String,
    tool_desc: String,
    schema: ToolSchema,
    permissions: PermissionSet,
    /// Session ID to include in every notification payload.
    session_id: String,
    /// Channel for sending JSON-encoded notifications to the client writer task.
    notify_tx: mpsc::Sender<Vec<u8>>,
    /// Shared map of pending (in-flight) tool calls.
    /// Key: tool_call_id, Value: oneshot::Sender to deliver the client response.
    pending_tool_calls: Arc<DashMap<String, oneshot::Sender<(serde_json::Value, bool)>>>,
}

impl ExternalToolBridge {
    /// Create a new bridge for a client-side tool.
    ///
    /// # Arguments
    ///
    /// * `tool_name` — snake_case tool name as registered in the client.
    /// * `tool_desc` — human-readable description passed to the LLM.
    /// * `input_schema` — JSON Schema object describing input parameters.
    /// * `session_id` — ID of the owning session (included in notifications).
    /// * `notify_tx` — channel used to push serialised notifications to the client.
    /// * `pending_tool_calls` — shared map; the bridge inserts a oneshot sender
    ///   here while waiting for the client response, and `handle_tool_result`
    ///   removes it when the response arrives.
    pub fn new(
        tool_name: impl Into<String>,
        tool_desc: impl Into<String>,
        input_schema: serde_json::Value,
        session_id: impl Into<String>,
        notify_tx: mpsc::Sender<Vec<u8>>,
        pending_tool_calls: Arc<DashMap<String, oneshot::Sender<(serde_json::Value, bool)>>>,
    ) -> Self {
        let name = tool_name.into();
        let desc = tool_desc.into();
        let schema = ToolSchema::new(name.clone(), desc.clone(), input_schema);
        Self {
            tool_name: name,
            tool_desc: desc,
            schema,
            permissions: PermissionSet::minimal(),
            session_id: session_id.into(),
            notify_tx,
            pending_tool_calls,
        }
    }
}

#[async_trait]
impl Tool for ExternalToolBridge {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_desc
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    fn permissions(&self) -> &PermissionSet {
        &self.permissions
    }

    fn timeout(&self) -> Duration {
        // Slightly longer than the client-side 30s timeout so the bridge's
        // own timeout fires first and can clean up the pending entry.
        Duration::from_secs(35)
    }

    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let tool_call_id = Uuid::new_v4().to_string();

        // Register a oneshot channel before sending the notification so there
        // is no race between the notification delivery and the client response.
        let (tx, rx) = oneshot::channel::<(serde_json::Value, bool)>();
        self.pending_tool_calls
            .insert(tool_call_id.clone(), tx);

        // Build and send the agent/toolCall notification.
        let params = ToolCallParams {
            session_id: self.session_id.clone(),
            tool_call_id: tool_call_id.clone(),
            tool_name: self.tool_name.clone(),
            arguments: args,
        };
        let params_value = match serde_json::to_value(params) {
            Ok(v) => v,
            Err(e) => {
                self.pending_tool_calls.remove(&tool_call_id);
                return ToolResult::err(
                    ToolError::internal(format!("failed to serialise tool call params: {e}")),
                    0,
                );
            }
        };
        let data = match serde_json::to_vec(&Notification::new("agent/toolCall", Some(params_value))) {
            Ok(d) => d,
            Err(e) => {
                self.pending_tool_calls.remove(&tool_call_id);
                return ToolResult::err(
                    ToolError::internal(format!("failed to serialise notification: {e}")),
                    0,
                );
            }
        };

        if let Err(_) = self.notify_tx.send(data).await {
            self.pending_tool_calls.remove(&tool_call_id);
            return ToolResult::err(
                ToolError::internal("notify channel closed before tool call could be sent"),
                0,
            );
        }

        // Wait for the client to call `toolResult`, with a 30-second timeout.
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok((result, true))) => ToolResult::ok(result, 0),
            Ok(Ok((result, false))) => ToolResult::err(
                ToolError::internal(result.to_string()),
                0,
            ),
            Ok(Err(_)) => {
                // The oneshot sender was dropped (session destroyed while waiting).
                ToolResult::err(
                    ToolError::internal("tool result channel closed unexpectedly"),
                    0,
                )
            }
            Err(_timeout) => {
                // Timed out — clean up the pending entry so it doesn't leak.
                self.pending_tool_calls.remove(&tool_call_id);
                ToolResult::err(ToolError::timeout(), 0)
            }
        }
    }
}
