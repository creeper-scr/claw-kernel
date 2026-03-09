//! Connection and request handlers for KernelServer.
//!
//! Handles JSON-RPC 2.0 requests over IPC connections.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::error::ServerError;
use crate::protocol::{
    error_codes, ChunkParams, CreateSessionParams, DestroySessionParams, FinishParams,
    Notification, Request, Response, SendMessageParams, ToolCallParams, ToolResultParams,
};
use crate::session::{Session, SessionManager};

/// Handles a client connection.
///
/// Reads JSON-RPC requests from the stream and dispatches them to appropriate handlers.
pub async fn handle_connection(
    mut stream: UnixStream,
    session_manager: Arc<SessionManager>,
) -> Result<(), ServerError> {
    info!("New client connection established");

    let (notify_tx, mut notify_rx) = mpsc::channel::<Vec<u8>>(100);
    let mut buffer = vec![0u8; 8192];
    let mut accumulated = Vec::new();

    loop {
        tokio::select! {
            // Read from socket
            result = stream.read(&mut buffer) => {
                match result {
                    Ok(0) => {
                        debug!("Client disconnected");
                        break;
                    }
                    Ok(n) => {
                        accumulated.extend_from_slice(&buffer[..n]);

                        // Process complete messages (assumes newline-delimited JSON)
                        while let Some(pos) = accumulated.iter().position(|&b| b == b'\n') {
                            let line = accumulated.drain(..=pos).collect::<Vec<_>>();
                            let line = &line[..line.len() - 1]; // Remove newline

                            if let Err(e) = handle_message(
                                line,
                                &session_manager,
                                &notify_tx,
                                &mut stream,
                            )
                            .await
                            {
                                warn!("Failed to handle message: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Read error: {}", e);
                        break;
                    }
                }
            }

            // Send notifications to client
            Some(data) = notify_rx.recv() => {
                if let Err(e) = stream.write_all(&data).await {
                    error!("Failed to send notification: {}", e);
                    break;
                }
                if let Err(e) = stream.write_all(b"\n").await {
                    error!("Failed to send notification newline: {}", e);
                    break;
                }
            }
        }
    }

    info!("Connection handler exiting");
    Ok(())
}

/// Handles a single JSON-RPC message.
async fn handle_message(
    data: &[u8],
    session_manager: &SessionManager,
    notify_tx: &mpsc::Sender<Vec<u8>>,
    stream: &mut UnixStream,
) -> Result<(), ServerError> {
    // Parse the request
    let request: Request = match serde_json::from_slice(data) {
        Ok(req) => req,
        Err(e) => {
            let response = Response::error(
                error_codes::PARSE_ERROR,
                format!("Parse error: {}", e),
                None,
                None,
            );
            send_response(stream, response).await?;
            return Ok(());
        }
    };

    debug!(
        "Received request: method={}, id={:?}",
        request.method, request.id
    );

    // Dispatch to appropriate handler
    let result = match request.method.as_str() {
        "createSession" => handle_create_session(request.params, session_manager, notify_tx).await,
        "sendMessage" => handle_send_message(request.params, session_manager).await,
        "toolResult" => handle_tool_result(request.params, session_manager).await,
        "destroySession" => handle_destroy_session(request.params, session_manager).await,
        _ => Err(ServerError::Serialization(format!(
            "Method not found: {}",
            request.method
        ))),
    };

    // Send response (unless it's a notification)
    if let Some(id) = request.id {
        let response = match result {
            Ok(result) => Response::success(result, Some(id)),
            Err(e) => Response::error(e.error_code(), e.to_string(), None, Some(id)),
        };
        send_response(stream, response).await?;
    }

    Ok(())
}

/// Sends a JSON-RPC response to the client.
async fn send_response(stream: &mut UnixStream, response: Response) -> Result<(), ServerError> {
    let json =
        serde_json::to_vec(&response).map_err(|e| ServerError::Serialization(e.to_string()))?;
    stream
        .write_all(&json)
        .await
        .map_err(|_| ServerError::Ipc(claw_pal::error::IpcError::BrokenPipe))?;
    stream
        .write_all(b"\n")
        .await
        .map_err(|_| ServerError::Ipc(claw_pal::error::IpcError::BrokenPipe))?;
    Ok(())
}

/// Sends a notification to the client.
#[allow(dead_code)]
async fn send_notification(
    notify_tx: &mpsc::Sender<Vec<u8>>,
    method: impl Into<String>,
    params: serde_json::Value,
) -> Result<(), ServerError> {
    let notification = Notification::new(method, Some(params));
    let json =
        serde_json::to_vec(&notification).map_err(|e| ServerError::Serialization(e.to_string()))?;
    notify_tx
        .send(json)
        .await
        .map_err(|_| ServerError::Ipc(claw_pal::error::IpcError::BrokenPipe))
}

/// Handles `createSession` method.
async fn handle_create_session(
    params: Option<serde_json::Value>,
    session_manager: &SessionManager,
    notify_tx: &mpsc::Sender<Vec<u8>>,
) -> Result<serde_json::Value, ServerError> {
    debug!("Creating new session");

    // Parse params (optional)
    let _params: Option<CreateSessionParams> = params
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| ServerError::Serialization(format!("Invalid params: {}", e)))?;

    // Create session
    let session = session_manager.create(notify_tx.clone())?;

    info!("Created session: {}", session.id);

    Ok(serde_json::json!({
        "session_id": session.id,
    }))
}

/// Handles `sendMessage` method.
async fn handle_send_message(
    params: Option<serde_json::Value>,
    session_manager: &SessionManager,
) -> Result<serde_json::Value, ServerError> {
    let params: SendMessageParams = params
        .ok_or_else(|| ServerError::Serialization("Missing params".to_string()))
        .and_then(|p| {
            serde_json::from_value(p)
                .map_err(|e| ServerError::Serialization(format!("Invalid params: {}", e)))
        })?;

    debug!("Sending message to session: {}", params.session_id);

    // Get session
    let _session = session_manager
        .get(&params.session_id)
        .ok_or_else(|| ServerError::SessionNotFound(params.session_id.clone()))?;

    // TODO: Integrate with claw-loop to actually send message to agent
    // For now, just return a placeholder response
    Ok(serde_json::json!({
        "status": "accepted",
        "session_id": params.session_id,
    }))
}

/// Handles `toolResult` method.
async fn handle_tool_result(
    params: Option<serde_json::Value>,
    session_manager: &SessionManager,
) -> Result<serde_json::Value, ServerError> {
    let params: ToolResultParams = params
        .ok_or_else(|| ServerError::Serialization("Missing params".to_string()))
        .and_then(|p| {
            serde_json::from_value(p)
                .map_err(|e| ServerError::Serialization(format!("Invalid params: {}", e)))
        })?;

    debug!("Received tool result for session: {}", params.session_id);

    // Get session
    let session = session_manager
        .get(&params.session_id)
        .ok_or_else(|| ServerError::SessionNotFound(params.session_id.clone()))?;

    // Send result to session's tool result channel
    session
        .send_tool_result(params.tool_call_id, params.result, params.success)
        .await?;

    Ok(serde_json::json!({
        "status": "acknowledged",
    }))
}

/// Handles `destroySession` method.
async fn handle_destroy_session(
    params: Option<serde_json::Value>,
    session_manager: &SessionManager,
) -> Result<serde_json::Value, ServerError> {
    let params: DestroySessionParams = params
        .ok_or_else(|| ServerError::Serialization("Missing params".to_string()))
        .and_then(|p| {
            serde_json::from_value(p)
                .map_err(|e| ServerError::Serialization(format!("Invalid params: {}", e)))
        })?;

    debug!("Destroying session: {}", params.session_id);

    // Remove session
    if session_manager.remove(&params.session_id) {
        info!("Destroyed session: {}", params.session_id);
        Ok(serde_json::json!({
            "status": "destroyed",
            "session_id": params.session_id,
        }))
    } else {
        Err(ServerError::SessionNotFound(params.session_id))
    }
}

/// Sends a stream chunk notification to a session's client.
pub async fn notify_chunk(
    session: &Session,
    delta: impl Into<String>,
    done: bool,
) -> Result<(), ServerError> {
    let params = ChunkParams {
        session_id: session.id.clone(),
        delta: delta.into(),
        done,
    };
    let params_value =
        serde_json::to_value(params).map_err(|e| ServerError::Serialization(e.to_string()))?;
    let data = serde_json::to_vec(&Notification::new("agent/streamChunk", Some(params_value)))
        .map_err(|e| ServerError::Serialization(e.to_string()))?;
    session.notify(data).await
}

/// Sends a tool call notification to a session's client.
pub async fn notify_tool_call(
    session: &Session,
    tool_call_id: impl Into<String>,
    tool_name: impl Into<String>,
    arguments: serde_json::Value,
) -> Result<(), ServerError> {
    let params = ToolCallParams {
        session_id: session.id.clone(),
        tool_call_id: tool_call_id.into(),
        tool_name: tool_name.into(),
        arguments,
    };
    let params_value =
        serde_json::to_value(params).map_err(|e| ServerError::Serialization(e.to_string()))?;
    let data = serde_json::to_vec(&Notification::new("agent/toolCall", Some(params_value)))
        .map_err(|e| ServerError::Serialization(e.to_string()))?;
    session.notify(data).await
}

/// Sends a finish notification to a session's client.
pub async fn notify_finish(
    session: &Session,
    content: Option<impl Into<String>>,
    reason: impl Into<String>,
) -> Result<(), ServerError> {
    let params = FinishParams {
        session_id: session.id.clone(),
        content: content.map(|c| c.into()),
        reason: reason.into(),
    };
    let params_value =
        serde_json::to_value(params).map_err(|e| ServerError::Serialization(e.to_string()))?;
    let data = serde_json::to_vec(&Notification::new("agent/finish", Some(params_value)))
        .map_err(|e| ServerError::Serialization(e.to_string()))?;
    session.notify(data).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::RequestId;
    use tokio::io::AsyncReadExt;
    use tokio::net::UnixListener;

    #[tokio::test]
    async fn test_send_response() {
        use tokio::net::UnixStream;

        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("test.sock");

        // Create listener
        let listener = UnixListener::bind(&socket_path).unwrap();

        // Spawn server side
        let _server_path = socket_path.clone();
        let server_task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let response = Response::success(
                serde_json::json!({ "test": true }),
                Some(RequestId::Number(1)),
            );
            send_response(&mut stream, response).await.unwrap();
        });

        // Connect client
        let mut client = UnixStream::connect(&socket_path).await.unwrap();

        // Wait for server to send response
        server_task.await.unwrap();

        // Read response
        let mut buf = vec![0u8; 1024];
        let n = client.read(&mut buf).await.unwrap();
        let data = String::from_utf8(buf[..n].to_vec()).unwrap();
        assert!(data.contains("test"));
        assert!(data.contains("true"));
    }

    #[test]
    fn test_create_session_params_deserialization() {
        let json = r#"{"config": {"model": "gpt-4"}}"#;
        let params: CreateSessionParams = serde_json::from_str(json).unwrap();
        assert!(params.config.is_some());
    }

    #[test]
    fn test_send_message_params_deserialization() {
        let json = r#"{"session_id": "abc-123", "content": "Hello"}"#;
        let params: SendMessageParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.session_id, "abc-123");
        assert_eq!(params.content, "Hello");
    }

    #[test]
    fn test_tool_result_params_deserialization() {
        let json = r#"{"session_id": "abc-123", "tool_call_id": "call-1", "result": "output", "success": true}"#;
        let params: ToolResultParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.tool_call_id, "call-1");
        assert!(params.success);
    }

    #[test]
    fn test_destroy_session_params_deserialization() {
        let json = r#"{"session_id": "abc-123"}"#;
        let params: DestroySessionParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.session_id, "abc-123");
    }
}
