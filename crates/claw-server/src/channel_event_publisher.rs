//! RuntimeChannelEventPublisher — bridges claw-channel events into the EventBus.
//!
//! This module closes the GAP-05 gap: inbound channel messages were previously
//! handled by the IPC layer in isolation, never reaching the EventBus, which
//! meant EventTriggers and AgentOrchestrator were blind to channel traffic.
//!
//! # Dependency direction
//!
//! `claw-channel` intentionally does **not** depend on `claw-runtime` (to avoid
//! circular crates).  The [`ChannelEventPublisher`] trait in `claw-channel` is
//! the injection point — callers in `claw-server` (which depends on both crates)
//! supply a concrete implementation backed by the runtime `EventBus`.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::debug;

use claw_channel::{ChannelError, ChannelEvent, ChannelEventPublisher};
use claw_runtime::EventBus;
use claw_runtime::agent_types::AgentId;
use claw_runtime::events::Event;

/// Concrete [`ChannelEventPublisher`] that forwards channel-layer events to
/// the runtime [`EventBus`].
///
/// # What it publishes
///
/// | `ChannelEvent` variant   | `Event` variant published         |
/// |--------------------------|-----------------------------------|
/// | `MessageReceived`        | `Event::MessageReceived`          |
/// | `MessageSent`            | `Event::Custom("channel.message_sent")`  |
/// | `ConnectionState`        | `Event::Custom("channel.connection_state")` |
///
/// Failures to publish (e.g. no subscribers) are silently ignored — the
/// `ChannelEventPublisher` contract says publishing must be best-effort.
pub struct RuntimeChannelEventPublisher {
    event_bus: EventBus,
}

impl RuntimeChannelEventPublisher {
    /// Create a new publisher and wrap it in an `Arc<dyn ChannelEventPublisher>`.
    pub fn new(event_bus: EventBus) -> Arc<dyn ChannelEventPublisher> {
        Arc::new(Self { event_bus })
    }
}

#[async_trait]
impl ChannelEventPublisher for RuntimeChannelEventPublisher {
    async fn publish(&self, event: ChannelEvent) -> Result<(), ChannelError> {
        match event {
            ChannelEvent::MessageReceived {
                agent_id,
                channel,
                platform,
                content_preview,
            } => {
                debug!(
                    agent_id = %agent_id,
                    channel  = %channel,
                    platform = %platform,
                    preview_len = content_preview.len(),
                    "ChannelEvent::MessageReceived → EventBus"
                );
                self.event_bus.publish(Event::MessageReceived {
                    agent_id: AgentId::new(&agent_id),
                    channel,
                    message_type: platform.to_string(),
                });
            }

            ChannelEvent::MessageSent {
                agent_id,
                channel,
                platform,
                success,
            } => {
                debug!(
                    agent_id = %agent_id,
                    channel  = %channel,
                    success  = success,
                    "ChannelEvent::MessageSent → EventBus"
                );
                self.event_bus.publish(Event::Custom {
                    event_type: "channel.message_sent".to_string(),
                    data: serde_json::json!({
                        "agent_id": agent_id,
                        "channel":  channel,
                        "platform": platform.to_string(),
                        "success":  success,
                    }),
                });
            }

            ChannelEvent::ConnectionState {
                channel,
                platform,
                connected,
            } => {
                debug!(
                    channel   = %channel,
                    connected = connected,
                    "ChannelEvent::ConnectionState → EventBus"
                );
                self.event_bus.publish(Event::Custom {
                    event_type: "channel.connection_state".to_string(),
                    data: serde_json::json!({
                        "channel":   channel,
                        "platform":  platform.to_string(),
                        "connected": connected,
                    }),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claw_channel::Platform;

    #[tokio::test]
    async fn test_message_received_publishes_to_event_bus() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let publisher = RuntimeChannelEventPublisher::new(bus.clone());
        publisher
            .publish(ChannelEvent::MessageReceived {
                agent_id: "agent-42".to_string(),
                channel: "ch-discord".to_string(),
                platform: Platform::Discord,
                content_preview: "hello world".to_string(),
            })
            .await
            .unwrap();

        let event = rx.recv().await.unwrap();
        match event {
            Event::MessageReceived { agent_id, channel, message_type } => {
                assert_eq!(agent_id.as_str(), "agent-42");
                assert_eq!(channel, "ch-discord");
                assert_eq!(message_type, "discord");
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_message_sent_publishes_custom_event() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let publisher = RuntimeChannelEventPublisher::new(bus.clone());
        publisher
            .publish(ChannelEvent::MessageSent {
                agent_id: "agent-1".to_string(),
                channel: "ch-wh".to_string(),
                platform: Platform::Webhook,
                success: true,
            })
            .await
            .unwrap();

        let event = rx.recv().await.unwrap();
        match event {
            Event::Custom { event_type, data } => {
                assert_eq!(event_type, "channel.message_sent");
                assert_eq!(data["agent_id"], "agent-1");
                assert_eq!(data["success"], true);
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_connection_state_publishes_custom_event() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let publisher = RuntimeChannelEventPublisher::new(bus.clone());
        publisher
            .publish(ChannelEvent::ConnectionState {
                channel: "ch-stdin".to_string(),
                platform: Platform::Stdin,
                connected: false,
            })
            .await
            .unwrap();

        let event = rx.recv().await.unwrap();
        match event {
            Event::Custom { event_type, data } => {
                assert_eq!(event_type, "channel.connection_state");
                assert_eq!(data["connected"], false);
                assert_eq!(data["platform"], "stdin");
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_publish_with_no_subscribers_does_not_error() {
        // EventBus with no subscribers — publish should succeed (best-effort).
        let bus = EventBus::new();
        let publisher = RuntimeChannelEventPublisher::new(bus);
        let result = publisher
            .publish(ChannelEvent::MessageReceived {
                agent_id: "a".to_string(),
                channel: "c".to_string(),
                platform: Platform::Stdin,
                content_preview: String::new(),
            })
            .await;
        assert!(result.is_ok());
    }
}
