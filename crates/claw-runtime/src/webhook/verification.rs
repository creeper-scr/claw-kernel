//! HMAC signature verification for webhooks.
//!
//! Supports HMAC-SHA256 verification used by GitHub, Stripe, and other services.

use super::WebhookError;
use sha2::{Digest, Sha256};

/// Verify HMAC-SHA256 signature.
///
/// # Arguments
///
/// * `secret` - The secret key
/// * `body` - The request body
/// * `signature` - The provided signature (with or without "sha256=" prefix)
///
/// # Returns
///
/// Returns `Ok(())` if verification succeeds, `Err(WebhookError)` otherwise.
///
/// # Example
///
/// ```rust
/// use claw_runtime::webhook::verify_hmac_sha256;
///
/// let secret = "my-secret";
/// let body = b"{\"event\": \"push\"}";
/// let signature = "sha256=computed-signature"; // Or just "computed-signature"
///
/// // In real usage, compute the expected signature:
/// // let expected = compute_hmac_sha256(secret, body);
/// ```
pub fn verify_hmac_sha256(
    secret: &str,
    body: &[u8],
    signature: &str,
) -> Result<(), WebhookError> {
    // Remove prefix if present
    let sig_hex = signature
        .strip_prefix("sha256=")
        .unwrap_or(signature);

    // Decode hex signature
    let sig_bytes = hex::decode(sig_hex)
        .map_err(|_| WebhookError::InvalidSignature)?;

    // Compute expected signature
    let expected = compute_hmac_sha256(secret, body);

    // Constant-time comparison
    if subtle::constant_time_eq(&sig_bytes, &expected) {
        Ok(())
    } else {
        Err(WebhookError::HmacVerificationFailed)
    }
}

/// Compute HMAC-SHA256 signature.
///
/// This is a simplified implementation. In production, use the `hmac` crate.
pub fn compute_hmac_sha256(secret: &str, body: &[u8]) -> Vec<u8> {
    // Simple HMAC using SHA-256
    // HMAC(K, m) = H((K' ⊕ opad) || H((K' ⊕ ipad) || m))
    
    let key = secret.as_bytes();
    
    // Key padding to block size (64 bytes for SHA-256)
    let mut k_pad = vec![0u8; 64];
    if key.len() > 64 {
        // Hash the key if it's too long
        let hash = Sha256::digest(key);
        k_pad[..32].copy_from_slice(&hash);
    } else {
        k_pad[..key.len()].copy_from_slice(key);
    }

    // Inner padding: ipad = 0x36
    let mut ipad = k_pad.clone();
    for b in ipad.iter_mut() {
        *b ^= 0x36;
    }

    // Outer padding: opad = 0x5c
    let mut opad = k_pad;
    for b in opad.iter_mut() {
        *b ^= 0x5c;
    }

    // Inner hash: H((K' ⊕ ipad) || m)
    let mut inner_hasher = Sha256::new();
    inner_hasher.update(&ipad);
    inner_hasher.update(body);
    let inner_hash = inner_hasher.finalize();

    // Outer hash: H((K' ⊕ opad) || inner_hash)
    let mut outer_hasher = Sha256::new();
    outer_hasher.update(&opad);
    outer_hasher.update(&inner_hash);
    
    outer_hasher.finalize().to_vec()
}

/// Webhook verifier trait for different signature schemes.
pub trait WebhookVerifier: Send + Sync {
    /// Verify a webhook request signature.
    fn verify(&self, body: &[u8], signature: &str) -> Result<(), WebhookError>;
}

/// HMAC-SHA256 verifier.
pub struct HmacSha256Verifier {
    secret: String,
    prefix: Option<String>,
}

impl HmacSha256Verifier {
    /// Create a new HMAC-SHA256 verifier.
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
            prefix: Some("sha256=".to_string()),
        }
    }

    /// Create a verifier without signature prefix.
    pub fn new_without_prefix(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
            prefix: None,
        }
    }

    /// Set the signature prefix.
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }
}

