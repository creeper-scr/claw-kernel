//! IPC transport trait for cross-process communication.
//!
//! Provides the core trait for implementing IPC transports (Unix Domain Sockets,
//! Named Pipes, etc.) and a mock implementation for testing.

use crate::error::IpcError;
use crate::types::ipc::{IpcConnection, IpcListener};

/// IPC transport trait for sending and receiving messages across processes.
///
/// Implementations must be thread-safe and async-compatible.
#[async_trait::async_trait]
pub trait IpcTransport: Send + Sync {
    /// Connect to an IPC endpoint.
    ///
    /// # Arguments
    /// * `endpoint` - The endpoint string (e.g., "/tmp/socket" for Unix, "\\.\pipe\name" for Windows)
    ///
    /// # Returns
    /// A connected IpcConnection or an IpcError
    async fn connect(endpoint: &str) -> Result<IpcConnection, IpcError>;

    /// Listen on an IPC endpoint.
    ///
    /// # Arguments
    /// * `endpoint` - The endpoint string to listen on
    ///
    /// # Returns
    /// An IpcListener or an IpcError
    async fn listen(endpoint: &str) -> Result<IpcListener, IpcError>;

    /// Send a message through the connection.
    ///
    /// # Arguments
    /// * `msg` - The message bytes to send
    ///
    /// # Returns
    /// Ok(()) on success, IpcError on failure
    async fn send(&self, msg: &[u8]) -> Result<(), IpcError>;

    /// Receive a message from the connection.
    ///
    /// # Returns
    /// The received message bytes or an IpcError
    async fn recv(&self) -> Result<Vec<u8>, IpcError>;
}

/// Mock IPC transport for testing.
///
/// This implementation allows testing code that uses IpcTransport
/// without requiring actual IPC infrastructure.
#[derive(Debug, Clone)]
pub struct MockIpcTransport {
    endpoint: String,
    is_listener: bool,
}

impl MockIpcTransport {
    /// Create a new mock transport.
    pub fn new(endpoint: String) -> Self {
        Self {
            endpoint,
            is_listener: false,
        }
    }

    /// Create a mock listener.
    pub fn listener(endpoint: String) -> Self {
        Self {
            endpoint,
            is_listener: true,
        }
    }
}

#[async_trait::async_trait]
impl IpcTransport for MockIpcTransport {
    async fn connect(endpoint: &str) -> Result<IpcConnection, IpcError> {
        if endpoint.is_empty() {
            return Err(IpcError::InvalidMessage);
        }
        Ok(IpcConnection::new(endpoint.to_string()))
    }

    async fn listen(endpoint: &str) -> Result<IpcListener, IpcError> {
        if endpoint.is_empty() {
            return Err(IpcError::InvalidMessage);
        }
        Ok(IpcListener::new(endpoint.to_string()))
    }

    async fn send(&self, msg: &[u8]) -> Result<(), IpcError> {
        if msg.is_empty() {
            return Err(IpcError::InvalidMessage);
        }
        Ok(())
    }

    async fn recv(&self) -> Result<Vec<u8>, IpcError> {
        Ok(vec![1, 2, 3, 4])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_ipc_transport_connect() {
        let conn = MockIpcTransport::connect("/tmp/test.sock").await;
        assert!(conn.is_ok());
        let conn = conn.unwrap();
        assert_eq!(conn.endpoint, "/tmp/test.sock");
    }

    #[tokio::test]
    async fn test_mock_ipc_transport_connect_empty_endpoint() {
        let result = MockIpcTransport::connect("").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), IpcError::InvalidMessage);
    }

    #[tokio::test]
    async fn test_mock_ipc_transport_listen() {
        let listener = MockIpcTransport::listen("/tmp/test.sock").await;
        assert!(listener.is_ok());
        let listener = listener.unwrap();
        assert_eq!(listener.endpoint, "/tmp/test.sock");
    }

    #[tokio::test]
    async fn test_mock_ipc_transport_listen_empty_endpoint() {
        let result = MockIpcTransport::listen("").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), IpcError::InvalidMessage);
    }

    #[tokio::test]
    async fn test_mock_ipc_transport_send() {
        let transport = MockIpcTransport::new("/tmp/test.sock".to_string());
        let result = transport.send(b"hello").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_mock_ipc_transport_send_empty_message() {
        let transport = MockIpcTransport::new("/tmp/test.sock".to_string());
        let result = transport.send(b"").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), IpcError::InvalidMessage);
    }

    #[tokio::test]
    async fn test_mock_ipc_transport_recv() {
        let transport = MockIpcTransport::new("/tmp/test.sock".to_string());
        let result = transport.recv().await;
        assert!(result.is_ok());
        let msg = result.unwrap();
        assert_eq!(msg, vec![1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn test_mock_ipc_transport_clone() {
        let transport = MockIpcTransport::new("/tmp/test.sock".to_string());
        let cloned = transport.clone();
        assert_eq!(transport.endpoint, cloned.endpoint);
        assert_eq!(transport.is_listener, cloned.is_listener);
    }

    #[tokio::test]
    async fn test_mock_ipc_transport_debug() {
        let transport = MockIpcTransport::new("/tmp/test.sock".to_string());
        let debug_str = format!("{:?}", transport);
        assert!(debug_str.contains("MockIpcTransport"));
    }
}
