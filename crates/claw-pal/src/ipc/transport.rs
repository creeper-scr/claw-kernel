//! Core IPC transport backed by `interprocess` local sockets (Unix) or
//! Windows Named Pipes.
//!
//! **Platform Support:**
//! - Unix-like systems (Linux, macOS): backed by `interprocess` local sockets.
//! - Windows: backed by `tokio::net::windows::named_pipe`.
//!
//! Design: a single background reader task drains the socket/pipe and forwards
//! frames via an `mpsc` channel.  The write path is serialised through a
//! `Mutex<PipeWriter>`.  This deliberately avoids concurrent bi-directional
//! split I/O on the same handle (which panics on macOS with interprocess 1.2.1,
//! and is also unsafe on Windows named pipes that cannot be split).

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
/// awaiting `recv`.  Writes are serialised through a `Mutex`.
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
        f.debug_struct("InterprocessTransport")
            .finish_non_exhaustive()
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
            .map_err(|_| IpcError::ConnectionRefused)?;
        Ok(Self::from_stream(stream))
    }

    /// Bind a listener, accept exactly one incoming connection.
    pub async fn new_server(endpoint: &str) -> Result<Self, IpcError> {
        let listener =
            LocalSocketListener::bind(endpoint).map_err(|_| IpcError::ConnectionRefused)?;
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
    /// Does not establish an actual socket connection; use `new_client` for that.
    async fn connect(endpoint: &str) -> Result<IpcConnection, IpcError> {
        if endpoint.is_empty() {
            return Err(IpcError::InvalidMessage);
        }
        Ok(IpcConnection::new(endpoint.to_string()))
    }

    /// Return metadata for a listener endpoint.
    ///
    /// Does not bind an actual socket; use `new_server` for that.
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
// Windows Named Pipe implementation
// ---------------------------------------------------------------------------

#[cfg(windows)]
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};

#[cfg(windows)]
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[cfg(windows)]
use std::pin::Pin;
#[cfg(windows)]
use std::task::{Context, Poll};

/// Convert endpoint path to Windows Named Pipe name.
/// If the path doesn't start with `\\.\pipe\`, prepend it.
#[cfg(windows)]
fn to_pipe_name(endpoint: &str) -> String {
    const PIPE_PREFIX: &str = r"\\.\pipe\";
    if endpoint.starts_with(PIPE_PREFIX) {
        endpoint.to_string()
    } else {
        format!("{}{}", PIPE_PREFIX, endpoint.replace('/', "\\"))
    }
}

/// Wrapper enum to unify server and client pipe write operations.
#[cfg(windows)]
enum PipeWriter {
    Server(NamedPipeServer),
    Client(NamedPipeClient),
}

#[cfg(windows)]
impl AsyncWrite for PipeWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            PipeWriter::Server(server) => Pin::new(server).poll_write(cx, buf),
            PipeWriter::Client(client) => Pin::new(client).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            PipeWriter::Server(server) => Pin::new(server).poll_flush(cx),
            PipeWriter::Client(client) => Pin::new(client).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            PipeWriter::Server(server) => Pin::new(server).poll_shutdown(cx),
            PipeWriter::Client(client) => Pin::new(client).poll_shutdown(cx),
        }
    }
}

/// Wrapper enum to unify server and client pipe read operations.
#[cfg(windows)]
enum PipeReader {
    Server(NamedPipeServer),
    Client(NamedPipeClient),
}

#[cfg(windows)]
impl AsyncRead for PipeReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            PipeReader::Server(server) => Pin::new(server).poll_read(cx, buf),
            PipeReader::Client(client) => Pin::new(client).poll_read(cx, buf),
        }
    }
}

/// IPC transport backed by Windows Named Pipes.
///
/// A dedicated reader task continuously reads frames from the pipe and
/// sends them into an internal `mpsc` channel.  Callers receive frames by
/// awaiting `recv`.  Writes are serialised through a `Mutex`.
#[cfg(windows)]
pub struct InterprocessTransport {
    writer: Mutex<PipeWriter>,
    recv_rx: Mutex<mpsc::Receiver<Result<Vec<u8>, IpcError>>>,
    /// Keeps the reader task alive for the lifetime of this transport.
    _reader_task: tokio::task::JoinHandle<()>,
}

#[cfg(windows)]
impl std::fmt::Debug for InterprocessTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InterprocessTransport")
            .finish_non_exhaustive()
    }
}

