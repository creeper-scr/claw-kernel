//! EventBus → IPC event forwarding bridge.
//!
//! Spawns a background task per session that subscribes to the EventBus
//! and forwards matching events as JSON-RPC notifications to the client.

use crate::protocol::JsonRpcNotification;
use claw_runtime::{EventBus, EventFilter};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

/// Spawn a background task that forwards EventBus events to the IPC client.
///
/// Returns a `tokio::task::JoinHandle`. Drop the handle (or abort it) to stop
/// forwarding when the session ends.
pub fn spawn_event_forwarder(
    bus: EventBus,
    filter: EventFilter,
    writer: Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>,
    session_id: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut rx = bus.subscribe_with_filter(filter);
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let params = match serde_json::to_value(&event) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!(session_id = %session_id, "Failed to serialize event: {}", e);
                            continue;
                        }
                    };
                    let notification = JsonRpcNotification {
                        jsonrpc: "2.0".to_string(),
                        method: "events.notification".to_string(),
                        params: Some(params),
                    };
                    let payload = match serde_json::to_vec(&notification) {
                        Ok(b) => b,
                        Err(e) => {
                            tracing::warn!(session_id = %session_id, "Failed to encode notification: {}", e);
                            continue;
                        }
                    };
                    // Inline framing: 4-byte BE length prefix + payload
                    let len = payload.len() as u32;
                    let mut w = writer.lock().await;
                    if w.write_all(&len.to_be_bytes()).await.is_err() {
                        tracing::debug!(session_id = %session_id, "Event forwarder write error (client disconnected?)");
                        break;
                    }
                    if w.write_all(&payload).await.is_err() {
                        tracing::debug!(session_id = %session_id, "Event forwarder write error (client disconnected?)");
                        break;
                    }
                }
                Err(e) => {
                    tracing::debug!(session_id = %session_id, "Event forwarder recv error: {}", e);
                    break;
                }
            }
        }
    })
}
