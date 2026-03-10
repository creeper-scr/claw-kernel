//! Integration tests for KernelServer.
//!
//! Tests the full JSON-RPC 2.0 over IPC flow:
//!   client → 4-byte-BE-framed request → server → AgentLoop → 4-byte-BE-framed response/notification → client
//!
//! All tests use a mock LLM provider to avoid network calls.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use claw_loop::AgentLoopBuilder;
use claw_provider::{
    error::ProviderError,
    traits::LLMProvider,
    types::{CompletionResponse, Delta, FinishReason, Message, Options, TokenUsage},
};
use claw_server::{KernelServer, ProviderConfig, ServerConfig};
use futures::stream;
use serde_json::{json, Value};
use std::pin::Pin;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::timeout;

// ── Mock provider ─────────────────────────────────────────────────────────────

struct MockProvider {
    response: String,
}

impl MockProvider {
    fn new(response: impl Into<String>) -> Arc<Self> {
        Arc::new(Self { response: response.into() })
    }
}

#[async_trait]
impl LLMProvider for MockProvider {
    fn provider_id(&self) -> &str { "mock" }
    fn model_id(&self) -> &str { "mock-v1" }

    async fn complete_inner(
        &self,
        _messages: Vec<Message>,
        _opts: Options,
    ) -> Result<CompletionResponse, ProviderError> {
        Ok(CompletionResponse {
            id: "mock-id".to_string(),
            model: "mock-v1".to_string(),
            message: Message::assistant(self.response.clone()),
            finish_reason: FinishReason::Stop,
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
        })
    }

    async fn complete_stream(
        &self,
        _messages: Vec<Message>,
        _opts: Options,
    ) -> Result<Pin<Box<dyn futures::Stream<Item = Result<Delta, ProviderError>> + Send>>, ProviderError> {
        Ok(Box::pin(stream::empty()))
    }
}

// ── Framing helpers (inline, matching server implementation) ──────────────────

/// Write one 4-byte-BE-length-prefixed frame.
async fn write_frame(stream: &mut UnixStream, data: &[u8]) {
    let len = data.len() as u32;
    stream.write_all(&len.to_be_bytes()).await.unwrap();
    stream.write_all(data).await.unwrap();
}

/// Read one 4-byte-BE-length-prefixed frame.
async fn read_frame(stream: &mut UnixStream) -> Vec<u8> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await.unwrap();
    let len = u32::from_be_bytes(header) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await.unwrap();
    buf
}

/// Send a JSON-RPC request and return the parsed response.
async fn rpc(stream: &mut UnixStream, method: &str, params: Option<Value>, id: i64) -> Value {
    let request = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": id,
    });
    write_frame(stream, request.to_string().as_bytes()).await;

    let raw = timeout(Duration::from_secs(5), read_frame(stream))
        .await
        .expect("response timed out");
    serde_json::from_slice(&raw).unwrap()
}

/// Read the next framed notification (skips nothing).
#[allow(dead_code)]
async fn read_notification(stream: &mut UnixStream) -> Value {
    let raw = timeout(Duration::from_secs(10), read_frame(stream))
        .await
        .expect("notification timed out");
    serde_json::from_slice(&raw).unwrap()
}

// ── Server fixture ────────────────────────────────────────────────────────────

/// Binds a KernelServer on a temp socket and returns (server_handle, auth_token).
async fn start_server(socket_path: &str) -> (tokio::task::JoinHandle<()>, String) {
    let path = socket_path.to_string();
    let config = ServerConfig {
        socket_path: path,
        max_sessions: 10,
        webhook_port: None,
        provider_config: ProviderConfig::Dynamic, // will be overridden by AgentLoop
    };

    // We can't directly inject a mock provider via the public API, so we build
    // a server backed by Ollama and rely on `ProviderConfig::Dynamic` fallback.
    // For tests that don't invoke the LLM, this works fine.
    // For sendMessage tests, we test via the session manager directly.
    let server = KernelServer::new(config);
    let token = server.auth_token.as_ref().clone();
    let handle = tokio::spawn(async move {
        // Run for a limited time — tests must complete before this timeout.
        let _ = timeout(Duration::from_secs(30), server.run()).await;
    });
    (handle, token)
}

