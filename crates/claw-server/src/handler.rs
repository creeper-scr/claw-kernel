//! Connection and request handlers for KernelServer.
//!
//! Handles JSON-RPC 2.0 requests over IPC connections using
//! 4-byte big-endian length-prefixed framing.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};

use claw_loop::{FinishReason, StreamChunk};

use crate::error::ServerError;
use crate::protocol::{
    error_codes, ChunkParams, CreateSessionParams, DestroySessionParams, FinishParams,
    Notification, Request, Response, SendMessageParams, ToolCallParams, ToolResultParams,
    EventsSubscribeParams, EventsUnsubscribeParams, ScheduleCreateParams, ScheduleCancelParams,
    ScheduleListParams, ChannelCreateParams, ChannelSendParams, ChannelCloseParams,
    // B1
    ChannelRegisterParams, ChannelUnregisterParams,
    // B2
    TriggerAddCronParams, TriggerAddWebhookParams, TriggerRemoveParams,
    // B3
    AgentSpawnParams, AgentKillParams, AgentSteerParams,
    // B5
    ToolRegisterParams, ToolUnregisterParams,
    // B6
    SkillLoadDirParams, SkillGetFullParams,
};
use crate::channel_registry::ChannelRegistry;
use crate::session::{Session, SessionManager};
use claw_runtime::orchestrator::AgentOrchestrator;

/// Read one 4-byte-BE-length-prefixed frame from the stream.
/// Returns `ServerError::Ipc(IpcError::BrokenPipe)` on EOF,
/// `ServerError::Ipc(IpcError::InvalidMessage)` if frame > 16 MiB.
async fn read_frame(reader: &mut OwnedReadHalf) -> Result<Vec<u8>, ServerError> {
    let mut header = [0u8; 4];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(ServerError::Ipc(claw_pal::error::IpcError::BrokenPipe));
        }
        Err(_) => return Err(ServerError::Ipc(claw_pal::error::IpcError::BrokenPipe)),
    }
    let len = u32::from_be_bytes(header) as usize;
    const MAX_FRAME: usize = 16 * 1024 * 1024; // 16 MiB
    if len > MAX_FRAME {
        return Err(ServerError::Ipc(claw_pal::error::IpcError::InvalidMessage));
    }
    let mut buf = vec![0u8; len];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|_| ServerError::Ipc(claw_pal::error::IpcError::BrokenPipe))?;
    Ok(buf)
}

/// Write one 4-byte-BE-length-prefixed frame to the stream.
async fn write_frame(writer: &mut OwnedWriteHalf, data: &[u8]) -> Result<(), ServerError> {
    let len = data.len() as u32;
    writer
        .write_all(&len.to_be_bytes())
        .await
        .map_err(|_| ServerError::Ipc(claw_pal::error::IpcError::BrokenPipe))?;
    writer
        .write_all(data)
        .await
        .map_err(|_| ServerError::Ipc(claw_pal::error::IpcError::BrokenPipe))?;
    Ok(())
}

/// Write one 4-byte-BE-length-prefixed frame to a Mutex-wrapped writer.
async fn write_frame_locked(
    writer: &Arc<Mutex<OwnedWriteHalf>>,
    data: &[u8],
) -> Result<(), ServerError> {
    let mut w = writer.lock().await;
    write_frame(&mut *w, data).await
}

