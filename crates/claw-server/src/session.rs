//! Session management for KernelServer.
//!
//! Manages agent sessions with notification channels and tool result handling.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

use crate::error::ServerError;

/// A single agent session.
///
/// Each session has:
/// - A unique ID
/// - A notification channel for streaming responses to the client
/// - Channels for tool result communication
pub struct Session {
    /// Unique session identifier.
    pub id: String,
    /// Channel for sending notifications to the client.
    pub notify_tx: mpsc::Sender<Vec<u8>>,
    /// Channel for sending tool results from the client.
    pub tool_result_tx: mpsc::Sender<(String, serde_json::Value, bool)>,
    /// Channel for receiving tool results from the client.
    pub tool_result_rx: Mutex<mpsc::Receiver<(String, serde_json::Value, bool)>>,
}

impl Session {
    /// Creates a new session with the given ID and notification channel.
    pub fn new(id: String, notify_tx: mpsc::Sender<Vec<u8>>) -> Self {
        let (tx, rx) = mpsc::channel(32);
        Self {
            id,
            notify_tx,
            tool_result_tx: tx,
            tool_result_rx: Mutex::new(rx),
        }
    }

    /// Sends a notification to the client.
    pub async fn notify(&self, data: Vec<u8>) -> Result<(), ServerError> {
        self.notify_tx
            .send(data)
            .await
            .map_err(|_| ServerError::Ipc(claw_pal::error::IpcError::BrokenPipe))
    }

    /// Sends a tool result to the agent loop.
    pub async fn send_tool_result(
        &self,
        tool_call_id: String,
        result: serde_json::Value,
        success: bool,
    ) -> Result<(), ServerError> {
        self.tool_result_tx
            .send((tool_call_id, result, success))
            .await
            .map_err(|_| ServerError::Agent("tool result channel closed".to_string()))
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
    /// Maximum number of allowed sessions.
    max_sessions: usize,
}

impl SessionManager {
    /// Creates a new session manager with the given maximum sessions.
    pub fn new(max_sessions: usize) -> Self {
        Self {
            sessions: DashMap::new(),
            max_sessions,
        }
    }

    /// Creates a new session and returns it.
    ///
    /// Returns an error if the maximum number of sessions is reached.
    pub fn create(&self, notify_tx: mpsc::Sender<Vec<u8>>) -> Result<Arc<Session>, ServerError> {
        if self.sessions.len() >= self.max_sessions {
            return Err(ServerError::MaxSessionsReached {
                max: self.max_sessions,
            });
        }

        let session_id = Uuid::new_v4().to_string();
        let session = Arc::new(Session::new(session_id.clone(), notify_tx));

        self.sessions.insert(session_id, Arc::clone(&session));

        Ok(session)
    }

    /// Gets a session by ID.
    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.get(id).map(|entry| Arc::clone(entry.value()))
    }

    /// Removes a session by ID.
    ///
    /// Returns true if a session was removed.
    pub fn remove(&self, id: &str) -> bool {
        self.sessions.remove(id).is_some()
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
            .field("max_sessions", &self.max_sessions)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_creation() {
        let (tx, _rx) = mpsc::channel(10);
        let session = Session::new("test-session".to_string(), tx);

        assert_eq!(session.id, "test-session");
    }

    #[tokio::test]
    async fn test_session_notify() {
        let (tx, mut rx) = mpsc::channel(10);
        let session = Session::new("test-session".to_string(), tx);

        session.notify(b"hello".to_vec()).await.unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received, b"hello");
    }

    #[tokio::test]
    async fn test_session_tool_result() {
        let (tx, _rx) = mpsc::channel(10);
        let session = Session::new("test-session".to_string(), tx);

        session
            .send_tool_result("call-1".to_string(), serde_json::json!("result"), true)
            .await
            .unwrap();

        let mut rx = session.tool_result_rx.lock().await;
        let (id, result, success) = rx.recv().await.unwrap();
        assert_eq!(id, "call-1");
        assert_eq!(result, serde_json::json!("result"));
        assert!(success);
    }

    #[test]
    fn test_session_manager_create() {
        let manager = SessionManager::new(10);
        let (tx, _rx) = mpsc::channel(10);

        let session = manager.create(tx).unwrap();
        assert_eq!(manager.count(), 1);
        assert!(manager.get(&session.id).is_some());
    }

    #[test]
    fn test_session_manager_max_sessions() {
        let manager = SessionManager::new(2);

        let (tx1, _rx1) = mpsc::channel(10);
        let (tx2, _rx2) = mpsc::channel(10);
        let (tx3, _rx3) = mpsc::channel(10);

        manager.create(tx1).unwrap();
        manager.create(tx2).unwrap();

        let result = manager.create(tx3);
        assert!(matches!(
            result,
            Err(ServerError::MaxSessionsReached { max: 2 })
        ));
    }

    #[test]
    fn test_session_manager_get() {
        let manager = SessionManager::new(10);
        let (tx, _rx) = mpsc::channel(10);

        let session = manager.create(tx).unwrap();
        let id = session.id.clone();

        assert!(manager.get(&id).is_some());
        assert!(manager.get("nonexistent").is_none());
    }

    #[test]
    fn test_session_manager_remove() {
        let manager = SessionManager::new(10);
        let (tx, _rx) = mpsc::channel(10);

        let session = manager.create(tx).unwrap();
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

        manager.create(tx1).unwrap();
        manager.create(tx2).unwrap();

        manager.clear();
        assert_eq!(manager.count(), 0);
    }

    #[test]
    fn test_session_manager_default() {
        let manager: SessionManager = Default::default();
        assert_eq!(manager.max_sessions(), 100);
    }
}
