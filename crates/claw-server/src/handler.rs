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
    TriggerAddCronParams, TriggerAddWebhookParams, TriggerAddEventParams, TriggerRemoveParams,
    // B3
    AgentSpawnParams, AgentKillParams, AgentSteerParams,
    // G-15
    AgentAnnounceParams,
    // B5
    ToolRegisterParams, ToolUnregisterParams,
    // B6
    SkillLoadDirParams, SkillGetFullParams,
    // Phase 3
    ChannelRouteAddParams, ChannelRouteRemoveParams,
    // G-02
    ChannelInboundParams,
    // G-02 (ext)
    ChannelBroadcastParams,
    // G-11
    ToolWatchDirParams, ToolReloadParams,
    // G-16
    AuditListParams,
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
    scheduler: Arc<claw_runtime::TokioScheduler>,
    webhook_server: Option<Arc<claw_runtime::webhook::AxumWebhookServer>>,
    trigger_store: Option<Arc<crate::trigger_store::TriggerStore>>,
    event_trigger_handles: Arc<dashmap::DashMap<String, tokio::task::AbortHandle>>,
    channel_router: Arc<claw_channel::router::ChannelRouter>,
    audit_log: claw_tools::audit::AuditLogWriterHandle,
    audit_store: Arc<claw_tools::audit::AuditStore>,
    hot_loader: crate::hot_loader::HotLoaderHandle,
) -> Result<(), ServerError> {
    info!("New client connection established");

    // ─── Unique connection ID for hot-loader subscription management ──────────
    static CONN_COUNTER: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let conn_id = CONN_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    // ─── Connection-level auth state ──────────────────────────────────────────
    // The first frame on any connection MUST be a `kernel.auth` handshake.
    // After a successful handshake this flag is set to true.
    let mut authenticated = false;

    let (notify_tx, mut notify_rx) = mpsc::channel::<Vec<u8>>(100);
    let (mut reader, writer_raw) = stream.into_split();
    let writer = Arc::new(Mutex::new(writer_raw));

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
                            &webhook_server,
                            &trigger_store,
                            &event_trigger_handles,
                            &channel_router,
                            &audit_log,
                            &audit_store,
                            &hot_loader,
                            conn_id,
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
    // Unsubscribe from hot-loader so dead senders are not retained.
    hot_loader.unsubscribe(conn_id);
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
    webhook_server: &Option<Arc<claw_runtime::webhook::AxumWebhookServer>>,
    trigger_store: &Option<Arc<crate::trigger_store::TriggerStore>>,
    event_trigger_handles: &Arc<dashmap::DashMap<String, tokio::task::AbortHandle>>,
    channel_router: &Arc<claw_channel::router::ChannelRouter>,
    audit_log: &claw_tools::audit::AuditLogWriterHandle,
    audit_store: &Arc<claw_tools::audit::AuditStore>,
    hot_loader: &crate::hot_loader::HotLoaderHandle,
    conn_id: u64,
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
            handle_create_session(request.params, &session_manager, provider, registry, notify_tx, audit_log, skill_registry).await
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
            }), &session_manager, provider, registry, notify_tx, audit_log, skill_registry).await
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
            let params: ChannelSendParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_channel_send(params, channel_registry).await
        }
        "channel.close" => {
            let params: ChannelCloseParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_channel_close(params, channel_registry).await
        }
        // ─── B1: Channel register/unregister/list ─────────────────────────────
        "channel.register" => {
            let params: ChannelRegisterParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_channel_register(params, channel_registry, notify_tx).await
        }
        "channel.unregister" => {
            let params: ChannelUnregisterParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_channel_unregister(params, channel_registry).await
        }
        "channel.list" => handle_channel_list(channel_registry).await,
        // ─── Phase 3: Channel routing ────────────────────────────────────────
        "channel.route_add" => {
            let params: ChannelRouteAddParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_channel_route_add(params, channel_router).await
        }
        "channel.route_remove" => {
            let params: ChannelRouteRemoveParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_channel_route_remove(params, channel_router).await
        }
        "channel.route_list" => handle_channel_route_list(channel_router).await,
        // ─── G-02 (ext): Channel broadcast ───────────────────────────────────
        "channel.broadcast" => {
            let params: ChannelBroadcastParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_channel_broadcast(
                params,
                Arc::clone(&session_manager),
                provider,
                channel_registry,
                channel_router,
            ).await
        }
        // ─── G-02: Inbound message pipeline ─────────────────────────────────
        "channel.inbound" => {
            let params: ChannelInboundParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_channel_inbound(
                params,
                Arc::clone(&session_manager),
                provider,
                channel_registry,
                channel_router,
                event_bus,
            ).await
        }
        // ─── B2: Trigger methods ──────────────────────────────────────────────
        "trigger.add_cron" => {
            let params: TriggerAddCronParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_trigger_add_cron(params, scheduler, orchestrator, trigger_store).await
        }
        "trigger.add_webhook" => {
            let params: TriggerAddWebhookParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_trigger_add_webhook(
                params,
                scheduler,
                channel_registry,
                orchestrator,
                webhook_server,
                trigger_store,
                Arc::clone(&session_manager),
                Arc::clone(provider),
                event_bus.clone(),
            ).await
        }
        // ─── G-08: EventTrigger ──────────────────────────────────────────────
        "trigger.add_event" => {
            let params: TriggerAddEventParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_trigger_add_event(
                params,
                event_bus,
                orchestrator,
                trigger_store,
                event_trigger_handles,
            ).await
        }
        "trigger.remove" => {
            let params: TriggerRemoveParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_trigger_remove(params, scheduler, trigger_store, event_trigger_handles).await
        }
        "trigger.list" => handle_trigger_list(scheduler).await,
        // ─── B3: Agent lifecycle ──────────────────────────────────────────────
        "agent.spawn" => {
            let params: AgentSpawnParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_agent_spawn(params, orchestrator, &session_manager, provider, registry, notify_tx).await
        }
        "agent.kill" => {
            let params: AgentKillParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_agent_kill(params, orchestrator, &session_manager).await
        }
        "agent.steer" => {
            let params: AgentSteerParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_agent_steer(params, &session_manager).await
        }
        "agent.list" => handle_agent_list(orchestrator, &session_manager).await,
        // ─── G-15: Agent Discovery ────────────────────────────────────────────
        "agent.announce" => {
            let params: AgentAnnounceParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_agent_announce(params, orchestrator).await
        }
        "agent.discover" => handle_agent_discover(orchestrator).await,
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
        // ─── G-11: HotLoader IPC endpoints ───────────────────────────────────
        "tool.watch_dir" => {
            let params: ToolWatchDirParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_tool_watch_dir(params, hot_loader, conn_id, notify_tx).await
        }
        "tool.reload" => {
            let params: ToolReloadParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_tool_reload(params, hot_loader).await
        }
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
        // ─── G-16: AuditLog IPC endpoint ──────────────────────────────────────
        "audit.list" => {
            let params: AuditListParams =
                serde_json::from_value(request.params.unwrap_or(serde_json::Value::Null))
                    .map_err(|e| ServerError::Serialization(format!("invalid params: {}", e)))?;
            handle_audit_list(params, audit_store).await
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
    audit_log: &claw_tools::audit::AuditLogWriterHandle,
    skill_registry: &Arc<crate::global_skill_registry::GlobalSkillRegistry>,
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
                tool_def
                    .permissions
                    .clone()
                    .unwrap_or_else(claw_tools::types::PermissionSet::minimal),
                session_id.clone(),
                notify_tx.clone(),
                Arc::clone(&pending_tool_calls),
                audit_log.clone(),
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

    // Inject SkillIndex into the system prompt (G-14).
    let skill_index = skill_registry.build_index().await.unwrap_or_default();
    let skill_block = if skill_index.is_empty() {
        None
    } else {
        Some(skill_index.to_prompt_block())
    };

    if let Some(ref cfg) = session_config {
        let base_sys = cfg.system_prompt.as_deref().unwrap_or("");
        let final_sys = match &skill_block {
            Some(block) if !base_sys.is_empty() => format!("{}\n\n{}", base_sys, block),
            Some(block) => block.clone(),
            None => base_sys.to_string(),
        };
        if !final_sys.is_empty() {
            builder = builder.with_system_prompt(final_sys);
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
    } else if let Some(ref block) = skill_block {
        // No session config at all, but skills exist — inject index alone.
        builder = builder.with_system_prompt(block.clone());
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
/// Registers an external channel into the kernel ChannelRegistry and stores
/// the current connection's `notify_tx` so that `channel.send` can push
/// outbound notifications back to the adapter process.
async fn handle_channel_register(
    params: ChannelRegisterParams,
    channel_registry: &Arc<ChannelRegistry>,
    notify_tx: &mpsc::Sender<Vec<u8>>,
) -> Result<serde_json::Value, ServerError> {
    channel_registry
        .register(params.r#type.clone(), params.channel_id.clone(), params.config, Some(notify_tx.clone()))
        .map_err(|e| ServerError::Serialization(e))?;
    info!("Channel registered: id={} type={}", params.channel_id, params.r#type);
    Ok(serde_json::json!({
        "ok": true,
        "channel_id": params.channel_id,
        "type": params.r#type,
    }))
}

/// Handles `channel.send` method (G-01 fix).
///
/// Routes a message to the adapter process that registered the named channel
/// by pushing a `channel/outbound` JSON-RPC notification via the connection's
/// saved `notify_tx`.
async fn handle_channel_send(
    params: ChannelSendParams,
    channel_registry: &Arc<ChannelRegistry>,
) -> Result<serde_json::Value, ServerError> {
    let notification = crate::protocol::Notification::new(
        "channel/outbound",
        Some(serde_json::json!({
            "channel_id": params.channel_id,
            "message": params.message,
        })),
    );
    let frame = serde_json::to_vec(&notification)
        .map_err(|e| ServerError::Serialization(e.to_string()))?;
    channel_registry
        .send_outbound(&params.channel_id, frame)
        .await
        .map_err(|e| ServerError::Serialization(e))?;
    info!("channel.send: routed message to channel={}", params.channel_id);
    Ok(serde_json::json!({ "ok": true, "channel_id": params.channel_id }))
}

/// Handles `channel.close` method (G-01 fix).
///
/// Closes (unregisters) a channel, releasing its outbound sender and removing
/// it from the registry.  This is the clean-up counterpart to `channel.register`.
async fn handle_channel_close(
    params: ChannelCloseParams,
    channel_registry: &Arc<ChannelRegistry>,
) -> Result<serde_json::Value, ServerError> {
    let removed = channel_registry.unregister(&params.channel_id);
    if removed {
        info!("channel.close: channel={} unregistered", params.channel_id);
    } else {
        warn!("channel.close: channel={} not found", params.channel_id);
    }
    Ok(serde_json::json!({ "ok": removed, "channel_id": params.channel_id }))
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

/// Handles `trigger.add_event` method (G-08).
///
/// Subscribes to the internal EventBus and steers `target_agent` whenever an
/// event whose canonical name matches `event_pattern` (glob syntax) is published.
/// The background listener task runs for the lifetime of the trigger; its
/// `AbortHandle` is stored in `event_trigger_handles` so `trigger.remove` can
/// cancel it.
async fn handle_trigger_add_event(
    params: TriggerAddEventParams,
    event_bus: &claw_runtime::EventBus,
    orchestrator: &Arc<AgentOrchestrator>,
    trigger_store: &Option<Arc<crate::trigger_store::TriggerStore>>,
    event_trigger_handles: &Arc<dashmap::DashMap<String, tokio::task::AbortHandle>>,
) -> Result<serde_json::Value, ServerError> {
    use crate::event_trigger::{
        build_event_pattern_regex, condition_matches, event_type_name, render_template,
    };
    use claw_runtime::agent_types::AgentId;
    use claw_runtime::orchestrator::SteerCommand;

    let trigger_id  = params.trigger_id.clone();
    let pattern_str = params.event_pattern.clone();
    let target      = params.target_agent.clone();
    let message     = params.message.clone();
    let condition   = params.condition.clone();

    // Validate the pattern early so the caller gets an immediate error.
    let pattern_regex = build_event_pattern_regex(&pattern_str)
        .map_err(|e| ServerError::Agent(format!("Invalid event_pattern: {}", e)))?;

    let mut rx  = event_bus.subscribe();
    let orch    = Arc::clone(orchestrator);
    let tid_log = trigger_id.clone();
    let eh      = Arc::clone(event_trigger_handles);

    let task = tokio::spawn(async move {
        loop {
            let event = match rx.recv().await {
                Ok(e)  => e,
                Err(_) => break, // EventBus closed — daemon is shutting down
            };

            let name = event_type_name(&event);
            if !pattern_regex.is_match(name.as_ref()) {
                continue;
            }
            if let Some(ref cond) = condition {
                if !condition_matches(&event, cond) {
                    continue;
                }
            }

            let msg = render_template(
                message.as_deref().unwrap_or("{event.type}"),
                &event,
            );
            let aid = AgentId::new(target.clone());
            if let Err(e) = orch.steer(&aid, SteerCommand::Custom {
                command: "inject".to_string(),
                payload: Some(msg),
            }).await {
                tracing::warn!("event trigger {}: steer failed: {}", tid_log, e);
            } else {
                tracing::debug!("event trigger {} fired for event {}", tid_log, name);
            }
        }
        // Clean up the handle entry when the task exits naturally.
        eh.remove(&tid_log);
    });

    event_trigger_handles.insert(trigger_id.clone(), task.abort_handle());

    // Persist so the trigger survives daemon restarts.
    if let Some(ts) = trigger_store {
        if let Err(e) = ts.save_event(
            &trigger_id,
            &pattern_str,
            &params.target_agent,
            params.message.as_deref(),
        ) {
            tracing::warn!("Failed to persist event trigger {}: {}", trigger_id, e);
        }
    }

    tracing::info!(
        "trigger.add_event: id={}, pattern={}, agent={}",
        trigger_id, pattern_str, params.target_agent,
    );

    Ok(serde_json::json!({
        "trigger_id":    trigger_id,
        "event_pattern": pattern_str,
        "target_agent":  params.target_agent,
        "status":        "active",
    }))
}

/// Handles `trigger.add_cron` method (B2).
async fn handle_trigger_add_cron(
    params: TriggerAddCronParams,
    scheduler: &Arc<claw_runtime::TokioScheduler>,
    orchestrator: &Arc<AgentOrchestrator>,
    trigger_store: &Option<Arc<crate::trigger_store::TriggerStore>>,
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

    // Persist the trigger for restart recovery.
    if let Some(ts) = trigger_store {
        if let Err(e) = ts.save_cron(
            &trigger_id,
            &cron_expr,
            &params.target_agent,
            params.message.as_deref(),
        ) {
            tracing::warn!("Failed to persist cron trigger {}: {}", trigger_id, e);
        }
    }

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
    _orchestrator: &Arc<AgentOrchestrator>,
    webhook_server: &Option<Arc<claw_runtime::webhook::AxumWebhookServer>>,
    trigger_store: &Option<Arc<crate::trigger_store::TriggerStore>>,
    session_manager: Arc<SessionManager>,
    provider: Arc<dyn claw_provider::traits::LLMProvider>,
    event_bus: claw_runtime::EventBus,
) -> Result<serde_json::Value, ServerError> {
    use claw_runtime::webhook::{EndpointConfig, WebhookServer};

    let endpoint = format!("/hooks/{}", params.trigger_id);

    if let Some(server) = webhook_server {
        let target = params.target_agent.clone();
        let trigger_id_str = params.trigger_id.clone();
        let sm = Arc::clone(&session_manager);
        let prov = Arc::clone(&provider);
        let ch_reg = Arc::clone(channel_registry);
        let eb = event_bus.clone();

        let mut ep_config = EndpointConfig::new(
            endpoint.clone(),
            move |req: claw_runtime::webhook::WebhookRequest| {
                let target = target.clone();
                let tid = trigger_id_str.clone();
                let sm = Arc::clone(&sm);
                let prov = Arc::clone(&prov);
                let ch_reg = Arc::clone(&ch_reg);
                let eb = eb.clone();
                async move {
                    let body = String::from_utf8_lossy(&req.body).to_string();

                    // Publish TriggerEvent to EventBus so subscribers (F6-03) can react.
                    let payload = serde_json::from_str::<serde_json::Value>(&body)
                        .unwrap_or(serde_json::Value::Null);
                    let trigger_event = claw_runtime::trigger_event::TriggerEvent::webhook(
                        tid.clone(),
                        payload,
                        Some(claw_runtime::agent_types::AgentId::new(target.clone())),
                    );
                    let _ = eb.publish(claw_runtime::events::Event::TriggerFired(trigger_event));

                    // Route through the inbound session pipeline.
                    // Use target_agent as the thread_id so repeated webhook
                    // invocations reuse the same conversation session.
                    let session = match get_or_create_inbound_session(
                        Some(&target),
                        &sm,
                        &prov,
                        &ch_reg,
                    )
                    .await
                    {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(
                                "webhook trigger {}: failed to get session: {}",
                                tid,
                                e
                            );
                            return Ok::<_, claw_runtime::webhook::WebhookError>(
                                claw_runtime::webhook::WebhookResponse::error(500, e.to_string()),
                            );
                        }
                    };

                    let (chunk_tx, _chunk_rx) =
                        tokio::sync::mpsc::channel::<claw_loop::StreamChunk>(256);
                    let mut loop_guard = session.agent_loop.lock().await;

                    match loop_guard.run_streaming(body, chunk_tx).await {
                        Ok(result) => {
                            let resp_body = serde_json::json!({
                                "session_id": session.id,
                                "content": result.content,
                            });
                            Ok(
                                claw_runtime::webhook::WebhookResponse::json(&resp_body)
                                    .unwrap_or_else(|_| {
                                        claw_runtime::webhook::WebhookResponse::ok()
                                    }),
                            )
                        }
                        Err(e) => {
                            tracing::error!(
                                "webhook trigger {}: agent loop error: {}",
                                tid,
                                e
                            );
                            Ok(claw_runtime::webhook::WebhookResponse::error(500, e.to_string()))
                        }
                    }
                }
            },
        );

        // Apply HMAC-SHA256 if the caller supplied a secret.
        if let Some(ref secret) = params.hmac_secret {
            ep_config = ep_config.with_hmac_sha256(secret.clone(), "X-Hub-Signature-256");
        }

        server
            .register(ep_config)
            .await
            .map_err(|e| {
                ServerError::Agent(format!("Failed to register webhook endpoint: {}", e))
            })?;

        // Persist the webhook trigger.
        if let Some(ts) = trigger_store {
            if let Err(e) = ts.save_webhook(&params.trigger_id, &params.target_agent, &endpoint) {
                tracing::warn!(
                    "Failed to persist webhook trigger {}: {}",
                    params.trigger_id,
                    e
                );
            }
        }

        tracing::info!(
            "trigger.add_webhook: id={}, agent={}, endpoint={}",
            params.trigger_id,
            params.target_agent,
            endpoint,
        );
    } else {
        // Fallback: just store in channel registry (no HTTP endpoint)
        let config = serde_json::json!({
            "endpoint": endpoint,
            "target_agent": params.target_agent,
        });
        channel_registry
            .register(
                "webhook_trigger".to_string(),
                format!("trigger:{}", params.trigger_id),
                config,
                None,
            )
            .unwrap_or_else(|e| tracing::warn!("trigger.add_webhook: registry conflict: {}", e));

        tracing::warn!(
            "trigger.add_webhook: webhook server not configured (set webhook_port in ServerConfig);              stored as channel registry entry only"
        );
    }

    Ok(serde_json::json!({
        "trigger_id": params.trigger_id,
        "endpoint": endpoint,
        "target_agent": params.target_agent,
        "status": if webhook_server.is_some() { "registered" } else { "stored_only" },
    }))
}
/// Handles `trigger.remove` method (B2).
async fn handle_trigger_remove(
    params: TriggerRemoveParams,
    scheduler: &Arc<claw_runtime::TokioScheduler>,
    trigger_store: &Option<Arc<crate::trigger_store::TriggerStore>>,
    event_trigger_handles: &Arc<dashmap::DashMap<String, tokio::task::AbortHandle>>,
) -> Result<serde_json::Value, ServerError> {
    use claw_runtime::{Scheduler, TaskId};

    // Cancel from scheduler (cron/webhook triggers).
    // Ignore "task not found" errors — the trigger may be an EventTrigger.
    let _ = scheduler
        .cancel(&TaskId::new(params.trigger_id.clone()))
        .await;

    // Abort the background EventBus listener if this is an EventTrigger.
    if let Some((_, abort_handle)) = event_trigger_handles.remove(&params.trigger_id) {
        abort_handle.abort();
        tracing::debug!("trigger.remove: aborted event trigger {}", params.trigger_id);
    }

    // Remove from persistent store.
    if let Some(ts) = trigger_store {
        if let Err(e) = ts.remove(&params.trigger_id) {
            tracing::warn!("Failed to remove persisted trigger {}: {}", params.trigger_id, e);
        }
    }

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
/// Builds a real `AgentLoop`, registers the agent with `AgentOrchestrator`,
/// and creates a `Session` that backs it. The AgentId→SessionId mapping is
/// stored in `SessionManager` so that `agent.steer` and `agent.kill` can
/// locate the live `AgentLoop`.
async fn handle_agent_spawn(
    params: AgentSpawnParams,
    orchestrator: &Arc<AgentOrchestrator>,
    session_manager: &SessionManager,
    provider: &Arc<dyn claw_provider::traits::LLMProvider>,
    registry: &Arc<crate::server::ProviderRegistry>,
    notify_tx: &mpsc::Sender<Vec<u8>>,
) -> Result<serde_json::Value, ServerError> {
    use claw_runtime::agent_types::{AgentConfig, AgentId};

    let agent_id = params
        .agent_id
        .map(AgentId::new)
        .unwrap_or_else(AgentId::generate);

    // 1. Build a real AgentLoop using the spawn config.
    let session_provider: Arc<dyn claw_provider::traits::LLMProvider> =
        match params.config.provider.as_deref() {
            Some(name) => registry
                .get(name)
                .ok_or_else(|| ServerError::Agent(format!("Provider '{}' not found", name)))?,
            None => Arc::clone(provider),
        };

    let mut builder = claw_loop::AgentLoopBuilder::new()
        .with_provider(session_provider)
        .with_agent_id(agent_id.as_str().to_string());

    if let Some(ref sys) = params.config.system_prompt {
        builder = builder.with_system_prompt(sys.clone());
    }
    if let Some(max_turns) = params.config.max_turns {
        builder = builder.with_max_turns(max_turns);
    }

    let agent_loop = builder
        .build()
        .map_err(|e| ServerError::Agent(format!("agent.spawn: build AgentLoop failed: {}", e)))?;

    // 2. Register the agent with the orchestrator (metadata + event publication).
    let mut config = AgentConfig::new(agent_id.as_str());
    config.agent_id = agent_id.clone();
    if let Some(ref sys) = params.config.system_prompt {
        config = config.with_meta("system_prompt", sys.as_str());
    }
    if let Some(ref p) = params.config.provider {
        config = config.with_meta("provider", p.as_str());
    }
    if let Some(ref m) = params.config.model {
        config = config.with_meta("model", m.as_str());
    }
    if let Some(mt) = params.config.max_turns {
        config = config.with_meta("max_turns", mt.to_string());
    }

    orchestrator
        .register(config)
        .map_err(|e| ServerError::Agent(format!("agent.spawn: orchestrator.register failed: {}", e)))?;

    // 3. Create a Session that owns the AgentLoop and bind it to the AgentId.
    let session = session_manager
        .create_for_agent(agent_id.clone(), notify_tx.clone(), agent_loop)
        .map_err(|e| {
            // Roll back orchestrator registration if session creation fails.
            let _ = orchestrator.unregister(&agent_id, "session_create_failed");
            e
        })?;

    info!("Agent spawned: agent_id={} session_id={}", agent_id, session.id);
    Ok(serde_json::json!({
        "agent_id": agent_id.to_string(),
        "session_id": session.id,
        "status": "running",
    }))
}

/// Handles `agent.kill` method (B3).
///
/// Removes the agent from the orchestrator **and** destroys the backing Session
/// (which drops the AgentLoop and frees all associated resources).
async fn handle_agent_kill(
    params: AgentKillParams,
    orchestrator: &Arc<AgentOrchestrator>,
    session_manager: &SessionManager,
) -> Result<serde_json::Value, ServerError> {
    use claw_runtime::agent_types::AgentId;

    let agent_id = AgentId::new(params.agent_id.clone());

    // Remove the backing session (drops the AgentLoop).
    session_manager.remove_by_agent(&agent_id);

    // Unregister from orchestrator (publishes AgentStopped event).
    // We tolerate NotFound here because the agent may have been killed already.
    if let Err(e) = orchestrator.unregister(&agent_id, "killed") {
        warn!("agent.kill: orchestrator.unregister for {}: {}", agent_id, e);
    }

    Ok(serde_json::json!({ "ok": true, "agent_id": params.agent_id }))
}

/// Handles `agent.steer` method (B3).
///
/// Looks up the Session backing the target agent and injects the message by
/// running `AgentLoop::run_streaming`, mirroring the `sendMessage` flow.
async fn handle_agent_steer(
    params: AgentSteerParams,
    session_manager: &SessionManager,
) -> Result<serde_json::Value, ServerError> {
    use claw_runtime::agent_types::AgentId;

    let agent_id = AgentId::new(params.agent_id.clone());

    // Look up the session backed by this agent.
    let session_id = session_manager
        .session_for_agent(&agent_id)
        .ok_or_else(|| ServerError::Agent(format!(
            "agent.steer: agent '{}' not found or has no running session",
            params.agent_id
        )))?;

    let session = session_manager
        .get(&session_id)
        .ok_or_else(|| ServerError::SessionNotFound(session_id.clone()))?;

    let message = params.message.clone();
    let sid_for_log = session_id.clone();
    let session_clone = Arc::clone(&session);

    // Spawn a task so we return immediately (same pattern as sendMessage).
    tokio::spawn(async move {
        let (chunk_tx, mut chunk_rx) =
            tokio::sync::mpsc::channel::<claw_loop::StreamChunk>(256);

        let session_for_chunks = Arc::clone(&session_clone);
        tokio::spawn(async move {
            while let Some(chunk) = chunk_rx.recv().await {
                if let claw_loop::StreamChunk::Text { content, is_final } = chunk {
                    if !content.is_empty() || is_final {
                        let _ = notify_chunk(&session_for_chunks, content, is_final).await;
                    }
                }
            }
        });

        let mut loop_guard = session_clone.agent_loop.lock().await;
        match loop_guard.run_streaming(message, chunk_tx).await {
            Ok(result) => {
                let reason = match result.finish_reason {
                    FinishReason::Stop => "stop",
                    FinishReason::MaxTurns => "max_turns",
                    FinishReason::TokenBudget => "token_budget",
                    FinishReason::NoToolCall => "no_tool_call",
                    FinishReason::StopCondition => "stop_condition",
                    FinishReason::Error => "error",
                    _ => "unknown",
                };
                let _ = notify_finish(&session_clone, Some(result.content), reason).await;
            }
            Err(e) => {
                tracing::error!("Agent steer loop error for session {}: {}", sid_for_log, e);
                let _ =
                    notify_finish(&session_clone, None::<String>, format!("error: {e}")).await;
            }
        }
    });

    Ok(serde_json::json!({
        "ok": true,
        "agent_id": params.agent_id,
        "session_id": session_id,
        "status": "accepted",
    }))
}

/// Handles `agent.list` method (B3).
///
/// Returns agents registered in the orchestrator, enriched with the
/// `session_id` of their backing session when available.
async fn handle_agent_list(
    orchestrator: &Arc<AgentOrchestrator>,
    session_manager: &SessionManager,
) -> Result<serde_json::Value, ServerError> {
    let agent_ids = orchestrator.agent_ids();
    let agents: Vec<serde_json::Value> = agent_ids
        .into_iter()
        .filter_map(|id| {
            orchestrator.agent_info(&id).map(|info| {
                let session_id = session_manager.session_for_agent(&id);
                let mut entry = serde_json::json!({
                    "agent_id": info.config.agent_id.to_string(),
                    "status": format!("{:?}", info.status),
                    "resource_usage": info.resource_usage,
                });
                if let Some(sid) = session_id {
                    entry["session_id"] = serde_json::Value::String(sid);
                }
                entry
            })
        })
        .collect();
    Ok(serde_json::json!({ "agents": agents }))
}

// ─── G-15: Agent Discovery ────────────────────────────────────────────────────

/// Handles `agent.announce` method (G-15).
///
/// Stores capability declarations for the given agent ID.  Replaces any
/// previously announced capabilities for that agent.
async fn handle_agent_announce(
    params: AgentAnnounceParams,
    orchestrator: &Arc<AgentOrchestrator>,
) -> Result<serde_json::Value, ServerError> {
    use claw_runtime::agent_types::AgentId;
    let agent_id = AgentId(params.agent_id.clone());
    orchestrator.announce_capabilities(agent_id, params.capabilities.clone());
    debug!(
        "agent.announce: agent_id={}, capabilities={:?}",
        params.agent_id, params.capabilities
    );
    Ok(serde_json::json!({
        "agent_id": params.agent_id,
        "capabilities": params.capabilities,
        "status": "announced",
    }))
}

/// Handles `agent.discover` method (G-15).
///
/// Returns all currently registered agents together with their declared
/// capabilities and lifecycle status.
async fn handle_agent_discover(
    orchestrator: &Arc<AgentOrchestrator>,
) -> Result<serde_json::Value, ServerError> {
    let entries: Vec<serde_json::Value> = orchestrator
        .discover_capabilities()
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "agent_id": e.agent_id.0,
                "capabilities": e.capabilities,
                "status": e.status,
            })
        })
        .collect();
    Ok(serde_json::json!({ "agents": entries }))
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
    let permissions = params
        .permissions
        .unwrap_or_else(claw_tools::types::PermissionSet::minimal);
    let def = GlobalToolDef {
        name: params.name.clone(),
        description: params.description.clone(),
        schema: params.schema.clone(),
        executor: ExecutorType::External,
        caller_tx: Some(notify_tx.clone()),
        permissions,
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

// ─── G-11: HotLoader IPC handlers ─────────────────────────────────────────────

/// Handles `tool.watch_dir` (G-11).
///
/// Adds `path` to the server-level file watcher and subscribes this IPC
/// connection to receive `tool/hot_reloaded` push notifications when any
/// watched script file changes.
async fn handle_tool_watch_dir(
    params: ToolWatchDirParams,
    hot_loader: &crate::hot_loader::HotLoaderHandle,
    conn_id: u64,
    notify_tx: &mpsc::Sender<Vec<u8>>,
) -> Result<serde_json::Value, ServerError> {
    let path = std::path::PathBuf::from(&params.path);
    hot_loader
        .watch_dir(path.clone())
        .map_err(|e| ServerError::Serialization(format!("tool.watch_dir failed: {}", e)))?;
    // Register this connection (idempotent per conn_id).
    hot_loader.subscribe_conn(conn_id, notify_tx.clone());
    tracing::info!("tool.watch_dir: watching {:?}", path);
    Ok(serde_json::json!({
        "path": params.path,
        "status": "watching",
    }))
}

/// Handles `tool.reload` (G-11).
///
/// Injects a manual file-changed event for `path`. All IPC connections
/// that have called `tool.watch_dir` will receive a `tool/hot_reloaded`
/// notification. The 50 ms debounce still applies.
async fn handle_tool_reload(
    params: ToolReloadParams,
    hot_loader: &crate::hot_loader::HotLoaderHandle,
) -> Result<serde_json::Value, ServerError> {
    let path = std::path::PathBuf::from(&params.path);
    hot_loader
        .trigger_reload(path.clone())
        .await
        .map_err(|e| ServerError::Serialization(format!("tool.reload failed: {}", e)))?;
    tracing::info!("tool.reload: triggered for {:?}", path);
    Ok(serde_json::json!({
        "path": params.path,
        "status": "reload_triggered",
    }))
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

// ─── Phase 3: Channel routing ──────────────────────────────────────────────────

/// Handles `channel.route_add` method (Phase 3).
async fn handle_channel_route_add(
    params: ChannelRouteAddParams,
    channel_router: &Arc<claw_channel::router::ChannelRouter>,
) -> Result<serde_json::Value, ServerError> {
    use claw_channel::router::RoutingRule;

    let rule = match params.rule_type.as_str() {
        "channel" => {
            let channel_id = params.channel_id
                .ok_or_else(|| ServerError::Serialization("channel_id required for 'channel' rule".to_string()))?;
            RoutingRule::ByChannelId { channel_id, agent_id: params.agent_id.clone() }
        }
        "sender" => {
            let sender_id = params.sender_id
                .ok_or_else(|| ServerError::Serialization("sender_id required for 'sender' rule".to_string()))?;
            RoutingRule::BySenderId { sender_id, agent_id: params.agent_id.clone() }
        }
        "pattern" => {
            let pattern_str = params.pattern
                .ok_or_else(|| ServerError::Serialization("pattern required for 'pattern' rule".to_string()))?;
            let regex = regex::Regex::new(&pattern_str)
                .map_err(|e| ServerError::Serialization(format!("invalid regex: {}", e)))?;
            RoutingRule::ByPattern { pattern: regex, agent_id: params.agent_id.clone() }
        }
        "default" => RoutingRule::Default { agent_id: params.agent_id.clone() },
        other => return Err(ServerError::Serialization(format!("Unknown rule type: {}", other))),
    };

    channel_router.add_rule(rule);
    tracing::info!("channel.route_add: type={}, agent={}", params.rule_type, params.agent_id);
    Ok(serde_json::json!({
        "ok": true,
        "rule_type": params.rule_type,
        "agent_id": params.agent_id,
    }))
}

/// Handles `channel.route_remove` method (Phase 3).
async fn handle_channel_route_remove(
    params: ChannelRouteRemoveParams,
    channel_router: &Arc<claw_channel::router::ChannelRouter>,
) -> Result<serde_json::Value, ServerError> {
    let removed = channel_router.remove_rules_for_agent(&params.agent_id);
    Ok(serde_json::json!({
        "ok": true,
        "agent_id": params.agent_id,
        "removed": removed,
    }))
}

/// Handles `channel.route_list` method (Phase 3).
async fn handle_channel_route_list(
    channel_router: &Arc<claw_channel::router::ChannelRouter>,
) -> Result<serde_json::Value, ServerError> {
    let rules = channel_router.list_rules();
    Ok(serde_json::json!(rules))
}

// ─── G-16: AuditLog IPC endpoint ──────────────────────────────────────────────

/// Handles `audit.list` method (G-16).
///
/// Returns recent audit events from the in-memory ring buffer.
/// Results are ordered most-recent-first (up to `limit`, default 100).
async fn handle_audit_list(
    params: AuditListParams,
    audit_store: &Arc<claw_tools::audit::AuditStore>,
) -> Result<serde_json::Value, ServerError> {
    let limit = params.limit.unwrap_or(100).min(1000);
    let events = audit_store.list(
        limit,
        params.agent_id.as_deref(),
        params.since_ms,
    );
    let count = events.len();
    let entries: Vec<serde_json::Value> = events
        .into_iter()
        .map(|e| serde_json::to_value(&e).unwrap_or(serde_json::Value::Null))
        .collect();
    Ok(serde_json::json!({
        "entries": entries,
        "count": count,
    }))
}

// ─── G-02: Inbound message pipeline ──────────────────────────────────────────

/// Get or create a session for an inbound channel message.
///
/// If `thread_id` is provided and the registry has a live session for it,
/// that session is reused.  Otherwise a new session is created with a
/// discard notify channel (responses are delivered via `channel.send`).
pub(crate) async fn get_or_create_inbound_session(
    thread_id: Option<&str>,
    session_manager: &Arc<SessionManager>,
    provider: &Arc<dyn claw_provider::traits::LLMProvider>,
    channel_registry: &Arc<ChannelRegistry>,
) -> Result<Arc<Session>, ServerError> {
    // Reuse existing session if we have one for this thread.
    if let Some(tid) = thread_id {
        if let Some(session_id) = channel_registry.get_thread_session(tid) {
            if let Some(session) = session_manager.get(&session_id) {
                debug!("Reusing session {} for thread {}", session_id, tid);
                return Ok(session);
            }
            // Session was destroyed; fall through and create a fresh one.
        }
    }

    // Create a new session with a discard notify channel.
    // Inbound sessions deliver responses via channel_registry.send_outbound(),
    // not through the originating IPC connection.
    let (discard_tx, discard_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);

    // Keep discard_rx alive so that session.notify() does not return BrokenPipe.
    tokio::spawn(async move {
        let mut rx = discard_rx;
        while rx.recv().await.is_some() {}
    });

    let session_id = uuid::Uuid::new_v4().to_string();
    let pending: Arc<dashmap::DashMap<String, tokio::sync::oneshot::Sender<(serde_json::Value, bool)>>> =
        Arc::new(dashmap::DashMap::new());

    let agent_loop = claw_loop::AgentLoopBuilder::new()
        .with_provider(Arc::clone(provider))
        .with_agent_id(session_id.clone())
        .build()
        .map_err(|e| ServerError::Agent(format!("Failed to build agent loop for inbound session: {}", e)))?;

    let session = session_manager.create_with_id(
        session_id.clone(),
        discard_tx,
        agent_loop,
        pending,
    )?;

    // Remember the session for this thread so future messages reuse it.
    if let Some(tid) = thread_id {
        channel_registry.set_thread_session(tid.to_string(), session_id.clone());
    }

    info!("Created inbound session {} for thread {:?}", session_id, thread_id);
    Ok(session)
}

// ─── G-02 (ext): Channel broadcast ────────────────────────────────────────────

/// Handles `channel.broadcast` method.
///
/// Routes `msg` through [`ChannelRouter::broadcast_route`] (multi-match
/// semantics) and steers **every** matched agent with the same message.
/// Useful for fan-out to monitoring agents, multi-agent voting, and audit.
///
/// Return value contains `"agents_notified"` with the count of agents that
/// were successfully steered, plus `"agent_ids"` for diagnostic purposes.
async fn handle_channel_broadcast(
    params: ChannelBroadcastParams,
    session_manager: Arc<SessionManager>,
    provider: &Arc<dyn claw_provider::traits::LLMProvider>,
    channel_registry: &Arc<ChannelRegistry>,
    channel_router: &Arc<claw_channel::router::ChannelRouter>,
) -> Result<serde_json::Value, ServerError> {
    // 1. Dedup check (same TTL mechanism as channel.inbound).
    if let Some(ref msg_id) = params.message_id {
        if channel_registry.is_duplicate(msg_id).await {
            debug!("channel.broadcast: duplicate message_id={}, skipping", msg_id);
            return Ok(serde_json::json!({ "status": "duplicate", "skipped": true }));
        }
    }

    // 2. Multi-match routing — collect all matching agents.
    let mut channel_msg = claw_channel::types::ChannelMessage::inbound(
        claw_channel::types::ChannelId::new(&params.channel_id),
        claw_channel::types::Platform::Stdin,
        &params.content,
    );
    channel_msg.sender_id = Some(params.sender_id.clone());
    channel_msg.thread_id = params.thread_id.clone();
    channel_msg.metadata = serde_json::json!({ "extra": params.metadata });

    let agent_ids = channel_router.broadcast_route(&channel_msg);
    if agent_ids.is_empty() {
        return Err(ServerError::Serialization(format!(
            "channel.broadcast: no routing rules matched channel '{}'. \
             Add rules with channel.route_add first.",
            params.channel_id
        )));
    }

    info!(
        channel = %params.channel_id,
        agents  = ?agent_ids,
        "channel.broadcast: fan-out to {} agent(s)",
        agent_ids.len()
    );

    // 3. Steer each matched agent in its own background task.
    //    Each agent gets its own session (or reuses its thread session).
    let mut notified: Vec<String> = Vec::with_capacity(agent_ids.len());

    for agent_id in &agent_ids {
        // Each broadcast target gets a fresh session keyed by
        // "<thread_id>/<agent_id>" so that different agents don't share state.
        let thread_key = params.thread_id.as_deref().map(|tid| format!("{}/{}", tid, agent_id));

        let session = match get_or_create_inbound_session(
            thread_key.as_deref(),
            &session_manager,
            provider,
            channel_registry,
        ).await {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    "channel.broadcast: failed to get/create session for agent '{}': {}",
                    agent_id, e
                );
                continue;
            }
        };

        let content      = params.content.clone();
        let channel_id   = params.channel_id.clone();
        let sender_id    = params.sender_id.clone();
        let target_agent = agent_id.clone();
        let channel_reg  = Arc::clone(channel_registry);
        let session_id   = session.id.clone();

        tokio::spawn(async move {
            let (chunk_tx, _chunk_rx) = tokio::sync::mpsc::channel::<claw_loop::StreamChunk>(256);
            let mut loop_guard = session.agent_loop.lock().await;

            match loop_guard.run_streaming(content, chunk_tx).await {
                Ok(result) => {
                    let reply = serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "channel/inbound_reply",
                        "params": {
                            "channel_id": channel_id,
                            "sender_id": sender_id,
                            "session_id": session_id,
                            "agent_id": target_agent,
                            "content": result.content,
                            "finish_reason": "stop",
                            "broadcast": true,
                        }
                    });
                    match serde_json::to_vec(&reply) {
                        Ok(payload) => {
                            let mut framed = Vec::with_capacity(4 + payload.len());
                            framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                            framed.extend_from_slice(&payload);
                            if let Err(e) = channel_reg.send_outbound(&channel_id, framed).await {
                                warn!(
                                    "channel.broadcast: failed to send reply for agent '{}' on channel '{}': {}",
                                    target_agent, channel_id, e
                                );
                            }
                        }
                        Err(e) => warn!("channel.broadcast: serialisation error: {}", e),
                    }
                }
                Err(e) => warn!(
                    "channel.broadcast: agent loop error for agent '{}': {}",
                    target_agent, e
                ),
            }
        });

        notified.push(agent_id.clone());
    }

    Ok(serde_json::json!({
        "status": "dispatched",
        "agents_notified": notified.len(),
        "agent_ids": notified,
    }))
}

/// Handles `channel.inbound` method (G-02).
///
/// Entry point for all inbound messages pushed by external channel adapters.
/// Performs deduplication, routing, session management, and fires the agent
/// loop in a background task.  The agent's reply is delivered back to the
/// adapter via a `channel/inbound_reply` notification sent through
/// `channel_registry.send_outbound()`.
async fn handle_channel_inbound(
    params: ChannelInboundParams,
    session_manager: Arc<SessionManager>,
    provider: &Arc<dyn claw_provider::traits::LLMProvider>,
    channel_registry: &Arc<ChannelRegistry>,
    channel_router: &Arc<claw_channel::router::ChannelRouter>,
    event_bus: &claw_runtime::EventBus,
) -> Result<serde_json::Value, ServerError> {
    // 1. Idempotency / dedup check.
    if let Some(ref msg_id) = params.message_id {
        if channel_registry.is_duplicate(msg_id).await {
            debug!("channel.inbound: duplicate message_id={}, skipping", msg_id);
            return Ok(serde_json::json!({ "status": "duplicate", "skipped": true }));
        }
    }

    // 2. Route the message to an agent via ChannelRouter.
    //    Build a ChannelMessage so that all RoutingRule variants work correctly.
    let mut channel_msg = claw_channel::types::ChannelMessage::inbound(
        claw_channel::types::ChannelId::new(&params.channel_id),
        claw_channel::types::Platform::Stdin, // generic; adapter knows the real platform
        &params.content,
    );
    channel_msg.sender_id = Some(params.sender_id.clone());
    channel_msg.thread_id = params.thread_id.clone();
    channel_msg.metadata = serde_json::json!({ "extra": params.metadata });

    let agent_id_hint = channel_router.route(&channel_msg).ok_or_else(|| {
        ServerError::Serialization(format!(
            "No routing rule matched channel '{}'. \
             Add a rule with channel.route_add before sending inbound messages.",
            params.channel_id
        ))
    })?;

    // 2.5. Publish MessageReceived to the EventBus (GAP-05 fix).
    //
    // This wires inbound channel messages into the event-driven architecture so
    // that EventTriggers subscribed to "message.received" patterns fire, and
    // AgentOrchestrator can observe channel traffic via the EventBus.
    event_bus.publish(claw_runtime::events::Event::MessageReceived {
        agent_id: claw_runtime::agent_types::AgentId::new(&agent_id_hint),
        channel: params.channel_id.clone(),
        message_type: "channel_inbound".to_string(),
    });

    // 3. Get or create a session for this thread.
    let session = get_or_create_inbound_session(
        params.thread_id.as_deref(),
        &session_manager,
        provider,
        channel_registry,
    ).await?;

    let session_id = session.id.clone();
    let session_id_for_response = session_id.clone();
    let content = params.content.clone();
    let channel_id = params.channel_id.clone();
    let sender_id = params.sender_id.clone();
    let channel_reg = Arc::clone(channel_registry);

    // 4. Run the agent loop in a background task; deliver the reply via
    //    channel_registry.send_outbound() as a `channel/inbound_reply` notification.
    tokio::spawn(async move {
        let (chunk_tx, _chunk_rx) = tokio::sync::mpsc::channel::<claw_loop::StreamChunk>(256);
        let mut loop_guard = session.agent_loop.lock().await;

        match loop_guard.run_streaming(content, chunk_tx).await {
            Ok(result) => {
                let reply = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "channel/inbound_reply",
                    "params": {
                        "channel_id": channel_id,
                        "sender_id": sender_id,
                        "session_id": session_id,
                        "content": result.content,
                        "finish_reason": "stop",
                    }
                });
                match serde_json::to_vec(&reply) {
                    Ok(payload) => {
                        let mut framed = Vec::with_capacity(4 + payload.len());
                        framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                        framed.extend_from_slice(&payload);
                        if let Err(e) = channel_reg.send_outbound(&channel_id, framed).await {
                            warn!(
                                "channel.inbound: failed to send reply for channel '{}': {}",
                                channel_id, e
                            );
                        }
                    }
                    Err(e) => {
                        error!("channel.inbound: failed to serialize reply: {}", e);
                    }
                }
            }
            Err(e) => {
                error!(
                    "channel.inbound: agent loop error for session {}: {}",
                    session_id, e
                );
            }
        }
    });

    Ok(serde_json::json!({
        "status": "accepted",
        "session_id": session_id_for_response,
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
