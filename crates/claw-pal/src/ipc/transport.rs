//! Core IPC transport backed by `interprocess` local sockets.
//!
//! Design: a single background reader task drains the socket and forwards
//! frames via an `mpsc` channel.  The write path is serialised through a
//! `Mutex<OwnedWriteHalf>`.  This deliberately avoids concurrent bi-directional
//! split I/O on the same socket (which panics on macOS with interprocess 1.2.1).

#[cfg(not(windows))]
use interprocess::local_socket::tokio::{LocalSocketListener, LocalSocketStream};

use tokio::sync::{mpsc, Mutex};

use crate::{
    error::IpcError,
    ipc::framing::{read_frame, write_frame},
    traits::ipc::IpcTransport,
    types::ipc::{IpcConnection, IpcListener},
};

// On non-Windows platforms the write half is interprocess's OwnedWriteHalf.
#[cfg(not(windows))]
use interprocess::local_socket::tokio::OwnedWriteHalf;

/// IPC transport backed by a local socket (Unix Domain Socket on Unix,
/// Named Pipe on Windows – Windows support is future work).
///
/// A dedicated reader task continuously reads frames from the socket and
/// sends them into an internal `mpsc` channel.  Callers receive frames by
/// awaiting [`recv`].  Writes are serialised through a `Mutex`.
#[cfg(not(windows))]
pub struct InterprocessTransport {
    writer: Mutex<OwnedWriteHalf>,
    recv_rx: Mutex<mpsc::Receiver<Result<Vec<u8>, IpcError>>>,
    /// Keeps the reader task alive for the lifetime of this transport.
    _reader_task: tokio::task::JoinHandle<()>,
}

#[cfg(not(windows))]
impl std::fmt::Debug for InterprocessTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InterprocessTransport").finish_non_exhaustive()
    }
}

#[cfg(not(windows))]
impl InterprocessTransport {
    /// Internal constructor: split `stream`, spawn reader task, return Self.
    fn from_stream(stream: LocalSocketStream) -> Self {
        let (mut read_half, write_half) = stream.into_split();
        let (tx, rx) = mpsc::channel::<Result<Vec<u8>, IpcError>>(128);
        let handle = tokio::spawn(async move {
            loop {
                match read_frame(&mut read_half).await {
                    Ok(frame) => {
                        if tx.send(Ok(frame)).await.is_err() {
                            // Receiver dropped; exit reader task.
                            break;
                        }
                    }
                    Err(e) => {
                        // Send the error then stop; the channel will be closed.
                        let _ = tx.send(Err(e)).await;
                        break;
                    }
                }
            }
        });
        Self {
            writer: Mutex::new(write_half),
            recv_rx: Mutex::new(rx),
            _reader_task: handle,
        }
    }

    /// Connect as a client to the given endpoint path.
    pub async fn new_client(endpoint: &str) -> Result<Self, IpcError> {
        let stream = LocalSocketStream::connect(endpoint)
            .await
            .map_err(|e| IpcError::ConnectionRefused)?;
        Ok(Self::from_stream(stream))
    }

    /// Bind a listener, accept exactly one incoming connection.
    pub async fn new_server(endpoint: &str) -> Result<Self, IpcError> {
        let listener = LocalSocketListener::bind(endpoint)
            .map_err(|_| IpcError::ConnectionRefused)?;
        let stream = listener
            .accept()
            .await
            .map_err(|_| IpcError::ConnectionRefused)?;
        Ok(Self::from_stream(stream))
    }
}

#[cfg(not(windows))]
// SAFETY: OwnedWriteHalf and the mpsc types are Send; JoinHandle is Send.
unsafe impl Send for InterprocessTransport {}
#[cfg(not(windows))]
unsafe impl Sync for InterprocessTransport {}

