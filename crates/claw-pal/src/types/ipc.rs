//! IPC types for cross-process communication.
//!
//! Provides core types for IPC communication including messages, connections, and endpoints.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// IPC message structure.
///
/// Represents a message sent over IPC with metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpcMessage {
    /// Unique message identifier.
    pub id: u64,
    /// Message payload.
    pub payload: Vec<u8>,
    /// Timestamp in milliseconds since epoch.
    pub timestamp: u64,
}

impl IpcMessage {
    /// Create a new IPC message.
    pub fn new(id: u64, payload: Vec<u8>, timestamp: u64) -> Self {
        Self {
            id,
            payload,
            timestamp,
        }
    }
}

/// IPC connection representing an established connection to an endpoint.
///
/// This is a handle to an active IPC connection that can be used to send/receive messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpcConnection {
    /// The endpoint this connection is connected to.
    pub endpoint: String,
}

impl IpcConnection {
    /// Create a new IPC connection.
    pub fn new(endpoint: String) -> Self {
        Self { endpoint }
    }
}

/// IPC listener representing a listening endpoint.
///
/// This is a handle to an endpoint that is listening for incoming connections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpcListener {
    /// The endpoint this listener is listening on.
    pub endpoint: String,
}

impl IpcListener {
    /// Create a new IPC listener.
    pub fn new(endpoint: String) -> Self {
        Self { endpoint }
    }
}

/// IPC endpoint specification.
///
/// Represents different types of IPC endpoints supported by the platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IpcEndpoint {
    /// Unix Domain Socket endpoint (Linux/macOS).
    UnixSocket(PathBuf),
    /// Named Pipe endpoint (Windows).
    NamedPipe(String),
}

impl IpcEndpoint {
    /// Get the endpoint as a string.
    pub fn as_str(&self) -> String {
        match self {
            IpcEndpoint::UnixSocket(path) => path.to_string_lossy().to_string(),
            IpcEndpoint::NamedPipe(name) => name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipc_message_new() {
        let msg = IpcMessage::new(1, vec![1, 2, 3], 1000);
        assert_eq!(msg.id, 1);
        assert_eq!(msg.payload, vec![1, 2, 3]);
        assert_eq!(msg.timestamp, 1000);
    }

    #[test]
    fn test_ipc_message_clone() {
        let msg = IpcMessage::new(1, vec![1, 2, 3], 1000);
        let cloned = msg.clone();
        assert_eq!(msg, cloned);
    }

    #[test]
    fn test_ipc_message_serialize() {
        let msg = IpcMessage::new(1, vec![1, 2, 3], 1000);
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: IpcMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, deserialized);
    }

    #[test]
    fn test_ipc_connection_new() {
        let conn = IpcConnection::new("/tmp/test.sock".to_string());
        assert_eq!(conn.endpoint, "/tmp/test.sock");
    }

    #[test]
    fn test_ipc_connection_clone() {
        let conn = IpcConnection::new("/tmp/test.sock".to_string());
        let cloned = conn.clone();
        assert_eq!(conn, cloned);
    }

    #[test]
    fn test_ipc_connection_serialize() {
        let conn = IpcConnection::new("/tmp/test.sock".to_string());
        let json = serde_json::to_string(&conn).unwrap();
        let deserialized: IpcConnection = serde_json::from_str(&json).unwrap();
        assert_eq!(conn, deserialized);
    }

    #[test]
    fn test_ipc_listener_new() {
        let listener = IpcListener::new("/tmp/test.sock".to_string());
        assert_eq!(listener.endpoint, "/tmp/test.sock");
    }

    #[test]
    fn test_ipc_listener_clone() {
        let listener = IpcListener::new("/tmp/test.sock".to_string());
        let cloned = listener.clone();
        assert_eq!(listener, cloned);
    }

    #[test]
    fn test_ipc_listener_serialize() {
        let listener = IpcListener::new("/tmp/test.sock".to_string());
        let json = serde_json::to_string(&listener).unwrap();
        let deserialized: IpcListener = serde_json::from_str(&json).unwrap();
        assert_eq!(listener, deserialized);
    }

    #[test]
    fn test_ipc_endpoint_unix_socket() {
        let endpoint = IpcEndpoint::UnixSocket(PathBuf::from("/tmp/test.sock"));
        assert_eq!(endpoint.as_str(), "/tmp/test.sock");
    }

    #[test]
    fn test_ipc_endpoint_named_pipe() {
        let endpoint = IpcEndpoint::NamedPipe("test_pipe".to_string());
        assert_eq!(endpoint.as_str(), "test_pipe");
    }

    #[test]
    fn test_ipc_endpoint_clone() {
        let endpoint = IpcEndpoint::UnixSocket(PathBuf::from("/tmp/test.sock"));
        let cloned = endpoint.clone();
        assert_eq!(endpoint, cloned);
    }

    #[test]
    fn test_ipc_endpoint_serialize_unix_socket() {
        let endpoint = IpcEndpoint::UnixSocket(PathBuf::from("/tmp/test.sock"));
        let json = serde_json::to_string(&endpoint).unwrap();
        let deserialized: IpcEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(endpoint, deserialized);
    }

    #[test]
    fn test_ipc_endpoint_serialize_named_pipe() {
        let endpoint = IpcEndpoint::NamedPipe("test_pipe".to_string());
        let json = serde_json::to_string(&endpoint).unwrap();
        let deserialized: IpcEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(endpoint, deserialized);
    }

    #[test]
    fn test_ipc_endpoint_debug() {
        let endpoint = IpcEndpoint::UnixSocket(PathBuf::from("/tmp/test.sock"));
        let debug_str = format!("{:?}", endpoint);
        assert!(debug_str.contains("UnixSocket"));
    }
}
