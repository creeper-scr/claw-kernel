//! Session management for KernelServer.
//!
//! Manages agent sessions with notification channels and external tool bridges.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use claw_runtime::agent_types::AgentId;

use crate::error::ServerError;

/// A single agent session.
///
/// Each session has:
/// - A unique ID
/// - A notification channel for streaming responses to the client
/// - A pending tool calls map for external tool bridge communication
/// - An agent loop (wrapped in Mutex for async exclusive access)
/// - An optional AgentId when this session backs a spawned agent
pub struct Session {
    /// Unique session identifier.
    pub id: String,
    /// If this session was created by `agent.spawn`, the corresponding AgentId.
    pub agent_id: Option<AgentId>,
    /// Channel for sending notifications to the client.
    pub notify_tx: mpsc::Sender<Vec<u8>>,
    /// Pending external tool calls waiting for client response.
    /// Key: tool_call_id, Value: oneshot sender to deliver the result.
    pub pending_tool_calls: Arc<DashMap<String, oneshot::Sender<(serde_json::Value, bool)>>>,
    /// The agent loop for this session (wrapped in Mutex for async exclusive access).
    pub agent_loop: tokio::sync::Mutex<claw_loop::AgentLoop>,
    /// Handle for the background event-forwarding task, if subscribed.
    pub event_forwarder: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// IDs of scheduled tasks created in this session.
    pub scheduled_task_ids: tokio::sync::Mutex<Vec<String>>,
}

impl Session {
    /// Creates a new session with the given ID, notification channel, and agent loop.
    pub fn new(
        id: String,
        notify_tx: mpsc::Sender<Vec<u8>>,
        agent_loop: claw_loop::AgentLoop,
    ) -> Self {
        Self {
            id,
            agent_id: None,
            notify_tx,
            pending_tool_calls: Arc::new(DashMap::new()),
            agent_loop: tokio::sync::Mutex::new(agent_loop),
            event_forwarder: tokio::sync::Mutex::new(None),
            scheduled_task_ids: tokio::sync::Mutex::new(vec![]),
        }
    }

    /// Creates a new session with pre-built pending_tool_calls map.
    ///
    /// Used when ExternalToolBridge instances have already been created and
    /// share the same pending_tool_calls map (to avoid chicken-and-egg problems).
    pub fn new_with_pending(
        id: String,
        notify_tx: mpsc::Sender<Vec<u8>>,
        agent_loop: claw_loop::AgentLoop,
        pending_tool_calls: Arc<DashMap<String, oneshot::Sender<(serde_json::Value, bool)>>>,
    ) -> Self {
        Self {
            id,
            agent_id: None,
            notify_tx,
            pending_tool_calls,
            agent_loop: tokio::sync::Mutex::new(agent_loop),
            event_forwarder: tokio::sync::Mutex::new(None),
            scheduled_task_ids: tokio::sync::Mutex::new(vec![]),
        }
    }

    /// Creates a new session backed by an agent.spawn call, binding it to the given AgentId.
    pub fn new_for_agent(
        id: String,
        agent_id: AgentId,
        notify_tx: mpsc::Sender<Vec<u8>>,
        agent_loop: claw_loop::AgentLoop,
    ) -> Self {
        Self {
            id,
            agent_id: Some(agent_id),
            notify_tx,
            pending_tool_calls: Arc::new(DashMap::new()),
            agent_loop: tokio::sync::Mutex::new(agent_loop),
            event_forwarder: tokio::sync::Mutex::new(None),
            scheduled_task_ids: tokio::sync::Mutex::new(vec![]),
        }
    }

