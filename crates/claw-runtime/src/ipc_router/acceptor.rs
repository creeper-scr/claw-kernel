//! Acceptor — listens for incoming IPC connections and dispatches messages.

use super::codec;
use super::IpcConnection;
use super::IpcTransportFactory;
use crate::a2a::routing::SimpleRouter;
use crate::error::RuntimeError;
use crate::event_bus::EventBus;
use crate::events::Event;
use std::sync::Arc;

/// Start accepting incoming IPC connections in a background task.
///
/// Each accepted connection is handled in a dedicated `tokio::spawn` task
/// that reads frames, decodes them as `A2AMessage`, and routes them to
/// local agents.
pub(super) async fn start_accepting(
    endpoint: String,
    router: Arc<SimpleRouter>,
    event_bus: Arc<EventBus>,
    transport_factory: Arc<dyn IpcTransportFactory>,
) -> Result<(), RuntimeError> {
    tokio::spawn(async move {
        loop {
            // Remove any stale socket file before rebinding (Unix only).
            #[cfg(unix)]
            let _ = std::fs::remove_file(&endpoint);

            let transport = match transport_factory.create_server(&endpoint).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::error!("IpcRouter: failed to bind on {}: {}", endpoint, e);
                    break;
                }
            };

            let router_clone = Arc::clone(&router);
            let event_bus_clone = Arc::clone(&event_bus);
            tokio::spawn(async move {
                handle_transport(transport, router_clone, event_bus_clone).await;
            });
        }
    });

    Ok(())
}

/// Drive a single accepted IPC connection until it closes.
///
/// Reads length-prefixed frames, decodes each as an `A2AMessage`,
/// publishes a `MessageReceived` event, and routes the message locally.
async fn handle_transport(
    transport: Box<dyn IpcConnection>,
    router: Arc<SimpleRouter>,
    event_bus: Arc<EventBus>,
) {
    while let Ok(bytes) = transport.recv().await {
        match codec::decode_message(&bytes) {
            Ok(message) => {
                // Publish message received notification (metadata only)
                let _ = event_bus.publish(Event::MessageReceived {
                    agent_id: message.source.clone(),
                    channel: "ipc".to_string(),
                    message_type: format!("{:?}", message.message_type),
                });

                // Publish the full A2A event
                let _ = event_bus.publish(Event::A2A(message.clone()));

                // Route message to local agent
                let _ = router.route_message(message).await;
            }
            Err(e) => tracing::warn!("IpcRouter: decode error: {e}"),
        }
    }
}