#[cfg(not(windows))]
#[async_trait::async_trait]
impl IpcTransport for InterprocessTransport {
    /// Return metadata for a client connection endpoint.
    ///
    /// Does not establish an actual socket connection; use [`new_client`] for that.
    async fn connect(endpoint: &str) -> Result<IpcConnection, IpcError> {
        if endpoint.is_empty() {
            return Err(IpcError::InvalidMessage);
        }
        Ok(IpcConnection::new(endpoint.to_string()))
    }

    /// Return metadata for a listener endpoint.
    ///
    /// Does not bind an actual socket; use [`new_server`] for that.
    async fn listen(endpoint: &str) -> Result<IpcListener, IpcError> {
        if endpoint.is_empty() {
            return Err(IpcError::InvalidMessage);
        }
        Ok(IpcListener::new(endpoint.to_string()))
    }

    /// Send `msg` as a length-prefixed frame.
    ///
    /// Returns `IpcError::InvalidMessage` for an empty payload.
    async fn send(&self, msg: &[u8]) -> Result<(), IpcError> {
        if msg.is_empty() {
            return Err(IpcError::InvalidMessage);
        }
        let mut writer = self.writer.lock().await;
        write_frame(&mut *writer, msg).await
    }

    /// Receive the next frame from the reader task.
    ///
    /// Returns `IpcError::BrokenPipe` when the channel has been closed
    /// (i.e. the remote end disconnected).
    async fn recv(&self) -> Result<Vec<u8>, IpcError> {
        let mut rx = self.recv_rx.lock().await;
        rx.recv().await.ok_or(IpcError::BrokenPipe)?
    }
}

// ---------------------------------------------------------------------------
// Windows stub – Named Pipe support is future work.
// ---------------------------------------------------------------------------
#[cfg(windows)]
pub struct InterprocessTransport {
    _endpoint: String,
}

#[cfg(windows)]
impl InterprocessTransport {
    pub async fn new_client(_endpoint: &str) -> Result<Self, IpcError> {
        Err(IpcError::ConnectionRefused)
    }

    pub async fn new_server(_endpoint: &str) -> Result<Self, IpcError> {
        Err(IpcError::ConnectionRefused)
    }
}