/// Handles a client connection.
///
/// Reads JSON-RPC requests from the stream (4-byte BE length-prefixed frames)
/// and dispatches them to appropriate handlers.
pub async fn handle_connection(
    stream: UnixStream,
    session_manager: Arc<SessionManager>,
    provider: Arc<dyn claw_provider::traits::LLMProvider>,
    registry: Arc<crate::server::ProviderRegistry>,
    event_bus: claw_runtime::EventBus,
    channel_registry: Arc<ChannelRegistry>,
    orchestrator: Arc<AgentOrchestrator>,
    auth_token: Arc<String>,
    tool_registry: Arc<crate::global_tool_registry::GlobalToolRegistry>,
    skill_registry: Arc<crate::global_skill_registry::GlobalSkillRegistry>,
) -> Result<(), ServerError> {
    info!("New client connection established");

    // ─── Connection-level auth state ──────────────────────────────────────────
    // The first frame on any connection MUST be a `kernel.auth` handshake.
    // After a successful handshake this flag is set to true.
    let mut authenticated = false;

    let (notify_tx, mut notify_rx) = mpsc::channel::<Vec<u8>>(100);
    let (mut reader, writer_raw) = stream.into_split();
    let writer = Arc::new(Mutex::new(writer_raw));
    let scheduler = Arc::new(claw_runtime::TokioScheduler::new());

    loop {
        tokio::select! {
            frame_result = read_frame(&mut reader) => {
                match frame_result {
                    Ok(data) => {
                        if let Err(e) = handle_message(
                            &data,
                            Arc::clone(&session_manager),
                            &provider,
                            &registry,
                            &notify_tx,
                            &writer,
                            &event_bus,
                            &scheduler,
                            &channel_registry,
                            &orchestrator,
                            &mut authenticated,
                            &auth_token,
                            &tool_registry,
                            &skill_registry,
                        )
                        .await
                        {
                            warn!("Failed to handle message: {}", e);
                        }
                    }
                    Err(ServerError::Ipc(claw_pal::error::IpcError::BrokenPipe)) => {
                        debug!("Client disconnected");
                        break;
                    }
                    Err(e) => {
                        error!("Read error: {}", e);
                        break;
                    }
                }
            }

            Some(data) = notify_rx.recv() => {
                if let Err(e) = write_frame_locked(&writer, &data).await {
                    error!("Failed to send notification: {}", e);
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
    session_manager: Arc<SessionManager>,
    provider: &Arc<dyn claw_provider::traits::LLMProvider>,
    registry: &Arc<crate::server::ProviderRegistry>,
    notify_tx: &mpsc::Sender<Vec<u8>>,
    writer: &Arc<Mutex<OwnedWriteHalf>>,
    event_bus: &claw_runtime::EventBus,
    scheduler: &Arc<claw_runtime::TokioScheduler>,
    channel_registry: &Arc<ChannelRegistry>,
    orchestrator: &Arc<AgentOrchestrator>,
    authenticated: &mut bool,
    auth_token: &str,
    tool_registry: &Arc<crate::global_tool_registry::GlobalToolRegistry>,
    skill_registry: &Arc<crate::global_skill_registry::GlobalSkillRegistry>,
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
            let json = serde_json::to_vec(&response)
                .map_err(|e| ServerError::Serialization(e.to_string()))?;
            write_frame_locked(writer, &json).await?;
            return Ok(());
        }
    };

    debug!(
        "Received request: method={}, id={:?}",
        request.method, request.id
    );

    // ─── Auth gate ────────────────────────────────────────────────────────────
    if request.method == "kernel.auth" {
        let token = request
            .params
            .as_ref()
            .and_then(|p| p.get("token"))
            .and_then(|t| t.as_str())
            .unwrap_or("");
        let result: Result<serde_json::Value, ServerError> = if token == auth_token {
            *authenticated = true;
            Ok(serde_json::json!({ "ok": true }))
        } else {
            Err(ServerError::Serialization(
                "kernel.auth: invalid token".to_string(),
            ))
        };
        if let Some(id) = request.id {
            let response = match result {
                Ok(r) => Response::success(r, Some(id)),
                Err(e) => Response::error(e.error_code(), e.to_string(), None, Some(id)),
            };
            let json = serde_json::to_vec(&response)
                .map_err(|e| ServerError::Serialization(e.to_string()))?;
            write_frame_locked(writer, &json).await?;
        }
        return Ok(());
    }

    if !*authenticated {
        let result: Result<serde_json::Value, ServerError> = Err(ServerError::Serialization(
            "not authenticated: send kernel.auth first".to_string(),
        ));
        if let Some(id) = request.id {
            let response = match result {
                Ok(r) => Response::success(r, Some(id)),
                Err(e) => Response::error(e.error_code(), e.to_string(), None, Some(id)),
            };
            let json = serde_json::to_vec(&response)
                .map_err(|e| ServerError::Serialization(e.to_string()))?;
            write_frame_locked(writer, &json).await?;
        }
        return Ok(());
    }

    // Dispatch to appropriate handler
    let result = match request.method.as_str() {
        "createSession" => {
            handle_create_session(request.params, &session_manager, provider, registry, notify_tx).await
        }
        "sendMessage" => handle_send_message(request.params, &session_manager).await,
        "toolResult" => handle_tool_result(request.params, &session_manager).await,
        "destroySession" => handle_destroy_session(request.params, &session_manager).await,
        "kernel.ping" => {
            Ok(serde_json::json!({
                "pong": true,
                "ts": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64
            }))
        }
        "kernel.info" => handle_kernel_info(&session_manager, registry).await,
        "provider.register" => handle_provider_register(request.params, registry).await,
        "provider.list" => handle_provider_list(registry).await,
        // Session aliases
        "session.create" => {
            handle_create_session(request.params.map(|p| {
                // session.create accepts { system_prompt, ... } directly as config
                serde_json::json!({ "config": p })
            }), &session_manager, provider, registry, notify_tx).await
        }
        "session.close" => {
            let params = request.params.unwrap_or(serde_json::Value::Null);
            let destroy: DestroySessionParams = serde_json::from_value(params)
                .map_err(|e| ServerError::Serialization(format!("Invalid params: {}", e)))?;
            if session_manager.remove(&destroy.session_id) {
                Ok(serde_json::json!({"closed": true, "session_id": destroy.session_id}))
            } else {
                Err(ServerError::SessionNotFound(destroy.session_id))
            }
        }
        // EventBus subscription
        "events.subscribe" => {
            let params: EventsSubscribeParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_events_subscribe(params, &session_manager, event_bus.clone(), writer.clone()).await
        }
        "events.unsubscribe" => {
            let params: EventsUnsubscribeParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_events_unsubscribe(params, &session_manager).await
        }
        // Scheduler
        "schedule.create" => {
            let params: ScheduleCreateParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_schedule_create(params, Arc::clone(&session_manager), scheduler.clone()).await
        }
        "schedule.cancel" => {
            let params: ScheduleCancelParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_schedule_cancel(params, scheduler.clone()).await
        }
        "schedule.list" => {
            let params: ScheduleListParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_schedule_list(params, &session_manager, scheduler.clone()).await
        }
        // Channel management
        "channel.create" => {
            let params: ChannelCreateParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_channel_create(params).await
        }
        "channel.send" => {
            let _params: ChannelSendParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            Err(ServerError::Serialization("channel.send not yet implemented".to_string()))
        }
        "channel.close" => {
            let _params: ChannelCloseParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            Err(ServerError::Serialization("channel.close not yet implemented".to_string()))
        }
        // ─── B1: Channel register/unregister/list ─────────────────────────────
        "channel.register" => {
            let params: ChannelRegisterParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_channel_register(params, channel_registry).await
        }
        "channel.unregister" => {
            let params: ChannelUnregisterParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_channel_unregister(params, channel_registry).await
        }
        "channel.list" => handle_channel_list(channel_registry).await,
        // ─── B2: Trigger methods ──────────────────────────────────────────────
        "trigger.add_cron" => {
            let params: TriggerAddCronParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_trigger_add_cron(params, scheduler, orchestrator).await
        }
        "trigger.add_webhook" => {
            let params: TriggerAddWebhookParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_trigger_add_webhook(params, scheduler, channel_registry).await
        }
        "trigger.remove" => {
            let params: TriggerRemoveParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_trigger_remove(params, scheduler).await
        }
        "trigger.list" => handle_trigger_list(scheduler).await,
        // ─── B3: Agent lifecycle ──────────────────────────────────────────────
        "agent.spawn" => {
            let params: AgentSpawnParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_agent_spawn(params, orchestrator).await
        }
        "agent.kill" => {
            let params: AgentKillParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_agent_kill(params, orchestrator).await
        }
        "agent.steer" => {
            let params: AgentSteerParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_agent_steer(params, orchestrator).await
        }
        "agent.list" => handle_agent_list(orchestrator).await,
        // REMOVED in v1.3.0: memory.search / memory.store.
        // Memory is application-layer; see docs/kernel-gap-analysis.md § D1.
        // ─── B5: Tool registration ────────────────────────────────────────────
        "tool.register" => {
            let params: ToolRegisterParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_tool_register(params, tool_registry, notify_tx).await
        }
        "tool.unregister" => {
            let params: ToolUnregisterParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_tool_unregister(params, tool_registry).await
        }
        "tool.list" => handle_tool_list(tool_registry).await,
        // ─── B6: Skill management ─────────────────────────────────────────────
        "skill.load_dir" => {
            let params: SkillLoadDirParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_skill_load_dir(params, skill_registry).await
        }
        "skill.list" => handle_skill_list(skill_registry).await,
        "skill.get_full" => {
            let params: SkillGetFullParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_skill_get_full(params, skill_registry).await
        }
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
        let json =
            serde_json::to_vec(&response).map_err(|e| ServerError::Serialization(e.to_string()))?;
        write_frame_locked(writer, &json).await?;
    }

    Ok(())
}

/// Sends a JSON-RPC response to the client using length-prefixed framing.
#[allow(dead_code)]
async fn send_response_direct(writer: &mut OwnedWriteHalf, response: Response) -> Result<(), ServerError> {
    let json =
        serde_json::to_vec(&response).map_err(|e| ServerError::Serialization(e.to_string()))?;
    write_frame(writer, &json).await
}

/// Sends a notification to the client via the notify channel.
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
///
/// Parses the session config, builds an AgentLoop (with optional external tool bridges),
/// and registers the new session with the SessionManager.
async fn handle_create_session(
    params: Option<serde_json::Value>,
    session_manager: &SessionManager,
    provider: &Arc<dyn claw_provider::traits::LLMProvider>,
    registry: &Arc<crate::server::ProviderRegistry>,
    notify_tx: &mpsc::Sender<Vec<u8>>,
) -> Result<serde_json::Value, ServerError> {
    debug!("Creating new session");

    // Parse params — the outer object is CreateSessionParams, the inner is SessionConfig.
    let create_params: Option<CreateSessionParams> = params
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| ServerError::Serialization(format!("Invalid params: {}", e)))?;

    let session_config = create_params.and_then(|p| p.config);

    // Pre-allocate a session ID and the shared pending_tool_calls map.
    // This lets ExternalToolBridge instances be constructed before the Session
    // (breaking the circular dependency: Session → AgentLoop → ToolRegistry → Bridge → Session).
    let session_id = uuid::Uuid::new_v4().to_string();
    let pending_tool_calls: Arc<
        dashmap::DashMap<String, tokio::sync::oneshot::Sender<(serde_json::Value, bool)>>,
    > = Arc::new(dashmap::DashMap::new());

    // Build optional ToolRegistry from declared external tools.
    let tool_registry = if let Some(tool_defs) = session_config
        .as_ref()
        .and_then(|cfg| cfg.tools.as_ref())
        .filter(|t| !t.is_empty())
    {
        let tool_reg = Arc::new(claw_tools::registry::ToolRegistry::new());
        for tool_def in tool_defs {
            let bridge = crate::tool_bridge::ExternalToolBridge::new(
                tool_def.name.clone(),
                tool_def.description.clone(),
                tool_def.input_schema.clone(),
                session_id.clone(),
                notify_tx.clone(),
                Arc::clone(&pending_tool_calls),
            );
            tool_reg
                .register(Box::new(bridge))
                .map_err(|e| ServerError::Agent(format!("Failed to register tool '{}': {}", tool_def.name, e)))?;
        }
        Some(tool_reg)
    } else {
        None
    };

    // Determine the provider for this session (supports per-session provider override).
    let session_provider: Arc<dyn claw_provider::traits::LLMProvider> = match session_config
        .as_ref()
        .and_then(|cfg| cfg.provider_override.as_deref())
    {
        Some(name) => registry
            .get(name)
            .ok_or_else(|| ServerError::Agent(format!("Provider '{}' not found in registry", name)))?,
        None => Arc::clone(provider),
    };

    // Build the AgentLoop.
    let mut builder = claw_loop::AgentLoopBuilder::new()
        .with_provider(session_provider)
        .with_agent_id(session_id.clone());

    if let Some(ref cfg) = session_config {
        if let Some(ref sys) = cfg.system_prompt {
            builder = builder.with_system_prompt(sys.clone());
        }
        if let Some(max_turns) = cfg.max_turns {
            builder = builder.with_max_turns(max_turns);
        }
        if cfg.persist_history {
            if let Ok(data_dir) = claw_pal::dirs::KernelDirs::data_dir() {
                let db_path = data_dir.join("history").join(format!("{}.db", session_id));
                builder = builder.with_sqlite_history(db_path, &session_id);
            } else {
                tracing::warn!("Failed to get data_dir for SQLite history; falling back to in-memory");
            }
        }
    }

    if let Some(reg) = tool_registry {
        builder = builder.with_tools(reg);
    }

    let agent_loop = builder
        .build()
        .map_err(|e| ServerError::Agent(e.to_string()))?;

    // Create the session with all pre-built components.
    let session = session_manager.create_with_id(
        session_id.clone(),
        notify_tx.clone(),
        agent_loop,
        pending_tool_calls,
    )?;

    info!("Created session: {}", session.id);

    Ok(serde_json::json!({
        "session_id": session.id,
    }))
}

/// Handles `sendMessage` method.
///
/// Locks the session's agent loop and runs it in a spawned task so that the
/// JSON-RPC response ("accepted") is returned immediately, and the agent's
/// progress is delivered via `agent/finish` notifications.
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
    let session = session_manager
        .get(&params.session_id)
        .ok_or_else(|| ServerError::SessionNotFound(params.session_id.clone()))?;

    let content = params.content.clone();
    let session_id = params.session_id.clone();
    let session_clone = Arc::clone(&session);

    // Spawn async task to run agent loop (non-blocking so we can return "accepted" immediately).
    tokio::spawn(async move {
        let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::channel::<StreamChunk>(256);

        // Spawn a task to forward StreamChunks to the IPC client as notifications
        let session_for_chunks = Arc::clone(&session_clone);
        tokio::spawn(async move {
            while let Some(chunk) = chunk_rx.recv().await {
                match chunk {
                    StreamChunk::Text { content, is_final } => {
                        if !content.is_empty() || is_final {
                            let _ = notify_chunk(&session_for_chunks, content, is_final).await;
                        }
                    }
                    // Tool, usage, finish, error chunks are handled by the outer task
                    _ => {}
                }
            }
        });

        let mut loop_guard = session_clone.agent_loop.lock().await;
        match loop_guard.run_streaming(content, chunk_tx).await {
            Ok(result) => {
                let reason = match result.finish_reason {
                    FinishReason::Stop => "stop",
                    FinishReason::MaxTurns => "max_turns",
                    FinishReason::TokenBudget => "token_budget",
                    FinishReason::NoToolCall => "no_tool_call",
                    FinishReason::StopCondition => "stop_condition",
                    FinishReason::Error => "error",
                    // FinishReason is #[non_exhaustive]
                    _ => "unknown",
                };
                let _ = notify_finish(&session_clone, Some(result.content), reason).await;
            }
            Err(e) => {
                tracing::error!("Agent loop error for session {}: {}", session_id, e);
                let _ =
                    notify_finish(&session_clone, None::<String>, format!("error: {e}")).await;
            }
        }
    });

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

    // Send result to the pending tool call oneshot channel if present.
    if let Some((_, sender)) = session.pending_tool_calls.remove(&params.tool_call_id) {
        // Ignore error if the receiver has already dropped.
        let _ = sender.send((params.result, params.success));
    }

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

/// Handles `kernel.info` method.
async fn handle_kernel_info(
    session_manager: &SessionManager,
    registry: &Arc<crate::server::ProviderRegistry>,
) -> Result<serde_json::Value, ServerError> {
    let active_provider = registry.default_name().to_string();
    let active_model = registry.default_provider().model_id().to_string();
    let current_sessions = session_manager.count();
    let max_sessions = session_manager.max_sessions();

    // Collect compiled-in provider names
    let mut providers = vec![
        "anthropic", "openai", "ollama", "deepseek", "moonshot",
    ];
    #[cfg(feature = "gemini")]
    providers.push("gemini");
    #[cfg(feature = "mistral")]
    providers.push("mistral");
    #[cfg(feature = "azure-openai")]
    providers.push("azure-openai");
    let providers: Vec<String> = providers.into_iter().map(String::from).collect();

    let features: Vec<String> = vec![
        "streaming".to_string(),
        "external_tools".to_string(),
    ];

    let result = crate::protocol::KernelInfoResult {
        version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: 2,
        providers,
        active_provider,
        active_model,
        features,
        max_sessions,
        current_sessions,
    };

    serde_json::to_value(result).map_err(|e| ServerError::Serialization(e.to_string()))
}

/// Handles `provider.register` method — registers a new provider at runtime.
async fn handle_provider_register(
    params: Option<serde_json::Value>,
    registry: &Arc<crate::server::ProviderRegistry>,
) -> Result<serde_json::Value, ServerError> {
    use crate::protocol::ProviderRegisterParams;

    let params: ProviderRegisterParams = params
        .ok_or_else(|| ServerError::Serialization("Missing params".to_string()))
        .and_then(|p| {
            serde_json::from_value(p)
                .map_err(|e| ServerError::Serialization(format!("Invalid params: {}", e)))
        })?;

    let provider: std::sync::Arc<dyn claw_provider::traits::LLMProvider> = match params.provider_type.as_str() {
        #[cfg(feature = "anthropic")]
        "anthropic" => {
            let api_key = params.api_key.unwrap_or_default();
            let model = params.model.unwrap_or_else(|| "claude-sonnet-4-6".to_string());
            std::sync::Arc::new(claw_provider::AnthropicProvider::new(api_key, model))
        }
        #[cfg(feature = "openai")]
        "openai" => {
            let api_key = params.api_key.unwrap_or_default();
            let model = params.model.unwrap_or_else(|| "gpt-4o".to_string());
            let mut p = claw_provider::OpenAIProvider::new(api_key, model);
            if let Some(base_url) = params.base_url {
                p = p.with_base_url(base_url);
            }
            std::sync::Arc::new(p)
        }
        #[cfg(feature = "ollama")]
        "ollama" => {
            let model = params.model.unwrap_or_else(|| "llama3.2:latest".to_string());
            let base_url = params.base_url.unwrap_or_else(|| "http://localhost:11434".to_string());
            std::sync::Arc::new(claw_provider::OllamaProvider::new(model).with_base_url(base_url))
        }
        #[cfg(feature = "gemini")]
        "gemini" => {
            let api_key = params.api_key.unwrap_or_default();
            let model = params.model.unwrap_or_else(|| "gemini-2.0-flash".to_string());
            std::sync::Arc::new(claw_provider::gemini::gemini_provider(api_key, model))
        }
        #[cfg(feature = "mistral")]
        "mistral" => {
            let api_key = params.api_key.unwrap_or_default();
            let model = params.model.unwrap_or_else(|| "mistral-large-latest".to_string());
            std::sync::Arc::new(claw_provider::mistral::mistral_provider(api_key, model))
        }
        other => {
            return Err(ServerError::Serialization(format!("Unknown provider type: {}", other)));
        }
    };

    registry.register(params.name.clone(), provider);
    tracing::info!("Registered provider: {}", params.name);

    Ok(serde_json::json!({
        "status": "registered",
        "name": params.name,
    }))
}

/// Handles `provider.list` method — lists all registered provider names.
async fn handle_provider_list(
    registry: &Arc<crate::server::ProviderRegistry>,
) -> Result<serde_json::Value, ServerError> {
    let names = registry.names();
    let default = registry.default_name().to_string();
    Ok(serde_json::json!({
        "providers": names,
        "default": default,
    }))
}

/// Handles `events.subscribe` method.
async fn handle_events_subscribe(
    params: EventsSubscribeParams,
    session_manager: &SessionManager,
    event_bus: claw_runtime::EventBus,
    writer: Arc<Mutex<OwnedWriteHalf>>,
) -> Result<serde_json::Value, ServerError> {
    use claw_runtime::EventFilter;

    let filter = match params.filter.as_str() {
        "agent_lifecycle" => EventFilter::AgentLifecycle,
        "tool_calls" => EventFilter::ToolCalls,
        "llm_requests" => EventFilter::LlmRequests,
        "memory" => EventFilter::MemoryEvents,
        "a2a" => EventFilter::A2A,
        "shutdown" => EventFilter::ShutdownOnly,
        _ => EventFilter::All,
    };
    let handle = crate::event_publisher::spawn_event_forwarder(
        event_bus,
        filter,
        writer,
        params.session_id.clone(),
    );
    let session = session_manager
        .get(&params.session_id)
        .ok_or_else(|| {
            handle.abort();
            ServerError::SessionNotFound(params.session_id.clone())
        })?;
    // Abort any existing forwarder
    let mut forwarder = session.event_forwarder.lock().await;
    if let Some(old) = forwarder.take() {
        old.abort();
    }
    *forwarder = Some(handle);
    Ok(serde_json::json!({"subscribed": true, "filter": params.filter}))
}

/// Handles `events.unsubscribe` method.
async fn handle_events_unsubscribe(
    params: EventsUnsubscribeParams,
    session_manager: &SessionManager,
) -> Result<serde_json::Value, ServerError> {
    let session = session_manager
        .get(&params.session_id)
        .ok_or_else(|| ServerError::SessionNotFound(params.session_id.clone()))?;
    let mut forwarder = session.event_forwarder.lock().await;
    if let Some(handle) = forwarder.take() {
        handle.abort();
    }
    Ok(serde_json::json!({"unsubscribed": true}))
}

/// Handles `schedule.create` method.
async fn handle_schedule_create(
    params: ScheduleCreateParams,
    session_manager: Arc<SessionManager>,
    scheduler: Arc<claw_runtime::TokioScheduler>,
) -> Result<serde_json::Value, ServerError> {
    use claw_runtime::{Scheduler, TaskConfig, TaskTrigger};

    let session = session_manager
        .get(&params.session_id)
        .ok_or_else(|| ServerError::SessionNotFound(params.session_id.clone()))?;

    let task_id = uuid::Uuid::new_v4().to_string();
    let label = params.label.clone();
    let cron_str = params.cron.clone();

    let trigger = if cron_str == "once" {
        TaskTrigger::Immediate
    } else {
        TaskTrigger::Cron(cron_str.clone())
    };

    let prompt_clone = params.prompt.clone();
    let session_id_clone = params.session_id.clone();
    let session_manager_clone = Arc::clone(&session_manager);
    let config = TaskConfig::new(
        task_id.clone(),
        trigger,
        move || {
            let sm = Arc::clone(&session_manager_clone);
            let sid = session_id_clone.clone();
            let prompt = prompt_clone.clone();
            Box::pin(async move {
                if let Some(session) = sm.get(&sid) {
                    let (chunk_tx, _chunk_rx) = tokio::sync::mpsc::channel::<claw_loop::StreamChunk>(64);
                    let mut loop_guard = session.agent_loop.lock().await;
                    match loop_guard.run_streaming(prompt.clone(), chunk_tx).await {
                        Ok(_result) => tracing::debug!("Scheduled task completed for session {}", sid),
                        Err(e) => tracing::error!("Scheduled task failed for session {}: {}", sid, e),
                    }
                } else {
                    tracing::warn!("Scheduled task fired but session {} not found", sid);
                }
            })
        },
    );

    scheduler
        .schedule(config)
        .await
        .map_err(|e| ServerError::Agent(format!("Failed to schedule task: {}", e)))?;

    // Track the task ID in the session
    session.scheduled_task_ids.lock().await.push(task_id.clone());

    Ok(serde_json::json!({
        "task_id": task_id,
        "cron": cron_str,
        "label": label,
        "status": "scheduled",
    }))
}

/// Handles `schedule.cancel` method.
async fn handle_schedule_cancel(
    params: ScheduleCancelParams,
    scheduler: Arc<claw_runtime::TokioScheduler>,
) -> Result<serde_json::Value, ServerError> {
    use claw_runtime::{Scheduler, TaskId};

    scheduler
        .cancel(&TaskId::new(params.task_id.clone()))
        .await
        .map_err(|e| ServerError::Agent(format!("Failed to cancel task: {}", e)))?;

    Ok(serde_json::json!({"cancelled": true, "task_id": params.task_id}))
}

/// Handles `schedule.list` method.
async fn handle_schedule_list(
    params: ScheduleListParams,
    session_manager: &SessionManager,
    scheduler: Arc<claw_runtime::TokioScheduler>,
) -> Result<serde_json::Value, ServerError> {
    use claw_runtime::Scheduler;

    let session = session_manager
        .get(&params.session_id)
        .ok_or_else(|| ServerError::SessionNotFound(params.session_id.clone()))?;

    let session_task_ids = session.scheduled_task_ids.lock().await.clone();
    let all_tasks = scheduler.list_tasks().await;
    let all_task_strs: std::collections::HashSet<String> =
        all_tasks.iter().map(|t| t.0.clone()).collect();

    let tasks: Vec<serde_json::Value> = session_task_ids
        .iter()
        .filter(|id| all_task_strs.contains(*id))
        .map(|id| {
            serde_json::json!({
                "task_id": id,
                "status": "scheduled",
            })
        })
        .collect();

    Ok(serde_json::json!({"tasks": tasks}))
}

/// Handles `channel.create` method.
async fn handle_channel_create(
    params: ChannelCreateParams,
) -> Result<serde_json::Value, ServerError> {
    #[cfg(feature = "websocket")]
    if params.channel_type == "websocket" {
        use claw_channels::WebSocketChannel;
        use claw_channel::Channel;
        let channel_id = uuid::Uuid::new_v4().to_string();
        let ws = Arc::new(WebSocketChannel::new(
            claw_channel::ChannelId::new(&channel_id),
        ));
        ws.connect().await.map_err(|e| ServerError::Agent(format!("Failed to start WebSocket: {}", e)))?;
        let port = params.port.unwrap_or(9001);
        return Ok(serde_json::json!({
            "channel_id": channel_id,
            "channel_type": "websocket",
            "port": port,
        }));
    }
    Err(ServerError::Serialization(format!(
        "unsupported channel type: {}",
        params.channel_type
    )))
}

// ─── B1: Channel register / unregister / list ─────────────────────────────────

/// Handles `channel.register` method (B1).
///
/// Registers an external channel into the kernel ChannelRegistry.
async fn handle_channel_register(
    params: ChannelRegisterParams,
    channel_registry: &Arc<ChannelRegistry>,
) -> Result<serde_json::Value, ServerError> {
    channel_registry
        .register(params.r#type.clone(), params.channel_id.clone(), params.config)
        .map_err(|e| ServerError::Serialization(e))?;
    info!("Channel registered: id={} type={}", params.channel_id, params.r#type);
    Ok(serde_json::json!({
        "ok": true,
        "channel_id": params.channel_id,
        "type": params.r#type,
    }))
}

/// Handles `channel.unregister` method (B1).
async fn handle_channel_unregister(
    params: ChannelUnregisterParams,
    channel_registry: &Arc<ChannelRegistry>,
) -> Result<serde_json::Value, ServerError> {
    let removed = channel_registry.unregister(&params.channel_id);
    Ok(serde_json::json!({ "ok": removed, "channel_id": params.channel_id }))
}

/// Handles `channel.list` method (B1).
async fn handle_channel_list(
    channel_registry: &Arc<ChannelRegistry>,
) -> Result<serde_json::Value, ServerError> {
    let channels: Vec<serde_json::Value> = channel_registry
        .list()
        .into_iter()
        .map(|c| serde_json::json!({
            "channel_id": c.channel_id,
            "type": c.channel_type,
            "config": c.config,
        }))
        .collect();
    Ok(serde_json::json!({ "channels": channels }))
}

// ─── B2: Trigger add_cron / add_webhook / remove / list ───────────────────────

/// Handles `trigger.add_cron` method (B2).
async fn handle_trigger_add_cron(
    params: TriggerAddCronParams,
    scheduler: &Arc<claw_runtime::TokioScheduler>,
    orchestrator: &Arc<AgentOrchestrator>,
) -> Result<serde_json::Value, ServerError> {
    use claw_runtime::{Scheduler, TaskConfig, TaskTrigger};

    let trigger_id = params.trigger_id.clone();
    let cron_expr = params.cron_expr.clone();
    let target_agent = params.target_agent.clone();
    let message = params.message.clone();

    let orch = Arc::clone(orchestrator);
    let trigger = TaskTrigger::Cron(cron_expr.clone());
    let config = TaskConfig::new(
        trigger_id.clone(),
        trigger,
        move || {
            let orch = Arc::clone(&orch);
            let _agent = target_agent.clone();
            let _msg = message.clone();
            Box::pin(async move {
                use claw_runtime::agent_types::AgentId;
                use claw_runtime::orchestrator::SteerCommand;
                let agent_id = AgentId::new(_agent.clone());
                if let Err(e) = orch.steer(&agent_id, SteerCommand::Custom {
                    command: "inject".to_string(),
                    payload: Some(_msg.unwrap_or_default()),
                }).await {
                    tracing::warn!("trigger cron: steer failed for agent={}: {}", _agent, e);
                } else {
                    tracing::debug!("trigger cron fired: agent={}", _agent);
                }
            })
        },
    );

    scheduler
        .schedule(config)
        .await
        .map_err(|e| ServerError::Agent(format!("Failed to schedule cron trigger: {}", e)))?;

    Ok(serde_json::json!({
        "trigger_id": trigger_id,
        "cron_expr": cron_expr,
        "target_agent": params.target_agent,
        "status": "scheduled",
    }))
}

/// Handles `trigger.add_webhook` method (B2).
async fn handle_trigger_add_webhook(
    params: TriggerAddWebhookParams,
    _scheduler: &Arc<claw_runtime::TokioScheduler>,
    channel_registry: &Arc<ChannelRegistry>,
) -> Result<serde_json::Value, ServerError> {
    let endpoint = format!("/hooks/{}", params.trigger_id);
    // Store the webhook trigger mapping in the channel registry so it can be discovered.
    let config = serde_json::json!({
        "endpoint": endpoint,
        "target_agent": params.target_agent,
        "hmac_secret": params.hmac_secret,
    });
    channel_registry.register(
        "webhook_trigger".to_string(),
        format!("trigger:{}", params.trigger_id),
        config,
    ).unwrap_or_else(|e| tracing::warn!("trigger.add_webhook: registry conflict: {}", e));

    tracing::info!(
        "trigger.add_webhook: id={}, agent={}, endpoint={}",
        params.trigger_id,
        params.target_agent,
        endpoint,
    );
    Ok(serde_json::json!({
        "trigger_id": params.trigger_id,
        "endpoint": endpoint,
        "target_agent": params.target_agent,
        "status": "registered",
    }))
}

/// Handles `trigger.remove` method (B2).
async fn handle_trigger_remove(
    params: TriggerRemoveParams,
    scheduler: &Arc<claw_runtime::TokioScheduler>,
) -> Result<serde_json::Value, ServerError> {
    use claw_runtime::{Scheduler, TaskId};

    scheduler
        .cancel(&TaskId::new(params.trigger_id.clone()))
        .await
        .map_err(|e| ServerError::Agent(format!("Failed to remove trigger: {}", e)))?;

    Ok(serde_json::json!({
        "trigger_id": params.trigger_id,
        "status": "removed",
    }))
}

/// Handles `trigger.list` method (B2).
async fn handle_trigger_list(
    scheduler: &Arc<claw_runtime::TokioScheduler>,
) -> Result<serde_json::Value, ServerError> {
    use claw_runtime::Scheduler;

    let tasks = scheduler.list_tasks().await;
    let list: Vec<serde_json::Value> = tasks
        .iter()
        .map(|t| serde_json::json!({ "trigger_id": t.0, "status": "scheduled" }))
        .collect();
    Ok(serde_json::json!(list))
}

// ─── B3: Agent spawn / kill / steer / list ────────────────────────────────────

/// Handles `agent.spawn` method (B3).
///
/// Registers a new in-process agent with the AgentOrchestrator.
async fn handle_agent_spawn(
    params: AgentSpawnParams,
    orchestrator: &Arc<AgentOrchestrator>,
) -> Result<serde_json::Value, ServerError> {
    use claw_runtime::agent_types::{AgentConfig, AgentId};

    let agent_id = params
        .agent_id
        .map(|s| AgentId::new(s))
        .unwrap_or_else(AgentId::generate);

    let mut config = AgentConfig::new(agent_id.as_str());
    config.agent_id = agent_id.clone();

    // Store spawn parameters in metadata for reference.
    if let Some(ref sys) = params.config.system_prompt {
        config = config.with_meta("system_prompt", sys.as_str());
    }
    if let Some(ref provider) = params.config.provider {
        config = config.with_meta("provider", provider.as_str());
    }
    if let Some(ref model) = params.config.model {
        config = config.with_meta("model", model.as_str());
    }
    if let Some(max_turns) = params.config.max_turns {
        config = config.with_meta("max_turns", max_turns.to_string());
    }

    let handle = orchestrator
        .register(config)
        .map_err(|e| ServerError::Agent(format!("agent.spawn failed: {}", e)))?;

    info!("Agent spawned: {}", handle.agent_id);
    Ok(serde_json::json!({
        "agent_id": handle.agent_id.to_string(),
        "status": "running",
    }))
}

/// Handles `agent.kill` method (B3).
async fn handle_agent_kill(
    params: AgentKillParams,
    orchestrator: &Arc<AgentOrchestrator>,
) -> Result<serde_json::Value, ServerError> {
    use claw_runtime::agent_types::AgentId;

    let agent_id = AgentId::new(params.agent_id.clone());
    orchestrator
        .kill(&agent_id)
        .await
        .map_err(|e| ServerError::Agent(format!("agent.kill failed: {}", e)))?;
    Ok(serde_json::json!({ "ok": true, "agent_id": params.agent_id }))
}

/// Handles `agent.steer` method (B3).
async fn handle_agent_steer(
    params: AgentSteerParams,
    orchestrator: &Arc<AgentOrchestrator>,
) -> Result<serde_json::Value, ServerError> {
    use claw_runtime::agent_types::AgentId;
    use claw_runtime::orchestrator::SteerCommand;

    let agent_id = AgentId::new(params.agent_id.clone());
    orchestrator
        .steer(
            &agent_id,
            SteerCommand::Custom {
                command: "inject".to_string(),
                payload: Some(params.message),
            },
        )
        .await
        .map_err(|e| ServerError::Agent(format!("agent.steer failed: {}", e)))?;
    Ok(serde_json::json!({ "ok": true, "agent_id": params.agent_id }))
}

/// Handles `agent.list` method (B3).
async fn handle_agent_list(
    orchestrator: &Arc<AgentOrchestrator>,
) -> Result<serde_json::Value, ServerError> {
    let agent_ids = orchestrator.agent_ids();
    let agents: Vec<serde_json::Value> = agent_ids
        .into_iter()
        .filter_map(|id| {
            orchestrator.agent_info(&id).map(|info| {
                serde_json::json!({
                    "agent_id": info.config.agent_id.to_string(),
                    "status": format!("{:?}", info.status),
                })
            })
        })
        .collect();
    Ok(serde_json::json!({ "agents": agents }))
}

// ─── B5: Tool registration ────────────────────────────────────────────────────

/// Handles `tool.register` method (B5).
///
/// Registers an external tool definition for later use by agent sessions.
async fn handle_tool_register(
    params: ToolRegisterParams,
    tool_registry: &Arc<crate::global_tool_registry::GlobalToolRegistry>,
    notify_tx: &mpsc::Sender<Vec<u8>>,
) -> Result<serde_json::Value, ServerError> {
    use crate::global_tool_registry::{ExecutorType, GlobalToolDef};
    let def = GlobalToolDef {
        name: params.name.clone(),
        description: params.description.clone(),
        schema: params.schema.clone(),
        executor: ExecutorType::External,
        caller_tx: Some(notify_tx.clone()),
    };
    tool_registry.register(def);
    tracing::debug!("tool.register: name={}", params.name);
    Ok(serde_json::json!({
        "name": params.name,
        "status": "registered",
    }))
}

/// Handles `tool.unregister` method (B5).
async fn handle_tool_unregister(
    params: ToolUnregisterParams,
    tool_registry: &Arc<crate::global_tool_registry::GlobalToolRegistry>,
) -> Result<serde_json::Value, ServerError> {
    let removed = tool_registry.unregister(&params.name);
    tracing::debug!("tool.unregister: name={}, removed={}", params.name, removed);
    Ok(serde_json::json!({
        "name": params.name,
        "status": "unregistered",
        "removed": removed,
    }))
}

/// Handles `tool.list` method (B5).
async fn handle_tool_list(
    tool_registry: &Arc<crate::global_tool_registry::GlobalToolRegistry>,
) -> Result<serde_json::Value, ServerError> {
    let tools: Vec<serde_json::Value> = tool_registry.list()
        .into_iter()
        .map(|t| serde_json::json!({
            "name": t.name,
            "description": t.description,
            "schema": t.schema,
        }))
        .collect();
    Ok(serde_json::json!(tools))
}

// ─── B6: Skill management ─────────────────────────────────────────────────────

/// Handles `skill.load_dir` method (B6).
async fn handle_skill_load_dir(
    params: SkillLoadDirParams,
    skill_registry: &Arc<crate::global_skill_registry::GlobalSkillRegistry>,
) -> Result<serde_json::Value, ServerError> {
    let count = skill_registry.load_dir(&params.path).await
        .map_err(|e| ServerError::Agent(format!("skill.load_dir failed: {}", e)))?;
    tracing::debug!("skill.load_dir: path={}, count={}", params.path, count);
    Ok(serde_json::json!({
        "path": params.path,
        "count": count,
    }))
}

/// Handles `skill.list` method (B6).
async fn handle_skill_list(
    skill_registry: &Arc<crate::global_skill_registry::GlobalSkillRegistry>,
) -> Result<serde_json::Value, ServerError> {
    let skills: Vec<serde_json::Value> = skill_registry.list().await
        .into_iter()
        .map(|m| serde_json::json!({
            "name": m.name,
            "description": m.description,
        }))
        .collect();
    Ok(serde_json::json!(skills))
}

/// Handles `skill.get_full` method (B6).
async fn handle_skill_get_full(
    params: SkillGetFullParams,
    skill_registry: &Arc<crate::global_skill_registry::GlobalSkillRegistry>,
) -> Result<serde_json::Value, ServerError> {
    let content = skill_registry.get_full(&params.name).await
        .map_err(|e| ServerError::Agent(format!("skill.get_full failed: {}", e)))?;
    Ok(serde_json::json!({
        "name": params.name,
        "content": content,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{DestroySessionParams, RequestId, SendMessageParams, ToolResultParams};

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

    /// Verify that framing round-trips correctly using a pair of Unix sockets.
    #[tokio::test]
    async fn test_frame_round_trip() {
        let (a, b) = UnixStream::pair().unwrap();
        let (mut reader_a, _writer_a) = a.into_split();
        let (_reader_b, mut writer_b) = b.into_split();

        let payload = b"hello, world!";
        write_frame(&mut writer_b, payload).await.unwrap();

        let received = read_frame(&mut reader_a).await.unwrap();
        assert_eq!(received, payload);
    }

    /// Verify that EOF on the reader side yields BrokenPipe.
    #[tokio::test]
    async fn test_read_frame_eof() {
        let (a, b) = UnixStream::pair().unwrap();
        let (mut reader_a, _writer_a) = a.into_split();

        // Drop the write side to produce EOF.
        drop(b);

        let err = read_frame(&mut reader_a).await.unwrap_err();
        assert!(matches!(
            err,
            ServerError::Ipc(claw_pal::error::IpcError::BrokenPipe)
        ));
    }

    /// Verify that frames larger than 16 MiB are rejected.
    #[tokio::test]
    async fn test_read_frame_too_large() {
        use tokio::io::AsyncWriteExt;

        let (a, b) = UnixStream::pair().unwrap();
        let (mut reader_a, _writer_a) = a.into_split();
        let (_reader_b, mut writer_b) = b.into_split();

        // Write a 17 MiB length header but no body.
        let oversized: u32 = 17 * 1024 * 1024;
        writer_b.write_all(&oversized.to_be_bytes()).await.unwrap();

        let err = read_frame(&mut reader_a).await.unwrap_err();
        assert!(matches!(
            err,
            ServerError::Ipc(claw_pal::error::IpcError::InvalidMessage)
        ));
    }

    /// Verify send_response produces a well-formed framed JSON response.
    #[tokio::test]
    async fn test_send_response_framed() {
        let (a, b) = UnixStream::pair().unwrap();
        let (mut reader_a, _writer_a) = a.into_split();
        let (_reader_b, mut writer_b) = b.into_split();

        let response = Response::success(
            serde_json::json!({ "test": true }),
            Some(RequestId::Number(1)),
        );
        send_response_direct(&mut writer_b, response).await.unwrap();

        let received = read_frame(&mut reader_a).await.unwrap();
        let text = String::from_utf8(received).unwrap();
        assert!(text.contains("test"));
        assert!(text.contains("true"));
    }
}