impl WebhookVerifier for HmacSha256Verifier {
    fn verify(&self, body: &[u8], signature: &str) -> Result<(), WebhookError> {
        let sig = if let Some(ref prefix) = self.prefix {
            signature.strip_prefix(prefix).unwrap_or(signature)
        } else {
            signature
        };

        verify_hmac_sha256(&self.secret, body, sig)
    }
}

/// No-op verifier (for testing or unverified endpoints).
pub struct NoopVerifier;

impl WebhookVerifier for NoopVerifier {
    fn verify(&self, _body: &[u8], _signature: &str) -> Result<(), WebhookError> {
        Ok(())
    }
}

// Hex encoding/decoding utilities
mod hex {
    pub fn decode(s: &str) -> Result<Vec<u8>, ()> {
        if s.len() % 2 != 0 {
            return Err(());
        }

        let mut result = Vec::with_capacity(s.len() / 2);
        for i in (0..s.len()).step_by(2) {
            let byte = u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| ())?;
            result.push(byte);
        }
        Ok(result)
    }

    #[allow(dead_code)]
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

// Constant-time comparison
mod subtle {
    /// Compare two byte slices in constant time.
    pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
        if a.len() != b.len() {
            return false;
        }

        let mut result = 0u8;
        for (x, y) in a.iter().zip(b.iter()) {
            result |= x ^ y;
        }

        result == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_hmac_sha256() {
        let secret = "my-secret-key";
        let body = b"{\"event\": \"push\", \"ref\": \"main\"}";

        let signature = compute_hmac_sha256(secret, body);
        assert_eq!(signature.len(), 32); // SHA-256 output is 32 bytes
    }

    #[test]
    fn test_verify_hmac_sha256_valid() {
        let secret = "my-secret";
        let body = b"test body";
        let expected = compute_hmac_sha256(secret, body);
        let signature = format!("sha256={}", hex::encode(&expected));

        let result = verify_hmac_sha256(secret, body, &signature);
        assert!(result.is_ok());
    }

    #[test]
    fn test_verify_hmac_sha256_invalid() {
        let secret = "my-secret";
        let body = b"test body";
        let signature = "sha256=0000000000000000000000000000000000000000000000000000000000000000";

        let result = verify_hmac_sha256(secret, body, signature);
        assert!(matches!(result, Err(WebhookError::HmacVerificationFailed)));
    }

    #[test]
    fn test_verify_hmac_sha256_no_prefix() {
        let secret = "my-secret";
        let body = b"test body";
        let expected = compute_hmac_sha256(secret, body);
        let signature = hex::encode(&expected);

        let result = verify_hmac_sha256(secret, body, &signature);
        assert!(result.is_ok());
    }

    #[test]
    fn test_hmac_verifier() {
        let verifier = HmacSha256Verifier::new("secret-key");
        let body = b"webhook payload";
        let expected = compute_hmac_sha256("secret-key", body);
        let signature = format!("sha256={}", hex::encode(&expected));

        assert!(verifier.verify(body, &signature).is_ok());
        assert!(verifier.verify(body, "sha256=invalid").is_err());
    }

    #[test]
    fn test_noop_verifier() {
        let verifier = NoopVerifier;
        assert!(verifier.verify(b"any", "any").is_ok());
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(subtle::constant_time_eq(b"abc", b"abc"));
        assert!(!subtle::constant_time_eq(b"abc", b"def"));
        assert!(!subtle::constant_time_eq(b"ab", b"abc"));
    }

    #[test]
    fn test_hex_encode_decode() {
        let bytes = b"hello world";
        let encoded = hex::encode(bytes);
        let decoded = hex::decode(&encoded).unwrap();
        assert_eq!(bytes.to_vec(), decoded);
    }

    #[test]
    fn test_hex_decode_invalid() {
        // Odd length
        assert!(hex::decode("abc").is_err());
        // Invalid characters
        assert!(hex::decode("gg").is_err());
    }
}