/// Authenticate a client connection using the given token.
async fn authenticate(stream: &mut tokio::net::UnixStream, token: &str) {
    let resp = rpc(
        stream,
        "kernel.auth",
        Some(json!({ "token": token })),
        0,
    )
    .await;
    assert!(resp["error"].is_null(), "kernel.auth failed: {:?}", resp);
}

// ── Test: 4-byte frame round-trip (no server needed) ─────────────────────────

#[tokio::test]
async fn test_frame_round_trip_direct() {
    let (mut a, mut b) = UnixStream::pair().unwrap();

    let payload = b"hello, world!";
    write_frame(&mut b, payload).await;

    let received = read_frame(&mut a).await;
    assert_eq!(received, payload);
}

// ── Test: createSession → returns session_id ──────────────────────────────────

#[tokio::test]
async fn test_create_session_returns_id() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("test_create.sock");
    let path_str = socket_path.to_str().unwrap().to_string();

    let (_server, token) = start_server(&path_str).await;
    tokio::time::sleep(Duration::from_millis(50)).await; // let server bind

    let mut client = UnixStream::connect(&path_str).await.unwrap();
    authenticate(&mut client, &token).await;

    let resp = rpc(&mut client, "createSession", None, 1).await;

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert!(resp["result"]["session_id"].is_string(), "expected session_id in result");
}

// ── Test: createSession with config → respects system_prompt ─────────────────

#[tokio::test]
async fn test_create_session_with_config() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("test_create_cfg.sock");
    let path_str = socket_path.to_str().unwrap().to_string();

    let (_server, token) = start_server(&path_str).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client = UnixStream::connect(&path_str).await.unwrap();
    authenticate(&mut client, &token).await;

    let resp = rpc(
        &mut client,
        "createSession",
        Some(json!({
            "config": {
                "system_prompt": "You are a pirate. Respond only in pirate speak.",
                "max_turns": 5
            }
        })),
        1,
    )
    .await;

    assert!(resp["result"]["session_id"].is_string());
}

// ── Test: sendMessage on unknown session → error ──────────────────────────────

#[tokio::test]
async fn test_send_message_unknown_session() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("test_unknown.sock");
    let path_str = socket_path.to_str().unwrap().to_string();

    let (_server, token) = start_server(&path_str).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client = UnixStream::connect(&path_str).await.unwrap();
    authenticate(&mut client, &token).await;

    let resp = rpc(
        &mut client,
        "sendMessage",
        Some(json!({
            "session_id": "does-not-exist",
            "content": "hello"
        })),
        2,
    )
    .await;

    assert!(resp["error"].is_object(), "expected JSON-RPC error for unknown session");
    assert_eq!(resp["error"]["code"], -32000); // SESSION_NOT_FOUND
}

// ── Test: destroySession → removes session ────────────────────────────────────

#[tokio::test]
async fn test_destroy_session() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("test_destroy.sock");
    let path_str = socket_path.to_str().unwrap().to_string();

    let (_server, token) = start_server(&path_str).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client = UnixStream::connect(&path_str).await.unwrap();
    authenticate(&mut client, &token).await;

    // Create session
    let resp = rpc(&mut client, "createSession", None, 1).await;
    let session_id = resp["result"]["session_id"].as_str().unwrap().to_string();

    // Destroy it
    let resp = rpc(
        &mut client,
        "destroySession",
        Some(json!({ "session_id": session_id })),
        2,
    )
    .await;

    assert_eq!(resp["result"]["status"], "destroyed");

    // Destroy again → error
    let resp = rpc(
        &mut client,
        "destroySession",
        Some(json!({ "session_id": session_id })),
        3,
    )
    .await;
    assert!(resp["error"].is_object());
}

// ── Test: unknown method → error ─────────────────────────────────────────────