    /// Sends a notification to the client.
    pub async fn notify(&self, data: Vec<u8>) -> Result<(), ServerError> {
        self.notify_tx
            .send(data)
            .await
            .map_err(|_| ServerError::Ipc(claw_pal::error::IpcError::BrokenPipe))
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// Manages all active sessions.
pub struct SessionManager {
    /// Active sessions mapped by ID.
    sessions: DashMap<String, Arc<Session>>,
    /// Reverse index: AgentId → session_id.
    ///
    /// Populated when `agent.spawn` creates a session backed by an AgentId.
    agent_to_session: DashMap<AgentId, String>,
    /// Maximum number of allowed sessions.
    max_sessions: usize,
}

impl SessionManager {
    /// Creates a new session manager with the given maximum sessions.
    pub fn new(max_sessions: usize) -> Self {
        Self {
            sessions: DashMap::new(),
            agent_to_session: DashMap::new(),
            max_sessions,
        }
    }

    /// Creates a new session and returns it.
    ///
    /// Returns an error if the maximum number of sessions is reached.
    pub fn create(
        &self,
        notify_tx: mpsc::Sender<Vec<u8>>,
        agent_loop: claw_loop::AgentLoop,
    ) -> Result<Arc<Session>, ServerError> {
        if self.sessions.len() >= self.max_sessions {
            return Err(ServerError::MaxSessionsReached {
                max: self.max_sessions,
            });
        }

        let session_id = Uuid::new_v4().to_string();
        let session = Arc::new(Session::new(session_id.clone(), notify_tx, agent_loop));

        self.sessions.insert(session_id, Arc::clone(&session));

        Ok(session)
    }

    /// Creates a new session with a pre-specified ID and pre-built components.
    ///
    /// Used when ExternalToolBridge instances need to share the pending_tool_calls
    /// map before the session is created (to avoid circular dependency).
    pub fn create_with_id(
        &self,
        id: String,
        notify_tx: mpsc::Sender<Vec<u8>>,
        agent_loop: claw_loop::AgentLoop,
        pending_tool_calls: Arc<DashMap<String, oneshot::Sender<(serde_json::Value, bool)>>>,
    ) -> Result<Arc<Session>, ServerError> {
        if self.sessions.len() >= self.max_sessions {
            return Err(ServerError::MaxSessionsReached {
                max: self.max_sessions,
            });
        }

        let session = Arc::new(Session::new_with_pending(
            id.clone(),
            notify_tx,
            agent_loop,
            pending_tool_calls,
        ));
        self.sessions.insert(id, Arc::clone(&session));

        Ok(session)
    }

    /// Creates a session that backs a spawned agent and registers the AgentId→SessionId index.
    ///
    /// Called by `agent.spawn` so that `agent.steer` and `agent.kill` can locate the
    /// real running `AgentLoop` via the `AgentId`.
    pub fn create_for_agent(
        &self,
        agent_id: AgentId,
        notify_tx: mpsc::Sender<Vec<u8>>,
        agent_loop: claw_loop::AgentLoop,
    ) -> Result<Arc<Session>, ServerError> {
        if self.sessions.len() >= self.max_sessions {
            return Err(ServerError::MaxSessionsReached {
                max: self.max_sessions,
            });
        }

        let session_id = Uuid::new_v4().to_string();
        let session = Arc::new(Session::new_for_agent(
            session_id.clone(),
            agent_id.clone(),
            notify_tx,
            agent_loop,
        ));
        self.sessions.insert(session_id.clone(), Arc::clone(&session));
        self.agent_to_session.insert(agent_id, session_id);

        Ok(session)
    }

    /// Returns the session_id bound to the given AgentId, if any.
    pub fn session_for_agent(&self, agent_id: &AgentId) -> Option<String> {
        self.agent_to_session
            .get(agent_id)
            .map(|entry| entry.value().clone())
    }

    /// Gets a session by ID.
    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.get(id).map(|entry| Arc::clone(entry.value()))
    }

    /// Removes a session by ID, cleaning up the AgentId→SessionId index if applicable.
    ///
    /// Returns true if a session was removed.
    pub fn remove(&self, id: &str) -> bool {
        if let Some((_, session)) = self.sessions.remove(id) {
            if let Some(ref aid) = session.agent_id {
                self.agent_to_session.remove(aid);
            }
            true
        } else {
            false
        }
    }

    /// Removes a session by AgentId.
    ///
    /// Looks up the session_id from the reverse index, then removes both.
    /// Returns the removed session_id on success.
    pub fn remove_by_agent(&self, agent_id: &AgentId) -> Option<String> {
        let session_id = self.agent_to_session.remove(agent_id)?.1;
        self.sessions.remove(&session_id);
        Some(session_id)
    }

    /// Returns the current number of active sessions.
    pub fn count(&self) -> usize {
        self.sessions.len()
    }

    /// Returns the maximum number of allowed sessions.
    pub fn max_sessions(&self) -> usize {
        self.max_sessions
    }

    /// Clears all sessions.
    pub fn clear(&self) {
        self.sessions.clear();
        self.agent_to_session.clear();
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new(100)
    }
}

impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager")
            .field("session_count", &self.sessions.len())
            .field("agent_count", &self.agent_to_session.len())
            .field("max_sessions", &self.max_sessions)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use claw_loop::AgentLoopBuilder;
    use claw_provider::{
        error::ProviderError,
        traits::LLMProvider,
        types::{CompletionResponse, Delta, FinishReason, Message, Options, TokenUsage},
    };
    use futures::stream;
    use std::pin::Pin;

    /// Mock LLM provider for testing (does not call any external API).
    struct MockProvider;

    #[async_trait]
    impl LLMProvider for MockProvider {
        fn provider_id(&self) -> &str {
            "mock"
        }
        fn model_id(&self) -> &str {
            "mock-v1"
        }

        async fn complete(
            &self,
            _messages: Vec<Message>,
            _opts: Options,
        ) -> Result<CompletionResponse, ProviderError> {
            Ok(CompletionResponse {
                id: "id".to_string(),
                model: "mock-v1".to_string(),
                message: Message::assistant("ok"),
                finish_reason: FinishReason::Stop,
                usage: TokenUsage {
                    prompt_tokens: 5,
                    completion_tokens: 3,
                    total_tokens: 8,
                },
            })
        }