#[cfg(windows)]
#[async_trait::async_trait]
impl IpcTransport for InterprocessTransport {
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
            Err(IpcError::InvalidMessage)
        } else {
            Err(IpcError::ConnectionRefused)
        }
    }

    async fn recv(&self) -> Result<Vec<u8>, IpcError> {
        Err(IpcError::ConnectionRefused)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Trait metadata tests (no socket required)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_interprocess_connect_returns_metadata() {
        let conn = InterprocessTransport::connect("/tmp/claw_test_meta.sock").await;
        assert!(conn.is_ok());
        let conn = conn.unwrap();
        assert_eq!(conn.endpoint, "/tmp/claw_test_meta.sock");
    }

    #[tokio::test]
    async fn test_interprocess_listen_returns_metadata() {
        let listener = InterprocessTransport::listen("/tmp/claw_test_listen_meta.sock").await;
        assert!(listener.is_ok());
        let listener = listener.unwrap();
        assert_eq!(listener.endpoint, "/tmp/claw_test_listen_meta.sock");
    }

    #[tokio::test]
    async fn test_connect_empty_endpoint_fails() {
        let result = InterprocessTransport::connect("").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), IpcError::InvalidMessage);
    }

    #[tokio::test]
    async fn test_listen_empty_endpoint_fails() {
        let result = InterprocessTransport::listen("").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), IpcError::InvalidMessage);
    }

    // ------------------------------------------------------------------
    // Send + Sync compile-time verification
    // ------------------------------------------------------------------

    #[test]
    fn test_transport_implements_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<InterprocessTransport>();
    }

    // ------------------------------------------------------------------
    // Socket-based tests (Unix / macOS only)
    // ------------------------------------------------------------------

    #[cfg(not(windows))]
    mod unix_socket_tests {
        use super::*;
        use std::sync::atomic::{AtomicU64, Ordering};

        /// Generate a unique socket path for each test to avoid collisions.
        fn tmp_sock() -> String {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            format!("/tmp/claw_ipc_test_{}.sock", id)
        }

        async fn make_server_client(path: &str) -> (InterprocessTransport, InterprocessTransport) {
            // Clean up any leftover socket file.
            let _ = std::fs::remove_file(path);

            let path_owned = path.to_string();
            let server_fut = tokio::spawn(async move {
                InterprocessTransport::new_server(&path_owned)
                    .await
                    .expect("server bind/accept failed")
            });

            // Give the listener a moment to bind before the client connects.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;

            let client = InterprocessTransport::new_client(path)
                .await
                .expect("client connect failed");
            let server = server_fut.await.expect("server task panicked");
            (server, client)
        }

        #[tokio::test]
        async fn test_new_client_server_roundtrip() {
            let path = tmp_sock();
            let (server, client) = make_server_client(&path).await;

            let msg = b"hello from client";
            client.send(msg).await.expect("send failed");
            let received = server.recv().await.expect("recv failed");
            assert_eq!(received, msg);

            let _ = std::fs::remove_file(&path);
        }

        #[tokio::test]
        async fn test_send_recv_multiple_messages() {
            let path = tmp_sock();
            let (server, client) = make_server_client(&path).await;

            let messages: &[&[u8]] = &[b"msg1", b"msg2", b"msg3"];
            for msg in messages {
                client.send(msg).await.expect("send failed");
            }
            for expected in messages {
                let got = server.recv().await.expect("recv failed");
                assert_eq!(&got, expected);
            }

            let _ = std::fs::remove_file(&path);
        }

        #[tokio::test]
        async fn test_send_recv_binary_data() {
            let path = tmp_sock();
            let (server, client) = make_server_client(&path).await;

            let binary: Vec<u8> = (0u8..=255).collect();
            client.send(&binary).await.expect("send failed");
            let got = server.recv().await.expect("recv failed");
            assert_eq!(got, binary);

            let _ = std::fs::remove_file(&path);
        }

        #[tokio::test]
        async fn test_send_empty_message_returns_invalid_message() {
            let path = tmp_sock();
            let (_server, client) = make_server_client(&path).await;

            let err = client.send(b"").await.expect_err("empty send must fail");
            assert_eq!(err, IpcError::InvalidMessage);

            let _ = std::fs::remove_file(&path);
        }

        #[tokio::test]
        async fn test_bidirectional_communication() {
            let path = tmp_sock();
            let (server, client) = make_server_client(&path).await;

            // client → server
            client.send(b"ping").await.expect("send failed");
            let got = server.recv().await.expect("recv failed");
            assert_eq!(got, b"ping");

            // server → client
            server.send(b"pong").await.expect("send failed");
            let got = client.recv().await.expect("recv failed");
            assert_eq!(got, b"pong");

            let _ = std::fs::remove_file(&path);
        }

        #[tokio::test]
        async fn test_large_message_roundtrip() {
            let path = tmp_sock();
            let (server, client) = make_server_client(&path).await;

            let large: Vec<u8> = (0..64 * 1024).map(|i| (i % 256) as u8).collect();
            client.send(&large).await.expect("send failed");
            let got = server.recv().await.expect("recv failed");
            assert_eq!(got, large);

            let _ = std::fs::remove_file(&path);
        }

        #[tokio::test]
        async fn test_connect_nonexistent_endpoint_fails() {
            // Attempt to connect to a socket that has no server listening.
            let path = "/tmp/claw_nonexistent_12345.sock";
            let _ = std::fs::remove_file(path);
            let err = InterprocessTransport::new_client(path)
                .await
                .expect_err("connecting to nothing must fail");
            // Any IpcError variant is acceptable here.
            let _ = err;
        }
    }
}