#[cfg(windows)]
impl InterprocessTransport {
    /// Internal constructor: spawn reader task, return Self.
    fn from_reader_writer(reader: PipeReader, writer: PipeWriter) -> Self {
        let (tx, rx) = mpsc::channel::<Result<Vec<u8>, IpcError>>(128);
        let handle = tokio::spawn(async move {
            let mut reader = reader;
            loop {
                match read_frame(&mut reader).await {
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
            writer: Mutex::new(writer),
            recv_rx: Mutex::new(rx),
            _reader_task: handle,
        }
    }

    /// Connect as a client to the given endpoint path.
    pub async fn new_client(endpoint: &str) -> Result<Self, IpcError> {
        let pipe_name = to_pipe_name(endpoint);
        let client = ClientOptions::new()
            .open(&pipe_name)
            .map_err(|_| IpcError::ConnectionRefused)?;

        // For client, we need to create another client handle for reading
        // Since NamedPipeClient is not cloneable, we open another connection
        let client_for_write = ClientOptions::new()
            .open(&pipe_name)
            .map_err(|_| IpcError::ConnectionRefused)?;

        Ok(Self::from_reader_writer(
            PipeReader::Client(client),
            PipeWriter::Client(client_for_write),
        ))
    }

    /// Bind a listener, accept exactly one incoming connection.
    pub async fn new_server(endpoint: &str) -> Result<Self, IpcError> {
        let pipe_name = to_pipe_name(endpoint);

        // Create the named pipe server
        let server = ServerOptions::new()
            .create(&pipe_name)
            .map_err(|_| IpcError::ConnectionRefused)?;

        // Wait for a client to connect
        server
            .connect()
            .await
            .map_err(|_| IpcError::ConnectionRefused)?;

        // Re-create the server for another connection (for read/write split simulation)
        let server_for_write = ServerOptions::new()
            .create(&pipe_name)
            .map_err(|_| IpcError::ConnectionRefused)?;
        server_for_write
            .connect()
            .await
            .map_err(|_| IpcError::ConnectionRefused)?;

        Ok(Self::from_reader_writer(
            PipeReader::Server(server),
            PipeWriter::Server(server_for_write),
        ))
    }
}

#[cfg(windows)]
// SAFETY: PipeWriter and the mpsc types are Send; JoinHandle is Send.
unsafe impl Send for InterprocessTransport {}
#[cfg(windows)]
unsafe impl Sync for InterprocessTransport {}

#[cfg(windows)]
#[async_trait::async_trait]
impl IpcTransport for InterprocessTransport {
    /// Return metadata for a client connection endpoint.
    ///
    /// Does not establish an actual pipe connection; use `new_client` for that.
    async fn connect(endpoint: &str) -> Result<IpcConnection, IpcError> {
        if endpoint.is_empty() {
            return Err(IpcError::InvalidMessage);
        }
        Ok(IpcConnection::new(endpoint.to_string()))
    }

    /// Return metadata for a listener endpoint.
    ///
    /// Does not bind an actual pipe; use `new_server` for that.
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

    // ------------------------------------------------------------------
    // Named Pipe tests (Windows only)
    // ------------------------------------------------------------------

    #[cfg(windows)]
    mod windows_pipe_tests {
        use super::*;
        use std::sync::atomic::{AtomicU64, Ordering};

        /// Generate a unique pipe name for each test to avoid collisions.
        fn tmp_pipe() -> String {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            format!(r"\\.\pipe\claw_ipc_test_{}", id)
        }

        async fn make_server_client(
            pipe_name: &str,
        ) -> (InterprocessTransport, InterprocessTransport) {
            let pipe_owned = pipe_name.to_string();
            let server_fut = tokio::spawn(async move {
                InterprocessTransport::new_server(&pipe_owned)
                    .await
                    .expect("server bind/accept failed")
            });

            // Give the listener a moment to create the pipe before the client connects.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;

            let client = InterprocessTransport::new_client(pipe_name)
                .await
                .expect("client connect failed");
            let server = server_fut.await.expect("server task panicked");
            (server, client)
        }

        #[tokio::test]
        async fn test_new_client_server_roundtrip() {
            let pipe = tmp_pipe();
            let (server, client) = make_server_client(&pipe).await;

            let msg = b"hello from client";
            client.send(msg).await.expect("send failed");
            let received = server.recv().await.expect("recv failed");
            assert_eq!(received, msg);
        }

        #[tokio::test]
        async fn test_send_recv_multiple_messages() {
            let pipe = tmp_pipe();
            let (server, client) = make_server_client(&pipe).await;

            let messages: &[&[u8]] = &[b"msg1", b"msg2", b"msg3"];
            for msg in messages {
                client.send(msg).await.expect("send failed");
            }
            for expected in messages {
                let got = server.recv().await.expect("recv failed");
                assert_eq!(&got, expected);
            }
        }

        #[tokio::test]
        async fn test_send_recv_binary_data() {
            let pipe = tmp_pipe();
            let (server, client) = make_server_client(&pipe).await;

            let binary: Vec<u8> = (0u8..=255).collect();
            client.send(&binary).await.expect("send failed");
            let got = server.recv().await.expect("recv failed");
            assert_eq!(got, binary);
        }

        #[tokio::test]
        async fn test_send_empty_message_returns_invalid_message() {
            let pipe = tmp_pipe();
            let (_server, client) = make_server_client(&pipe).await;

            let err = client.send(b"").await.expect_err("empty send must fail");
            assert_eq!(err, IpcError::InvalidMessage);
        }

        #[tokio::test]
        async fn test_bidirectional_communication() {
            let pipe = tmp_pipe();
            let (server, client) = make_server_client(&pipe).await;

            // client → server
            client.send(b"ping").await.expect("send failed");
            let got = server.recv().await.expect("recv failed");
            assert_eq!(got, b"ping");

            // server → client
            server.send(b"pong").await.expect("send failed");
            let got = client.recv().await.expect("recv failed");
            assert_eq!(got, b"pong");
        }

        #[tokio::test]
        async fn test_large_message_roundtrip() {
            let pipe = tmp_pipe();
            let (server, client) = make_server_client(&pipe).await;

            let large: Vec<u8> = (0..64 * 1024).map(|i| (i % 256) as u8).collect();
            client.send(&large).await.expect("send failed");
            let got = server.recv().await.expect("recv failed");
            assert_eq!(got, large);
        }

        #[tokio::test]
        async fn test_connect_nonexistent_endpoint_fails() {
            // Attempt to connect to a pipe that has no server listening.
            let pipe = r"\\.\pipe\claw_nonexistent_12345";
            let err = InterprocessTransport::new_client(pipe)
                .await
                .expect_err("connecting to nothing must fail");
            // Any IpcError variant is acceptable here.
            let _ = err;
        }
    }
}