        async fn complete_stream(
            &self,
            _messages: Vec<Message>,
            _opts: Options,
        ) -> Result<
            Pin<Box<dyn futures::Stream<Item = Result<Delta, ProviderError>> + Send>>,
            ProviderError,
        > {
            Ok(Box::pin(stream::empty()))
        }
    }

    fn make_agent_loop() -> claw_loop::AgentLoop {
        AgentLoopBuilder::new()
            .with_provider(Arc::new(MockProvider))
            .build()
            .expect("build should succeed")
    }

    #[tokio::test]
    async fn test_session_creation() {
        let (tx, _rx) = mpsc::channel(10);
        let agent_loop = make_agent_loop();
        let session = Session::new("test-session".to_string(), tx, agent_loop);

        assert_eq!(session.id, "test-session");
    }

    #[tokio::test]
    async fn test_session_notify() {
        let (tx, mut rx) = mpsc::channel(10);
        let agent_loop = make_agent_loop();
        let session = Session::new("test-session".to_string(), tx, agent_loop);

        session.notify(b"hello".to_vec()).await.unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received, b"hello");
    }

    #[test]
    fn test_session_manager_create() {
        let manager = SessionManager::new(10);
        let (tx, _rx) = mpsc::channel(10);
        let agent_loop = make_agent_loop();

        let session = manager.create(tx, agent_loop).unwrap();
        assert_eq!(manager.count(), 1);
        assert!(manager.get(&session.id).is_some());
    }

    #[test]
    fn test_session_manager_max_sessions() {
        let manager = SessionManager::new(2);

        let (tx1, _rx1) = mpsc::channel(10);
        let (tx2, _rx2) = mpsc::channel(10);
        let (tx3, _rx3) = mpsc::channel(10);

        manager.create(tx1, make_agent_loop()).unwrap();
        manager.create(tx2, make_agent_loop()).unwrap();

        let result = manager.create(tx3, make_agent_loop());
        assert!(matches!(
            result,
            Err(ServerError::MaxSessionsReached { max: 2 })
        ));
    }

    #[test]
    fn test_session_manager_get() {
        let manager = SessionManager::new(10);
        let (tx, _rx) = mpsc::channel(10);

        let session = manager.create(tx, make_agent_loop()).unwrap();
        let id = session.id.clone();

        assert!(manager.get(&id).is_some());
        assert!(manager.get("nonexistent").is_none());
    }

    #[test]
    fn test_session_manager_remove() {
        let manager = SessionManager::new(10);
        let (tx, _rx) = mpsc::channel(10);

        let session = manager.create(tx, make_agent_loop()).unwrap();
        let id = session.id.clone();

        assert!(manager.remove(&id));
        assert_eq!(manager.count(), 0);
        assert!(!manager.remove(&id));
    }

    #[test]
    fn test_session_manager_clear() {
        let manager = SessionManager::new(10);
        let (tx1, _rx1) = mpsc::channel(10);
        let (tx2, _rx2) = mpsc::channel(10);

        manager.create(tx1, make_agent_loop()).unwrap();
        manager.create(tx2, make_agent_loop()).unwrap();

        manager.clear();
        assert_eq!(manager.count(), 0);
    }

    #[test]
    fn test_session_manager_default() {
        let manager: SessionManager = Default::default();
        assert_eq!(manager.max_sessions(), 100);
    }

    #[test]
    fn test_session_manager_create_with_id() {
        let manager = SessionManager::new(10);
        let (tx, _rx) = mpsc::channel(10);
        let pending: Arc<DashMap<String, oneshot::Sender<(serde_json::Value, bool)>>> =
            Arc::new(DashMap::new());

        let session = manager
            .create_with_id(
                "fixed-id".to_string(),
                tx,
                make_agent_loop(),
                Arc::clone(&pending),
            )
            .unwrap();

        assert_eq!(session.id, "fixed-id");
        assert_eq!(manager.count(), 1);
    }

    #[test]
    fn test_session_pending_tool_calls_shared() {
        let manager = SessionManager::new(10);
        let (tx, _rx) = mpsc::channel(10);
        let pending: Arc<DashMap<String, oneshot::Sender<(serde_json::Value, bool)>>> =
            Arc::new(DashMap::new());

        // Insert a dummy sender into the shared pending map before session creation
        let (sender, _receiver) = oneshot::channel::<(serde_json::Value, bool)>();
        pending.insert("tool-call-1".to_string(), sender);

        let session = manager
            .create_with_id(
                "test-id".to_string(),
                tx,
                make_agent_loop(),
                Arc::clone(&pending),
            )
            .unwrap();

        // Session should share the same pending map
        assert!(session.pending_tool_calls.contains_key("tool-call-1"));
    }
}
