//! Message codec — JSON serialization/deserialization of `A2AMessage`.

use crate::a2a::protocol::A2AMessage;
use crate::error::RuntimeError;

/// Serialize an `A2AMessage` to JSON bytes.
pub(super) fn encode_message(msg: &A2AMessage) -> Result<Vec<u8>, RuntimeError> {
    serde_json::to_vec(msg).map_err(|e| RuntimeError::IpcError(format!("encode failed: {}", e)))
}

/// Deserialize an IPC frame payload back into an `A2AMessage`.
pub(super) fn decode_message(bytes: &[u8]) -> Result<A2AMessage, RuntimeError> {
    serde_json::from_slice(bytes)
        .map_err(|e| RuntimeError::IpcError(format!("decode failed: {}", e)))
}

/// Generate a short unique ID string.
pub(super) fn uuid() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);

    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let raw = t.as_nanos() ^ ((seq as u128).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    format!("{:08x}", raw & 0xFFFFFFFF)
}
