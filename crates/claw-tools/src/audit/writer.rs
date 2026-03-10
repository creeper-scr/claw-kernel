//! Async audit log writer with automatic flush, file rotation, and optional
//! HMAC-SHA256 per-record signatures for tamper detection.
//!
//! # Format
//!
//! Without HMAC key (plain JSON-lines):
//! ```json
//! {"event_type":"TOOL_CALL","timestamp_ms":1700000000000,"agent_id":"a1","tool_name":"echo",...}
//! ```
//!
//! With HMAC key (signed JSON-lines):
//! ```json
//! {"payload":"{\"event_type\":\"TOOL_CALL\",...}","signature":"deadbeef..."}
//! ```
//! The `payload` value is a JSON string of the serialised `AuditEvent`.
//! The `signature` is hex-encoded HMAC-SHA256(key, payload_bytes).

use super::{AuditEvent, AuditLogConfig, AuditStore};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

type HmacSha256 = Hmac<Sha256>;

/// Days in each month (non-leap year)
const DAYS_IN_MONTH: [u64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// Handle to the audit log writer for sending events.
#[derive(Debug, Clone)]
pub struct AuditLogWriterHandle {
    pub(crate) sender: mpsc::Sender<AuditEvent>,
}

impl AuditLogWriterHandle {
    /// Create a no-op handle that silently drops all audit events.
    ///
    /// Useful in contexts where audit logging is not required (e.g. agent.spawn sessions).
    pub fn noop() -> Self {
        let (sender, _receiver) = mpsc::channel(1);
        Self { sender }
    }

    /// Send an audit event to the writer.
    ///
    /// This is non-blocking. Events are queued for async processing.
    pub async fn send(&self, event: AuditEvent) {
        // Use try_send to avoid blocking, drop event if channel is full
        let _ = self.sender.try_send(event);
    }

    /// Send an audit event synchronously (fire-and-forget).
    ///
    /// This is useful when you don't want to await the send.
    pub fn send_blocking(&self, event: AuditEvent) {
        let _ = self.sender.try_send(event);
    }
}

/// Async audit log writer that handles file I/O.
pub struct AuditLogWriter {
    config: AuditLogConfig,
    receiver: mpsc::Receiver<AuditEvent>,
    buffer: Vec<u8>,
    current_file_size: u64,
    store: Arc<AuditStore>,
}

impl AuditLogWriter {
    /// Create a new audit log writer and start the background task.
    ///
    /// Returns a handle for sending events, the shared in-memory store, and a
    /// join handle for the background task.
    pub fn start(config: AuditLogConfig) -> (AuditLogWriterHandle, Arc<AuditStore>, tokio::task::JoinHandle<()>) {
        let store = Arc::new(AuditStore::new(config.max_memory_entries));
        let (sender, receiver) = mpsc::channel(10_000); // Buffer up to 10k events
        let writer = Self {
            config,
            receiver,
            buffer: Vec::with_capacity(64 * 1024), // 64KB initial buffer
            current_file_size: 0,
            store: Arc::clone(&store),
        };

        let handle = tokio::spawn(async move {
            if let Err(e) = writer.run().await {
                eprintln!("[audit-log] writer error: {}", e);
            }
        });

        (AuditLogWriterHandle { sender }, store, handle)
    }

    /// Main event loop: receive events and flush periodically.
    async fn run(mut self) -> std::io::Result<()> {
        // Ensure log directory exists
        fs::create_dir_all(&self.config.log_dir).await?;

        // Check current file size if file exists
        let log_path = self.config.log_path();
        if let Ok(metadata) = fs::metadata(&log_path).await {
            self.current_file_size = metadata.len();
        }

        let mut flush_tick = interval(Duration::from_secs(self.config.flush_interval_secs));
        let mut file = self.open_log_file().await?;

        loop {
            tokio::select! {
                // Receive audit events
                Some(event) = self.receiver.recv() => {
                    // Mirror into the in-memory query store before writing to disk.
                    self.store.push(event.clone());
                    self.format_event(&event);

                    // Check if we need rotation before writing
                    if self.current_file_size + self.buffer.len() as u64 > self.config.max_file_size_bytes {
                        // Flush current buffer and rotate
                        file.write_all(&self.buffer).await?;
                        file.flush().await?;
                        self.buffer.clear();

                        self.rotate_log_file().await?;
                        file = self.open_log_file().await?;
                        self.current_file_size = 0;
                    }
                }

                // Periodic flush
                _ = flush_tick.tick() => {
                    if !self.buffer.is_empty() {
                        file.write_all(&self.buffer).await?;
                        file.flush().await?;
                        self.current_file_size += self.buffer.len() as u64;
                        self.buffer.clear();
                    }
                }
            }
        }
    }

    /// Serialise an audit event to a log line and push it into the write buffer.
    ///
    /// # Format selection
    ///
    /// - **With HMAC key** — emits a JSON object with two fields:
    ///   - `"payload"`: the JSON-serialised event as a *string*
    ///   - `"signature"`: hex-encoded HMAC-SHA256(key, payload_bytes)
    ///   This lets verifiers detect any post-write tampering by recomputing the
    ///   signature from the payload bytes.
    ///
    /// - **Without HMAC key** — emits a flat JSON object of the event with an
    ///   additional top-level `"event_type"` field for easy grep/jq queries.
    fn format_event(&mut self, event: &AuditEvent) {
        if let Some(key) = &self.config.hmac_key {
            // Signed JSON-lines mode
            // 1. Serialise the event to a JSON string (the payload)
            if let Ok(payload_json) = serde_json::to_string(event) {
                let payload_bytes = payload_json.as_bytes();

                // 2. Compute HMAC-SHA256 over the payload bytes
                let mut mac = HmacSha256::new_from_slice(key)
                    .expect("HMAC accepts any key length");
                mac.update(payload_bytes);
                let sig_bytes = mac.finalize().into_bytes();
                let sig_hex = hex_encode(&sig_bytes);

                // 3. Emit: {"payload":"...","signature":"..."}
                if let Ok(record) = serde_json::to_string(&serde_json::json!({
                    "payload": payload_json,
                    "signature": sig_hex,
                })) {
                    self.buffer.extend_from_slice(record.as_bytes());
                    self.buffer.push(b'\n');
                }
            }
        } else {
            // Plain JSON-lines mode: flat event object + event_type field
            let timestamp = format_timestamp(event.timestamp_ms());
            let event_type = event.event_type();
            let agent_id = event.agent_id();

            // Build a JSON object combining event fields with a human-readable timestamp
            let mut obj = match serde_json::to_value(event) {
                Ok(serde_json::Value::Object(m)) => m,
                _ => {
                    // Fallback to text format on serialisation failure
                    let line = format!(
                        "{} [{}] agent_id={}\n",
                        timestamp, event_type, agent_id
                    );
                    self.buffer.extend_from_slice(line.as_bytes());
                    return;
                }
            };
            obj.insert("event_type".to_string(), serde_json::Value::String(event_type.to_string()));
            obj.insert("timestamp_utc".to_string(), serde_json::Value::String(timestamp));

            if let Ok(line) = serde_json::to_string(&serde_json::Value::Object(obj)) {
                self.buffer.extend_from_slice(line.as_bytes());
                self.buffer.push(b'\n');
            }
        }
    }

    /// Open the current log file for appending.
    async fn open_log_file(&self) -> std::io::Result<fs::File> {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.config.log_path())
            .await
    }

    /// Rotate the current log file (rename with timestamp).
    async fn rotate_log_file(&self) -> std::io::Result<()> {
        let log_path = self.config.log_path();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let rotated_name = format!("audit.log.{}", timestamp);
        let rotated_path = self.config.log_dir.join(rotated_name);

        fs::rename(&log_path, &rotated_path).await?;

        Ok(())
    }
}

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Encode a byte slice as a lowercase hex string.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Format timestamp as ISO 8601 without milliseconds.
///
/// Simple implementation that converts Unix timestamp to UTC datetime.
fn format_timestamp(timestamp_ms: u64) -> String {
    let secs = timestamp_ms / 1000;
    let (year, month, day, hour, minute, second) = unix_to_utc_datetime(secs);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second
    )
}

