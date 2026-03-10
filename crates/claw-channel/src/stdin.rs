//! StdinChannel — reads lines from stdin and writes to stdout.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use async_trait::async_trait;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{mpsc, Mutex},
    task::JoinHandle,
};

use crate::{
    error::ChannelError,
    traits::Channel,
    types::{ChannelId, ChannelMessage, Platform},
};

/// A channel adapter that reads from stdin and writes to stdout.
pub struct StdinChannel {
    id: ChannelId,
    tx: mpsc::Sender<ChannelMessage>,
    rx: Mutex<mpsc::Receiver<ChannelMessage>>,
    connected: Arc<AtomicBool>,
    task_handle: Mutex<Option<JoinHandle<()>>>,
    /// Maximum time to wait for the next inbound message.
    /// A value of `Duration::ZERO` means wait indefinitely (original behaviour).
    recv_timeout: std::time::Duration,
}

impl StdinChannel {
    /// Create a new StdinChannel with an internal queue capacity of 64.
    pub fn new(id: ChannelId) -> Self {
        let (tx, rx) = mpsc::channel(64);
        Self {
            id,
            tx,
            rx: Mutex::new(rx),
            connected: Arc::new(AtomicBool::new(false)),
            task_handle: Mutex::new(None),
            recv_timeout: std::time::Duration::ZERO,
        }
    }

    /// Set the maximum time to wait for the next inbound message.
    ///
    /// When set to a non-zero duration, `recv()` returns
    /// `Err(ChannelError::ReceiveFailed("recv timeout"))` if no message
    /// arrives within the deadline.
    ///
    /// A value of `Duration::ZERO` (the default) disables the timeout so
    /// that `recv()` blocks indefinitely, preserving the original behaviour.
    pub fn with_recv_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.recv_timeout = timeout;
        self
    }

    /// Inject a message directly into the queue (test helper).
    #[cfg(test)]
    pub async fn inject(&self, msg: ChannelMessage) {
        let _ = self.tx.send(msg).await;
    }
}

#[async_trait]
impl Channel for StdinChannel {
    fn platform(&self) -> &str {
        "stdin"
    }

    fn channel_id(&self) -> &ChannelId {
        &self.id
    }

    /// Start a background task that reads lines from stdin.
    async fn connect(&self) -> Result<(), ChannelError> {
        self.connected.store(true, Ordering::SeqCst);

        let tx = self.tx.clone();
        let connected = Arc::clone(&self.connected);
        let id = self.id.clone();

        let handle = tokio::spawn(async move {
            let stdin = tokio::io::stdin();
            let mut lines = BufReader::new(stdin).lines();

            while connected.load(Ordering::SeqCst) {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let msg = ChannelMessage::inbound(id.clone(), Platform::Stdin, line);
                        if tx.send(msg).await.is_err() {
                            // Receiver dropped — stop reading.
                            break;
                        }
                    }
                    // EOF or error — stop the loop.
                    Ok(None) | Err(_) => break,
                }
            }
        });

        *self.task_handle.lock().await = Some(handle);
        Ok(())
    }

    /// Stop reading from stdin.
    async fn disconnect(&self) -> Result<(), ChannelError> {
        self.connected.store(false, Ordering::SeqCst);
        if let Some(handle) = self.task_handle.lock().await.take() {
            handle.abort();
        }
        Ok(())
    }

    /// Write `message.content` followed by a newline to stdout.
    async fn send(&self, message: ChannelMessage) -> Result<(), ChannelError> {
        let mut stdout = tokio::io::stdout();
        let line = format!("{}\n", message.content);
        stdout
            .write_all(line.as_bytes())
            .await
            .map_err(|e| ChannelError::SendFailed(e.to_string()))?;
        stdout
            .flush()
            .await
            .map_err(|e| ChannelError::SendFailed(e.to_string()))?;
        Ok(())
    }

    /// Receive the next message from the internal queue.
    ///
    /// If `recv_timeout` is non-zero, returns `Err(ReceiveFailed)` if no
    /// message arrives within the configured deadline.
    async fn recv(&self) -> Result<ChannelMessage, ChannelError> {
        let recv_fut = async { self.rx.lock().await.recv().await };
        if self.recv_timeout.is_zero() {
            // FIX-17: original behaviour — block indefinitely.
            recv_fut
                .await
                .ok_or_else(|| ChannelError::ReceiveFailed("disconnected".to_string()))
        } else {
            tokio::time::timeout(self.recv_timeout, recv_fut)
                .await
                .map_err(|_| ChannelError::ReceiveFailed("recv timeout".to_string()))?
                .ok_or_else(|| ChannelError::ReceiveFailed("disconnected".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stdin_channel_new() {
        // Constructor must not panic.
        let _ch = StdinChannel::new(ChannelId::new("test-stdin"));
    }

    #[tokio::test]
    async fn test_stdin_channel_platform() {
        let ch = StdinChannel::new(ChannelId::new("s1"));
        assert_eq!(ch.platform(), "stdin");
    }

    #[tokio::test]
    async fn test_stdin_channel_send_without_connect() {
        // send() should not panic; actual write may or may not succeed in CI.
        let ch = StdinChannel::new(ChannelId::new("s2"));
        let msg = ChannelMessage::outbound(ChannelId::new("s2"), Platform::Stdin, "hello");
        // We only assert it doesn't panic, not that it succeeds.
        let _ = ch.send(msg).await;
    }

    #[tokio::test]
    async fn test_stdin_channel_manual_recv() {
        let ch = StdinChannel::new(ChannelId::new("s3"));
        let injected =
            ChannelMessage::inbound(ChannelId::new("s3"), Platform::Stdin, "injected line");
        ch.inject(injected.clone()).await;

        let received = ch.recv().await.expect("should receive injected message");
        assert_eq!(received.content, "injected line");
        assert_eq!(received.channel_id, ChannelId::new("s3"));
    }
}