#[tokio::test]
async fn test_unknown_method() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("test_unknown_method.sock");
    let path_str = socket_path.to_str().unwrap().to_string();

    let (_server, token) = start_server(&path_str).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client = UnixStream::connect(&path_str).await.unwrap();
    authenticate(&mut client, &token).await;

    let resp = rpc(&mut client, "nonExistentMethod", None, 1).await;

    assert!(resp["error"].is_object());
}

// ── Test: invalid JSON → parse error ─────────────────────────────────────────

#[tokio::test]
async fn test_invalid_json_parse_error() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("test_invalid_json.sock");
    let path_str = socket_path.to_str().unwrap().to_string();

    let (_server, _token) = start_server(&path_str).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client = UnixStream::connect(&path_str).await.unwrap();

    // Send malformed JSON as a frame
    write_frame(&mut client, b"{ this is not json }").await;

    // Server should respond with a parse error (no id since we couldn't parse the request)
    let raw = timeout(Duration::from_secs(3), read_frame(&mut client))
        .await
        .expect("response timed out");
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert!(resp["error"].is_object());
    assert_eq!(resp["error"]["code"], -32700); // PARSE_ERROR
}

// ── Test: SessionManager AgentLoop integration (unit-level) ──────────────────

#[tokio::test]
async fn test_session_manager_with_agent_loop() {
    use claw_server::SessionManager;
    use tokio::sync::mpsc;

    let manager = SessionManager::new(10);
    let (tx, _rx) = mpsc::channel(10);

    let agent_loop = AgentLoopBuilder::new()
        .with_provider(MockProvider::new("hello"))
        .build()
        .unwrap();

    let session = manager.create(tx, agent_loop).unwrap();
    assert_eq!(manager.count(), 1);

    // pending_tool_calls starts empty
    assert!(session.pending_tool_calls.is_empty());

    manager.remove(&session.id);
    assert_eq!(manager.count(), 0);
}

// ── Test: toolResult routing via pending_tool_calls ───────────────────────────

#[tokio::test]
async fn test_tool_result_routing() {
    use claw_server::SessionManager;
    use dashmap::DashMap;
    use tokio::sync::{mpsc, oneshot};

    let manager = SessionManager::new(10);
    let (tx, _rx) = mpsc::channel(10);

    let pending: Arc<DashMap<String, oneshot::Sender<(serde_json::Value, bool)>>> =
        Arc::new(DashMap::new());

    let agent_loop = AgentLoopBuilder::new()
        .with_provider(MockProvider::new("ok"))
        .build()
        .unwrap();

    let (result_tx, result_rx) = oneshot::channel::<(serde_json::Value, bool)>();
    pending.insert("call-42".to_string(), result_tx);

    let session = manager
        .create_with_id(
            "session-test".to_string(),
            tx,
            agent_loop,
            Arc::clone(&pending),
        )
        .unwrap();

    // Simulate handle_tool_result routing a response.
    if let Some((_, sender)) = session.pending_tool_calls.remove("call-42") {
        let _ = sender.send((json!({"output": "the result"}), true));
    }

    let (result, success) = result_rx.await.unwrap();
    assert!(success);
    assert_eq!(result["output"], "the result");
}

// ── Test: ExternalToolBridge notification sends correctly ─────────────────────