/// Convert Unix timestamp to UTC datetime components (year, month, day, hour, minute, second).
/// Uses a simple algorithm that's accurate for timestamps from 1970 to 2099.
fn unix_to_utc_datetime(mut secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    const SECS_PER_MINUTE: u64 = 60;

    // Extract time components
    let second = secs % SECS_PER_MINUTE;
    secs /= SECS_PER_MINUTE;
    let minute = secs % 60;
    secs /= 60;
    let hour = secs % 24;
    secs /= 24;

    // secs now contains days since 1970-01-01
    let mut days = secs;

    // Calculate year
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    // Calculate month
    let mut month = 1u64;
    while month <= 12 {
        let dim = days_in_month(year, month);
        if days < dim {
            break;
        }
        days -= dim;
        month += 1;
    }

    // days now contains day of month (0-indexed)
    let day = days + 1;

    (year, month, day, hour, minute, second)
}

/// Check if a year is a leap year.
fn is_leap_year(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Get number of days in a month for a given year.
fn days_in_month(year: u64, month: u64) -> u64 {
    if month == 2 && is_leap_year(year) {
        29
    } else {
        DAYS_IN_MONTH[(month - 1) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Create a temporary directory path for testing.
    /// Note: We don't use the tempfile crate to avoid additional dependencies.
    fn temp_dir() -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("claw-tools-test-{}", unique))
    }

    #[tokio::test]
    async fn test_audit_log_writer_basic() {
        let temp_path = temp_dir();
        let _ = fs::remove_dir_all(&temp_path).await;

        let config = AuditLogConfig::new()
            .with_log_dir(&temp_path)
            .with_flush_interval(1);

        let (handle, _store, _task) = AuditLogWriter::start(config.clone());

        // Send some events
        let event = AuditEvent::ToolCall {
            timestamp_ms: 1700000000000,
            agent_id: "agent-1".to_string(),
            tool_name: "echo".to_string(),
            args: Some(serde_json::json!({"msg": "hello"})),
        };

        handle.send(event).await;

        // Wait a bit for flush
        tokio::time::sleep(Duration::from_millis(1100)).await;

        // Check log file exists and contains our event
        let log_content = fs::read_to_string(config.log_path()).await.unwrap();
        assert!(log_content.contains("TOOL_CALL"));
        assert!(log_content.contains("agent-1"));
        assert!(log_content.contains("echo"));
        // Plain mode: no "signature" field
        assert!(!log_content.contains("\"signature\""));

        // Cleanup
        let _ = fs::remove_dir_all(&temp_path).await;
    }

    #[tokio::test]
    async fn test_audit_log_writer_signed() {
        let temp_path = temp_dir();
        let _ = fs::remove_dir_all(&temp_path).await;

        let key = [0u8; 32];
        let config = AuditLogConfig::new()
            .with_log_dir(&temp_path)
            .with_flush_interval(1)
            .with_hmac_key(key);

        let (handle, _store, _task) = AuditLogWriter::start(config.clone());

        let event = AuditEvent::ToolCall {
            timestamp_ms: 1700000000000,
            agent_id: "agent-1".to_string(),
            tool_name: "echo".to_string(),
            args: None,
        };
        handle.send(event).await;
        tokio::time::sleep(Duration::from_millis(1100)).await;

        let log_content = fs::read_to_string(config.log_path()).await.unwrap();

        // Signed mode: must have "payload" and "signature" fields
        assert!(log_content.contains("\"payload\""), "signed record must have payload field");
        assert!(log_content.contains("\"signature\""), "signed record must have signature field");

        // Parse first line as JSON and verify structure
        let first_line = log_content.lines().next().unwrap();
        let record: serde_json::Value = serde_json::from_str(first_line).unwrap();
        let payload_str = record["payload"].as_str().unwrap();
        let sig_str = record["signature"].as_str().unwrap();

        // Recompute signature to verify it matches
        let mut mac = HmacSha256::new_from_slice(&key).unwrap();
        mac.update(payload_str.as_bytes());
        let expected_sig = hex_encode(&mac.finalize().into_bytes());
        assert_eq!(sig_str, expected_sig, "HMAC signature must match");

        // Cleanup
        let _ = fs::remove_dir_all(&temp_path).await;
    }

    #[tokio::test]
    async fn test_audit_log_writer_agent_spawned_event() {
        let temp_path = temp_dir();
        let _ = fs::remove_dir_all(&temp_path).await;

        let config = AuditLogConfig::new()
            .with_log_dir(&temp_path)
            .with_flush_interval(1);

        let (handle, store, _task) = AuditLogWriter::start(config.clone());

        let event = AuditEvent::AgentSpawned {
            timestamp_ms: 1700000000000,
            agent_id: "agent-42".to_string(),
            agent_name: "worker".to_string(),
            restart_policy: "on_failure".to_string(),
        };
        handle.send(event).await;
        tokio::time::sleep(Duration::from_millis(1100)).await;

        // Check in-memory store
        let entries = store.list(10, Some("agent-42"), None);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event_type(), "AGENT_SPAWNED");

        // Check log file
        let log_content = fs::read_to_string(config.log_path()).await.unwrap();
        assert!(log_content.contains("AGENT_SPAWNED"));
        assert!(log_content.contains("agent-42"));
        assert!(log_content.contains("worker"));

        // Cleanup
        let _ = fs::remove_dir_all(&temp_path).await;
    }

    #[test]
    fn test_format_timestamp() {
        // Test with a known timestamp (2023-11-14 22:13:20 UTC)
        let ts = 1700000000000u64;
        let formatted = format_timestamp(ts);
        assert_eq!(formatted, "2023-11-14T22:13:20Z");
    }

    #[test]
    fn test_unix_to_utc_datetime() {
        // Test epoch
        let (y, m, d, h, min, s) = unix_to_utc_datetime(0);
        assert_eq!((y, m, d, h, min, s), (1970, 1, 1, 0, 0, 0));

        // Test 2023-11-14 22:13:20 UTC (1700000000 seconds)
        let (y, m, d, h, min, s) = unix_to_utc_datetime(1700000000);
        assert_eq!((y, m, d, h, min, s), (2023, 11, 14, 22, 13, 20));

        // Test leap year (2020-02-29)
        // 2020-02-29 00:00:00 UTC is 1582934400 seconds from epoch
        let (y, m, d, h, min, s) = unix_to_utc_datetime(1582934400);
        assert_eq!((y, m, d, h, min, s), (2020, 2, 29, 0, 0, 0));
    }

    #[test]
    fn test_is_leap_year() {
        assert!(is_leap_year(2000));
        assert!(is_leap_year(2020));
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(1900));
        assert!(!is_leap_year(2023));
    }

    #[test]
    fn test_hex_encode() {
        assert_eq!(hex_encode(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
        assert_eq!(hex_encode(&[0x00, 0xff]), "00ff");
    }
}