#[tokio::test]
async fn test_external_tool_bridge_notification() {
    use claw_server::tool_bridge::ExternalToolBridge;
    use claw_tools::{traits::Tool, types::{PermissionSet, ToolContext}};
    use dashmap::DashMap;
    use tokio::sync::{mpsc, oneshot};

    let (notify_tx, mut notify_rx) = mpsc::channel::<Vec<u8>>(10);
    let pending: Arc<DashMap<String, oneshot::Sender<(serde_json::Value, bool)>>> =
        Arc::new(DashMap::new());

    let bridge = ExternalToolBridge::new(
        "get_weather",
        "Get weather",
        json!({"type": "object", "properties": {"city": {"type": "string"}}}),
        PermissionSet::minimal(),
        "session-abc",
        notify_tx,
        Arc::clone(&pending),
        claw_tools::audit::AuditLogWriterHandle::noop(),
    );

    assert_eq!(bridge.name(), "get_weather");
    assert_eq!(bridge.description(), "Get weather");

    // Spawn the execute call so it sends the notification and then waits.
    let ctx = ToolContext::new("agent-1", PermissionSet::minimal());
    let execute_handle = tokio::spawn(async move {
        bridge.execute(json!({"city": "Shanghai"}), &ctx).await
    });

    // The bridge should have sent a notification to notify_rx.
    let notification_bytes = timeout(Duration::from_secs(3), notify_rx.recv())
        .await
        .expect("should receive notification")
        .expect("channel should not be closed");

    let notification: Value = serde_json::from_slice(&notification_bytes).unwrap();
    assert_eq!(notification["method"], "agent/toolCall");
    assert_eq!(notification["params"]["tool_name"], "get_weather");
    assert_eq!(notification["params"]["arguments"]["city"], "Shanghai");

    let tool_call_id = notification["params"]["tool_call_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Simulate client sending toolResult back by writing to the pending map.
    if let Some((_, sender)) = pending.remove(&tool_call_id) {
        let _ = sender.send((json!({"temperature": 22}), true));
    }

    let tool_result = execute_handle.await.unwrap();
    assert!(tool_result.success);
    assert_eq!(tool_result.output.unwrap()["temperature"], 22);
}

// ── Test: kernel.ping ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_kernel_ping() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("test_ping.sock");
    let path_str = socket_path.to_str().unwrap().to_string();

    let (_server, token) = start_server(&path_str).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client = UnixStream::connect(&path_str).await.unwrap();
    authenticate(&mut client, &token).await;

    let resp = rpc(&mut client, "kernel.ping", None, 1).await;

    assert!(resp["error"].is_null(), "kernel.ping should not error: {:?}", resp);
    assert_eq!(resp["result"]["pong"], true);
    assert!(resp["result"]["ts"].is_number());
}

// ── Test: kernel.info ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_kernel_info() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("test_info.sock");
    let path_str = socket_path.to_str().unwrap().to_string();

    let (_server, token) = start_server(&path_str).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client = UnixStream::connect(&path_str).await.unwrap();
    authenticate(&mut client, &token).await;

    let resp = rpc(&mut client, "kernel.info", None, 2).await;

    assert!(resp["error"].is_null(), "kernel.info failed: {:?}", resp);
    let result = &resp["result"];
    assert!(result["version"].is_string(), "version should be string");
    assert_eq!(result["protocol_version"], 2);
    assert!(result["providers"].is_array(), "providers should be array");
}

// ── Test: session.create and session.close ────────────────────────────────────

#[tokio::test]
async fn test_session_create_and_close() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("test_session_rpc.sock");
    let path_str = socket_path.to_str().unwrap().to_string();

    let (_server, token) = start_server(&path_str).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client = UnixStream::connect(&path_str).await.unwrap();
    authenticate(&mut client, &token).await;

    // Create session via new method name
    let resp = rpc(
        &mut client,
        "session.create",
        Some(json!({
            "system_prompt": "You are a test agent.",
        })),
        3,
    )
    .await;

    assert!(resp["error"].is_null(), "session.create failed: {:?}", resp);
    let session_id = resp["result"]["session_id"].as_str().unwrap().to_string();
    assert!(!session_id.is_empty());

    // Close session
    let resp2 = rpc(
        &mut client,
        "session.close",
        Some(json!({ "session_id": session_id })),
        4,
    )
    .await;

    assert!(resp2["error"].is_null(), "session.close failed: {:?}", resp2);
    assert_eq!(resp2["result"]["closed"], true);
}

// ── Test: provider.list ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_provider_list() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("test_providers.sock");
    let path_str = socket_path.to_str().unwrap().to_string();

    let (_server, token) = start_server(&path_str).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut client = UnixStream::connect(&path_str).await.unwrap();
    authenticate(&mut client, &token).await;

    let resp = rpc(&mut client, "provider.list", None, 5).await;

    assert!(resp["error"].is_null(), "provider.list failed: {:?}", resp);
    assert!(resp["result"]["providers"].is_array());
    assert!(resp["result"]["default"].is_string());
}
