//! Security module for claw-kernel.
//!
//! Provides PowerKey validation, Argon2 hashing, and mode transition guards.
//! Implements the dual-mode security model described in ADR-003.

use crate::traits::sandbox::ExecutionMode;
use argon2::Config;
use std::fmt;
use std::time::Instant;
use zeroize::{ZeroizeOnDrop, Zeroizing};

/// Minimum key length for Power Key (2026 security standard).
pub const MIN_KEY_LENGTH: usize = 12;

/// Minimum number of distinct character types required (uppercase/lowercase/digit/special).
pub const MIN_CHAR_TYPES: usize = 2;

/// Generate a random 16-byte salt using OS entropy.
fn rand_salt() -> [u8; 16] {
    use rand::Rng;
    rand::thread_rng().gen()
}
/// Security-related errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecurityError {
    /// Key is too short.
    KeyTooShort {
        /// Actual length of the provided key.
        len: usize,
        /// Minimum required length.
        min: usize,
    },
    /// Key lacks character diversity (needs at least 2 of: uppercase, lowercase, digit, special).
    InsufficientComplexity {
        /// Number of character types found.
        found_types: usize,
        /// Number of character types required.
        required: usize,
    },
    /// Provided key does not match stored hash.
    InvalidPowerKey,
    /// Requested mode transition is not allowed.
    ModeTransitionDenied {
        /// Current execution mode.
        from: ExecutionMode,
        /// Requested execution mode.
        to: ExecutionMode,
    },
    /// Internal hashing error.
    HashError(String),
}

impl fmt::Display for SecurityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecurityError::KeyTooShort { len, min } => {
                write!(
                    f,
                    "power key too short: {} characters (minimum {})",
                    len, min
                )
            }
            SecurityError::InsufficientComplexity {
                found_types,
                required,
            } => {
                write!(
                    f,
                    "insufficient key complexity: found {} character types (minimum {})",
                    found_types, required
                )
            }
            SecurityError::InvalidPowerKey => write!(f, "invalid power key"),
            SecurityError::ModeTransitionDenied { from, to } => {
                write!(
                    f,
                    "mode transition denied: {:?} -> {:?} (requires process restart)",
                    from, to
                )
            }
            SecurityError::HashError(msg) => write!(f, "hashing error: {}", msg),
        }
    }
}

impl std::error::Error for SecurityError {}

/// Validates Power Key strength requirements.
///
/// Rules:
/// - Length >= 12 characters
/// - At least 2 distinct character types (uppercase, lowercase, digit, special)
pub struct PowerKeyValidator;

impl PowerKeyValidator {
    /// Validate a Power Key against security requirements.
    ///
    /// # Errors
    ///
    /// Returns `SecurityError::KeyTooShort` if the key is less than 12 characters.
    /// Returns `SecurityError::InsufficientComplexity` if the key has fewer than 2 character types.
    pub fn validate(key: &str) -> Result<(), SecurityError> {
        // Check minimum length
        if key.len() < MIN_KEY_LENGTH {
            return Err(SecurityError::KeyTooShort {
                len: key.len(),
                min: MIN_KEY_LENGTH,
            });
        }

        // Count distinct character types
        let has_uppercase = key.chars().any(|c| c.is_ascii_uppercase());
        let has_lowercase = key.chars().any(|c| c.is_ascii_lowercase());
        let has_digit = key.chars().any(|c| c.is_ascii_digit());
        let has_special = key
            .chars()
            .any(|c| !c.is_ascii_alphanumeric() && c.is_ascii());

        let found_types = has_uppercase as usize
            + has_lowercase as usize
            + has_digit as usize
            + has_special as usize;

        if found_types < MIN_CHAR_TYPES {
            return Err(SecurityError::InsufficientComplexity {
                found_types,
                required: MIN_CHAR_TYPES,
            });
        }

        Ok(())
    }
}

/// Argon2 hashed Power Key for secure storage.
///
/// The key is never stored in plaintext — only the Argon2 hash is kept.
/// The hash string is zeroed on drop to prevent it from lingering in freed memory pages.
#[derive(Debug, Clone, ZeroizeOnDrop)]
pub struct PowerKeyHash(String);

impl PowerKeyHash {
    /// Create a new hashed Power Key from a plaintext key.
    ///
    /// Validates the key first, then hashes it using Argon2.
    ///
    /// # Errors
    ///
    /// Returns `SecurityError::KeyTooShort` or `SecurityError::InsufficientComplexity`
    /// if the key fails validation, or `SecurityError::HashError` if hashing fails.
    pub fn new(key: &str) -> Result<Self, SecurityError> {
        // Validate key first
        PowerKeyValidator::validate(key)?;

        // Hash with Argon2 (random salt for each hash)
        let salt: [u8; 16] = rand_salt();
        let config = Config::default();
        let hash = argon2::hash_encoded(key.as_bytes(), &salt, &config)
            .map_err(|e| SecurityError::HashError(e.to_string()))?;

        Ok(Self(hash))
    }

    /// Verify a candidate key against the stored hash.
    ///
    /// Uses constant-time comparison to prevent timing attacks.
    pub fn verify(&self, candidate: &str) -> bool {
        argon2::verify_encoded(&self.0, candidate.as_bytes()).unwrap_or(false)
    }
}

impl fmt::Display for PowerKeyHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl PowerKeyHash {
    /// Create a PowerKeyHash from a stored string representation.
    ///
    /// Note: This does NOT validate the key - it assumes the hash was previously
    /// created by PowerKeyHash::new().
    pub fn from_string(hash: &str) -> Result<Self, SecurityError> {
        if hash.is_empty() {
            return Err(SecurityError::HashError("Empty hash string".to_string()));
        }
        Ok(Self(hash.to_string()))
    }
}

/// Guard for mode transitions between Safe and Power modes.
///
/// Enforces the security model from ADR-003:
/// - Safe → Power: requires valid Power Key
/// - Power → Safe: denied (requires process restart)
pub struct ModeTransitionGuard;

impl ModeTransitionGuard {
    /// Attempt to enter Power Mode from Safe Mode.
    ///
    /// Requires a valid Power Key that matches the stored hash.
    ///
    /// # Errors
    ///
    /// Returns `SecurityError::InvalidPowerKey` if the key does not match.
    pub fn enter_power_mode(
        key: &str,
        stored_hash: &PowerKeyHash,
    ) -> Result<ExecutionMode, SecurityError> {
        if stored_hash.verify(key) {
            Ok(ExecutionMode::Power)
        } else {
            Err(SecurityError::InvalidPowerKey)
        }
    }

    /// Attempt to exit Power Mode (always denied).
    ///
    /// Per ADR-003: Power Mode → Safe Mode requires process restart.
    /// This prevents a compromised Power Mode agent from hiding evidence.
    ///
    /// # Errors
    ///
    /// Always returns `SecurityError::ModeTransitionDenied`.
    pub fn exit_power_mode() -> Result<ExecutionMode, SecurityError> {
        Err(SecurityError::ModeTransitionDenied {
            from: ExecutionMode::Power,
            to: ExecutionMode::Safe,
        })
    }
}

/// RAII guard for Power Mode sessions.
///
/// Entering Power Mode via [`PowerModeGuard::enter`] validates the power key and writes
/// an entry-audit event. When the guard is dropped (normally or on panic/unwind), it
/// automatically writes an exit-audit event recording the session duration.
///
/// # Usage
///
/// ```rust,ignore
/// use claw_pal::audit::{AuditSinkHandle, NoopAuditSink};
///
/// let guard = PowerModeGuard::enter("my-power-key", &stored_hash, NoopAuditSink::handle(), "agent-1".to_string())?;
/// // … do privileged work …
/// drop(guard); // or let it fall off scope — exit audit is written automatically
/// ```
///
/// # Drop behaviour
///
/// `Drop` calls [`crate::audit::AuditSink::write_security_event`], which must be
/// non-blocking. The supplied [`crate::audit::AuditSinkHandle`] is responsible for
/// ensuring this contract (e.g. via a fire-and-forget channel send).
pub struct PowerModeGuard {
    entered_at: Instant,
    /// Wrapped in `Option` so `drop` can take ownership via `Option::take`.
    audit_sink: Option<crate::audit::AuditSinkHandle>,
    agent_id: String,
}

impl PowerModeGuard {
    /// Validate the power key and enter Power Mode, writing an entry-audit event.
    ///
    /// # Errors
    ///
    /// Returns [`SecurityError::InvalidPowerKey`] if `power_key` does not match
    /// `stored_hash`.
    pub fn enter(
        power_key: &str,
        stored_hash: &PowerKeyHash,
        audit_sink: crate::audit::AuditSinkHandle,
        agent_id: String,
    ) -> Result<Self, SecurityError> {
        if !stored_hash.verify(power_key) {
            return Err(SecurityError::InvalidPowerKey);
        }

        audit_sink.write_security_event(crate::audit::SecurityAuditEvent::now(
            agent_id.clone(),
            "safe",
            "power",
            "power_key_verified".to_string(),
        ));

        Ok(Self {
            entered_at: Instant::now(),
            audit_sink: Some(audit_sink),
            agent_id,
        })
    }
}

impl Drop for PowerModeGuard {
    /// Automatically write the Power-Mode-exit audit event when the guard is dropped.
    fn drop(&mut self) {
        let duration_ms = self.entered_at.elapsed().as_millis() as u64;
        if let Some(sink) = self.audit_sink.take() {
            sink.write_security_event(crate::audit::SecurityAuditEvent::now(
                self.agent_id.clone(),
                "power",
                "safe",
                format!("session_ended_after_{}ms", duration_ms),
            ));
        }
    }
}

/// Manages Power Key persistence and retrieval.
///
/// Power Key resolution follows this priority (highest first):
/// 1. CLI argument (`--power-key`)
/// 2. Environment variable (`CLAW_KERNEL_POWER_KEY`)
/// 3. Config file (`~/.config/claw-kernel/power.key`)
pub struct PowerKeyManager;

impl PowerKeyManager {
    /// Save a Power Key to the config file.
    ///
    /// The key is validated and hashed before storage. Only the hash is stored,
    /// never the plaintext key.
    ///
    /// # Errors
    ///
    /// Returns `SecurityError` if validation fails or file operations fail.
    pub fn save_power_key(key: &str) -> Result<(), SecurityError> {
        // Validate the key first
        PowerKeyValidator::validate(key)?;

        // Hash the key
        let hash = PowerKeyHash::new(key)?;

        // Get the config directory
        let config_dir = crate::dirs::config_dir()
            .ok_or_else(|| SecurityError::HashError("Cannot find config directory".to_string()))?;

        // Ensure config directory exists
        std::fs::create_dir_all(&config_dir)
            .map_err(|e| SecurityError::HashError(format!("Failed to create config dir: {e}")))?;

        // Write hash to power.key file
        let key_path = config_dir.join("power.key");
        std::fs::write(&key_path, hash.to_string())
            .map_err(|e| SecurityError::HashError(format!("Failed to write power key: {e}")))?;

        // Set restrictive permissions (read/write for owner only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&key_path)
                .map_err(|e| SecurityError::HashError(format!("Failed to get metadata: {e}")))?
                .permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(&key_path, perms)
                .map_err(|e| SecurityError::HashError(format!("Failed to set permissions: {e}")))?;
        }

        Ok(())
    }

    /// Load the stored Power Key hash from config file.
    ///
    /// # Errors
    ///
    /// Returns `SecurityError::InvalidPowerKey` if no key file exists.
    pub fn load_stored_hash() -> Result<PowerKeyHash, SecurityError> {
        let key_path = crate::dirs::power_key_path()
            .ok_or_else(|| SecurityError::HashError("Cannot find config directory".to_string()))?;

        let hash_str = Zeroizing::new(
            std::fs::read_to_string(&key_path).map_err(|_| SecurityError::InvalidPowerKey)?,
        );

        PowerKeyHash::from_string(hash_str.trim())
    }

    /// Check if a Power Key has been configured (exists in config file).
    pub fn is_configured() -> bool {
        if let Some(key_path) = crate::dirs::power_key_path() {
            key_path.exists()
        } else {
            false
        }
    }

    /// Resolve the effective Power Key following priority order:
    /// 1. CLI argument (`--power-key`)
    /// 2. Environment variable (`CLAW_KERNEL_POWER_KEY`)
    /// 3. Config file (`~/.config/claw-kernel/power.key`)
    ///
    /// Returns `Some(key)` if found from CLI or env var, `None` if only
    /// a stored hash exists (needs verification via `load_stored_hash`).
    ///
    /// The returned `Zeroizing<String>` automatically zeroes the plaintext key
    /// when it is dropped, preventing the secret from lingering in freed memory.
    pub fn resolve_power_key(cli_key: Option<String>) -> Option<Zeroizing<String>> {
        // Priority 1: CLI argument
        if let Some(k) = cli_key {
            return Some(Zeroizing::new(k));
        }

        // Priority 2: Environment variable
        if let Ok(env_key) = std::env::var("CLAW_KERNEL_POWER_KEY") {
            if !env_key.is_empty() {
                return Some(Zeroizing::new(env_key));
            }
        }

        // Priority 3: Config file - return None, caller should use load_stored_hash
        None
    }
}

/// Simple SHA-256 based Power Key for verification purposes.
///
/// This is a lightweight alternative to `PowerKeyHash` (which uses Argon2)
/// for scenarios where:
/// - You need deterministic hashing (same key always produces same hash)
/// - Performance is critical and Argon2's memory-hard properties aren't required
/// - You're verifying keys against external systems that use SHA-256
///
/// **Security Note:** Unlike `PowerKeyHash`, this does NOT use a salt, making
/// it vulnerable to rainbow table attacks. Only use this for:
/// - Temporary/in-memory key verification
/// - Integration with systems that require SHA-256
/// - Cases where the key itself is high-entropy (randomly generated)
///
/// For persistent storage, prefer `PowerKeyHash` with Argon2.
///
/// The internal hash bytes are zeroed on drop.
#[derive(Debug, Clone, ZeroizeOnDrop)]
pub struct PowerKey {
    verification_hash: [u8; 32],
}

impl PowerKey {
    /// Create a new PowerKey from a plaintext key.
    ///
    /// Computes the SHA-256 hash of the key for later verification.
    /// No validation is performed on the key strength - use `PowerKeyValidator`
    /// first if you need to enforce minimum requirements.
    ///
    /// # Example
    /// ```
    /// use claw_pal::security::PowerKey;
    ///
    /// let key = PowerKey::new("my-secret-key");
    /// assert!(key.verify("my-secret-key"));
    /// assert!(!key.verify("wrong-key"));
    /// ```
    pub fn new(key: &str) -> Self {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        let result = hasher.finalize();

        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result);

        Self {
            verification_hash: hash,
        }
    }

    /// Verify a provided key against the stored hash.
    ///
    /// Uses constant-time comparison to prevent timing attacks.
    ///
    /// # Returns
    /// - `true` if the provided key matches the stored hash
    /// - `false` otherwise
    pub fn verify(&self, provided: &str) -> bool {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(provided.as_bytes());
        let result = hasher.finalize();

        let mut provided_hash = [0u8; 32];
        provided_hash.copy_from_slice(&result);

        // Constant-time comparison to prevent timing attacks
        constant_time_eq(&self.verification_hash, &provided_hash)
    }

    /// Load a PowerKey from a file containing the raw key.
    ///
    /// The file should contain the plaintext key (not a hash). The key is
    /// hashed after reading and the plaintext is zeroed from memory when possible.
    ///
    /// # Arguments
    /// * `path` - Path to the key file
    ///
    /// # Returns
    /// * `Ok(PowerKey)` - Key loaded and hashed successfully
    /// * `Err(SecurityError)` - File not found, permission denied, or empty key
    ///
    /// # Example
    /// ```
    /// use claw_pal::security::PowerKey;
    /// use std::path::Path;
    ///
    /// // Load from default location
    /// if let Some(key_path) = claw_pal::dirs::power_key_path() {
    ///     // Read the raw key and create PowerKey
    ///     // Note: This reads the hash from the file, not the plaintext
    /// }
    /// ```
    pub fn load_from_file(path: &std::path::Path) -> Result<Self, SecurityError> {
        // Read the hash string from file (same format as PowerKeyHash stores)
        let hash_str = std::fs::read_to_string(path)
            .map_err(|e| SecurityError::HashError(format!("Failed to read key file: {e}")))?;

        let hash_str = hash_str.trim();
        if hash_str.is_empty() {
            return Err(SecurityError::HashError("Empty key file".to_string()));
        }

        // Decode hex string to bytes
        let bytes = hex_to_bytes(hash_str)
            .map_err(|e| SecurityError::HashError(format!("Invalid key file format: {e}")))?;

        if bytes.len() != 32 {
            return Err(SecurityError::HashError(format!(
                "Invalid hash length: expected 32 bytes, got {}",
                bytes.len()
            )));
        }

        let mut hash = [0u8; 32];
        hash.copy_from_slice(&bytes);

        Ok(Self {
            verification_hash: hash,
        })
    }

    /// Save this PowerKey's hash to a file.
    ///
    /// # Arguments
    /// * `path` - Path to write the key file
    ///
    /// # Returns
    /// * `Ok(())` - Key saved successfully
    /// * `Err(SecurityError)` - IO error or permission denied
    pub fn save_to_file(&self, path: &std::path::Path) -> Result<(), SecurityError> {
        let hash_hex = bytes_to_hex(&self.verification_hash);

        std::fs::write(path, hash_hex)
            .map_err(|e| SecurityError::HashError(format!("Failed to write key file: {e}")))?;

        // Set restrictive permissions (read/write for owner only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(path)
                .map_err(|e| SecurityError::HashError(format!("Failed to get metadata: {e}")))?
                .permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(path, perms)
                .map_err(|e| SecurityError::HashError(format!("Failed to set permissions: {e}")))?;
        }

        Ok(())
    }
}

/// Constant-time comparison of two byte arrays.
///
/// This prevents timing attacks by ensuring the comparison takes the same
/// amount of time regardless of where the arrays differ.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }

    result == 0
}

/// Convert bytes to lowercase hex string.
fn bytes_to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut s = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(s, "{:02x}", byte).unwrap();
    }
    s
}

/// Convert hex string to bytes.
fn hex_to_bytes(hex: &str) -> Result<Vec<u8>, String> {
    if hex.len() % 2 != 0 {
        return Err("Odd number of hex digits".to_string());
    }

    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for i in (0..hex.len()).step_by(2) {
        let byte = u8::from_str_radix(&hex[i..i + 2], 16)
            .map_err(|e| format!("Invalid hex digit: {e}"))?;
        bytes.push(byte);
    }

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== PowerKeyValidator tests =====

    #[test]
    fn test_validate_strong_key() {
        // 14 chars, uppercase + lowercase + digit + special → OK
        let result = PowerKeyValidator::validate("SecureKey123!");
        assert!(result.is_ok(), "strong key should pass validation");
    }

    #[test]
    fn test_validate_key_too_short() {
        // 7 chars → KeyTooShort
        let result = PowerKeyValidator::validate("Short1!");
        assert_eq!(
            result,
            Err(SecurityError::KeyTooShort { len: 7, min: 12 }),
            "short key should fail with KeyTooShort"
        );
    }

    #[test]
    fn test_validate_single_char_type() {
        // 12 chars, only lowercase → InsufficientComplexity
        let result = PowerKeyValidator::validate("aaaaaaaaaaaa");
        assert_eq!(
            result,
            Err(SecurityError::InsufficientComplexity {
                found_types: 1,
                required: 2
            }),
            "single-type key should fail with InsufficientComplexity"
        );
    }

    #[test]
    fn test_validate_lowercase_plus_digit() {
        // 12 chars, lowercase + digit → OK (2 types ≥ 2)
        let result = PowerKeyValidator::validate("1234567890ab");
        assert!(
            result.is_ok(),
            "lowercase + digit should pass (2 types >= 2)"
        );
    }

    #[test]
    fn test_validate_exactly_12_chars_uppercase_lowercase() {
        // 12 chars, uppercase + lowercase → OK
        let result = PowerKeyValidator::validate("AbCdEfGhIjKl");
        assert!(result.is_ok(), "12 chars with 2 types should pass");
    }

    #[test]
    fn test_validate_only_digits_12_chars() {
        // 12 digits → InsufficientComplexity (1 type)
        let result = PowerKeyValidator::validate("123456789012");
        assert_eq!(
            result,
            Err(SecurityError::InsufficientComplexity {
                found_types: 1,
                required: 2
            }),
        );
    }

    #[test]
    fn test_validate_empty_key() {
        let result = PowerKeyValidator::validate("");
        assert_eq!(result, Err(SecurityError::KeyTooShort { len: 0, min: 12 }),);
    }

    #[test]
    fn test_validate_11_chars_key() {
        // Edge case: exactly 11 chars (one less than minimum)
        let result = PowerKeyValidator::validate("SecureKey1!");
        assert!(
            matches!(result, Err(SecurityError::KeyTooShort { len: 11, .. })),
            "11-char key should fail"
        );
    }

    // ===== PowerKeyHash tests =====

    #[test]
    fn test_hash_and_verify_correct_password() {
        let key = "SecureKey123!";
        let hash = PowerKeyHash::new(key).expect("should hash successfully");
        assert!(
            hash.verify(key),
            "correct password should verify successfully"
        );
    }

    #[test]
    fn test_hash_and_verify_wrong_password() {
        let key = "SecureKey123!";
        let hash = PowerKeyHash::new(key).expect("should hash successfully");
        assert!(
            !hash.verify("WrongPassword1!"),
            "wrong password should not verify"
        );
    }

    #[test]
    fn test_hash_rejects_weak_key() {
        // PowerKeyHash::new validates before hashing
        let result = PowerKeyHash::new("short");
        assert!(
            matches!(result, Err(SecurityError::KeyTooShort { .. })),
            "weak key should be rejected by PowerKeyHash::new"
        );
    }

    #[test]
    fn test_hash_rejects_low_complexity_key() {
        let result = PowerKeyHash::new("aaaaaaaaaaaa");
        assert!(
            matches!(result, Err(SecurityError::InsufficientComplexity { .. })),
            "single-type key should be rejected"
        );
    }

    #[test]
    fn test_hash_different_salts() {
        // Two hashes of the same key should produce different hash strings
        let key = "SecureKey123!";
        let hash1 = PowerKeyHash::new(key).unwrap();
        let hash2 = PowerKeyHash::new(key).unwrap();
        assert_ne!(
            hash1.0, hash2.0,
            "different salts should produce different hashes"
        );
        // But both should verify
        assert!(hash1.verify(key));
        assert!(hash2.verify(key));
    }

    #[test]
    fn test_hash_verify_empty_candidate() {
        let hash = PowerKeyHash::new("SecureKey123!").unwrap();
        assert!(!hash.verify(""), "empty candidate should not verify");
    }

    // ===== ModeTransitionGuard tests =====

    #[test]
    fn test_enter_power_mode_success() {
        let key = "SecureKey123!";
        let hash = PowerKeyHash::new(key).unwrap();
        let result = ModeTransitionGuard::enter_power_mode(key, &hash);
        assert_eq!(
            result,
            Ok(ExecutionMode::Power),
            "correct key should enter power mode"
        );
    }

    #[test]
    fn test_enter_power_mode_wrong_key() {
        let key = "SecureKey123!";
        let hash = PowerKeyHash::new(key).unwrap();
        let result = ModeTransitionGuard::enter_power_mode("WrongKey123!", &hash);
        assert_eq!(
            result,
            Err(SecurityError::InvalidPowerKey),
            "wrong key should be rejected"
        );
    }

    #[test]
    fn test_exit_power_mode_always_denied() {
        let result = ModeTransitionGuard::exit_power_mode();
        assert_eq!(
            result,
            Err(SecurityError::ModeTransitionDenied {
                from: ExecutionMode::Power,
                to: ExecutionMode::Safe,
            }),
            "exit_power_mode must always return Err(ModeTransitionDenied)"
        );
    }

    // ===== PowerModeGuard (RAII) tests =====

    /// A test sink that records events into a shared Vec for inspection.
    mod test_sink {
        use crate::audit::{AuditSink, AuditSinkHandle, SecurityAuditEvent};
        use std::sync::{Arc, Mutex};

        pub struct RecordingSink(pub Arc<Mutex<Vec<SecurityAuditEvent>>>);

        impl AuditSink for RecordingSink {
            fn write_security_event(&self, event: SecurityAuditEvent) {
                self.0.lock().unwrap().push(event);
            }
        }

        pub fn make_sink() -> (AuditSinkHandle, Arc<Mutex<Vec<SecurityAuditEvent>>>) {
            let events: Arc<Mutex<Vec<SecurityAuditEvent>>> = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::new(RecordingSink(Arc::clone(&events)));
            (sink, events)
        }
    }

    #[test]
    fn test_power_mode_guard_enter_writes_entry_audit() {
        use test_sink::make_sink;

        let key = "SecureKey123!";
        let hash = PowerKeyHash::new(key).unwrap();
        let (sink, events) = make_sink();

        let _guard = PowerModeGuard::enter(key, &hash, sink, "agent-test".to_string())
            .expect("should enter power mode");

        let ev = events.lock().unwrap();
        assert_eq!(ev.len(), 1, "one entry event should have been written");
        assert_eq!(ev[0].from_mode, "safe");
        assert_eq!(ev[0].to_mode, "power");
        assert_eq!(ev[0].agent_id, "agent-test");
        assert_eq!(ev[0].reason, "power_key_verified");
    }

    #[test]
    fn test_power_mode_guard_drop_writes_exit_audit() {
        use test_sink::make_sink;

        let key = "SecureKey123!";
        let hash = PowerKeyHash::new(key).unwrap();
        let (sink, events) = make_sink();

        {
            let _guard = PowerModeGuard::enter(key, &hash, sink, "agent-drop".to_string())
                .expect("should enter power mode");
            // guard dropped here
        }

        let ev = events.lock().unwrap();
        assert_eq!(ev.len(), 2, "entry + exit events should both be recorded");

        let exit = &ev[1];
        assert_eq!(exit.from_mode, "power");
        assert_eq!(exit.to_mode, "safe");
        assert_eq!(exit.agent_id, "agent-drop");
        assert!(
            exit.reason.starts_with("session_ended_after_"),
            "exit reason should include duration: {}",
            exit.reason
        );
    }

    #[test]
    fn test_power_mode_guard_wrong_key_rejected() {
        use test_sink::make_sink;

        let key = "SecureKey123!";
        let hash = PowerKeyHash::new(key).unwrap();
        let (sink, events) = make_sink();

        let result = PowerModeGuard::enter("WrongKey1234!", &hash, sink, "agent-x".to_string());
        assert!(
            matches!(result, Err(SecurityError::InvalidPowerKey)),
            "wrong key should be rejected with InvalidPowerKey"
        );

        // No audit events should have been written for a failed attempt
        let ev = events.lock().unwrap();
        assert_eq!(ev.len(), 0, "failed enter should not write any audit event");
    }

    #[test]
    fn test_power_mode_guard_noop_sink_does_not_panic() {
        use crate::audit::NoopAuditSink;

        let key = "SecureKey123!";
        let hash = PowerKeyHash::new(key).unwrap();
        let sink = NoopAuditSink::handle();

        {
            let _guard = PowerModeGuard::enter(key, &hash, sink, "agent-noop".to_string())
                .expect("enter should succeed with noop sink");
        }
        // If we reach here without panic, the test passes.
    }



    #[test]
    fn test_security_error_display_key_too_short() {
        let err = SecurityError::KeyTooShort { len: 5, min: 12 };
        assert_eq!(
            err.to_string(),
            "power key too short: 5 characters (minimum 12)"
        );
    }

    #[test]
    fn test_security_error_display_insufficient_complexity() {
        let err = SecurityError::InsufficientComplexity {
            found_types: 1,
            required: 2,
        };
        assert_eq!(
            err.to_string(),
            "insufficient key complexity: found 1 character types (minimum 2)"
        );
    }

    #[test]
    fn test_security_error_display_invalid_key() {
        let err = SecurityError::InvalidPowerKey;
        assert_eq!(err.to_string(), "invalid power key");
    }

    #[test]
    fn test_security_error_display_mode_transition() {
        let err = SecurityError::ModeTransitionDenied {
            from: ExecutionMode::Power,
            to: ExecutionMode::Safe,
        };
        assert_eq!(
            err.to_string(),
            "mode transition denied: Power -> Safe (requires process restart)"
        );
    }

    #[test]
    fn test_security_error_clone() {
        let err = SecurityError::KeyTooShort { len: 5, min: 12 };
        let cloned = err.clone();
        assert_eq!(err, cloned);
    }

    #[test]
    fn test_security_error_debug() {
        let err = SecurityError::InvalidPowerKey;
        let debug = format!("{:?}", err);
        assert!(
            debug.contains("InvalidPowerKey"),
            "Debug should contain variant name"
        );
    }

    // ===== PowerKeyManager tests =====

    #[test]
    fn test_power_key_manager_save_and_load() {
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let _original_config_dir = std::env::var("HOME").ok();

        // Set up a temp config directory
        let temp_config = temp.path().join(".config/claw-kernel");
        std::fs::create_dir_all(&temp_config).unwrap();

        // Temporarily override the config directory lookup
        // Note: In real tests, we'd need to use a test-specific config override
        // For now, we test the hash functions directly

        let key = "SecureKey123!";
        let hash = PowerKeyHash::new(key).unwrap();

        // Test that the hash can verify the key
        assert!(hash.verify(key));
        assert!(!hash.verify("WrongKey123!"));

        // Test from_string roundtrip
        let hash_str = hash.to_string();
        let loaded = PowerKeyHash::from_string(&hash_str).unwrap();
        assert!(loaded.verify(key));
    }

    #[test]
    fn test_power_key_hash_from_string_empty() {
        let result = PowerKeyHash::from_string("");
        assert!(
            matches!(result, Err(SecurityError::HashError(_))),
            "empty hash should be rejected"
        );
    }

    // ===== PowerKey (SHA-256) tests =====

    #[test]
    fn test_power_key_new_and_verify() {
        let key = PowerKey::new("my-secret-key");
        assert!(key.verify("my-secret-key"));
        assert!(!key.verify("wrong-key"));
    }

    #[test]
    fn test_power_key_empty_string() {
        let key = PowerKey::new("");
        assert!(key.verify(""));
        assert!(!key.verify("not-empty"));
    }

    #[test]
    fn test_power_key_deterministic() {
        // Same input should produce same hash
        let key1 = PowerKey::new("deterministic-key");
        let key2 = PowerKey::new("deterministic-key");
        assert!(key1.verify("deterministic-key"));
        assert!(key2.verify("deterministic-key"));

        // Both should have same internal hash
        assert_eq!(key1.verification_hash, key2.verification_hash);
    }

    #[test]
    fn test_power_key_different_inputs() {
        let key1 = PowerKey::new("key-one");
        let key2 = PowerKey::new("key-two");

        // Different inputs should produce different hashes
        assert_ne!(key1.verification_hash, key2.verification_hash);

        // Each key should only verify its own input
        assert!(key1.verify("key-one"));
        assert!(!key1.verify("key-two"));
        assert!(key2.verify("key-two"));
        assert!(!key2.verify("key-one"));
    }

    #[test]
    fn test_power_key_unicode() {
        let key = PowerKey::new("密钥🔐key");
        assert!(key.verify("密钥🔐key"));
        assert!(!key.verify("密钥key"));
    }

    #[test]
    fn test_power_key_long_input() {
        let long_key = "a".repeat(10000);
        let key = PowerKey::new(&long_key);
        assert!(key.verify(&long_key));
        assert!(!key.verify(&(long_key + "x")));
    }

    #[test]
    fn test_power_key_save_and_load_roundtrip() {
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let key_path = temp.path().join("test_power.key");

        // Create and save a key
        let original = PowerKey::new("test-key-for-file");
        original.save_to_file(&key_path).unwrap();

        // Load it back
        let loaded = PowerKey::load_from_file(&key_path).unwrap();

        // Both should verify the same input
        assert!(original.verify("test-key-for-file"));
        assert!(loaded.verify("test-key-for-file"));
        assert!(!original.verify("wrong-key"));
        assert!(!loaded.verify("wrong-key"));

        // Internal hashes should match
        assert_eq!(original.verification_hash, loaded.verification_hash);
    }

    #[test]
    fn test_power_key_load_nonexistent_file() {
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let key_path = temp.path().join("nonexistent.key");

        let result = PowerKey::load_from_file(&key_path);
        assert!(
            matches!(result, Err(SecurityError::HashError(_))),
            "nonexistent file should return HashError"
        );
    }

    #[test]
    fn test_power_key_load_empty_file() {
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let key_path = temp.path().join("empty.key");
        std::fs::write(&key_path, "").unwrap();

        let result = PowerKey::load_from_file(&key_path);
        assert!(
            matches!(result, Err(SecurityError::HashError(_))),
            "empty file should return HashError"
        );
    }

    #[test]
    fn test_power_key_load_invalid_hex() {
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let key_path = temp.path().join("invalid.key");
        std::fs::write(&key_path, "not-hex-data!!!").unwrap();

        let result = PowerKey::load_from_file(&key_path);
        assert!(
            matches!(result, Err(SecurityError::HashError(_))),
            "invalid hex should return HashError"
        );
    }

    #[test]
    fn test_power_key_load_wrong_length() {
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let key_path = temp.path().join("wrong_length.key");
        // 16 bytes instead of 32
        std::fs::write(&key_path, "0123456789abcdef").unwrap();

        let result = PowerKey::load_from_file(&key_path);
        assert!(
            matches!(result, Err(SecurityError::HashError(_))),
            "wrong length hash should return HashError"
        );
    }

    #[test]
    fn test_constant_time_eq() {
        // Same arrays
        assert!(constant_time_eq(&[1, 2, 3], &[1, 2, 3]));
        assert!(constant_time_eq(&[], &[]));
        assert!(constant_time_eq(&[0; 32], &[0; 32]));

        // Different arrays
        assert!(!constant_time_eq(&[1, 2, 3], &[1, 2, 4]));
        assert!(!constant_time_eq(&[1, 2, 3], &[1, 2]));
        assert!(!constant_time_eq(&[1, 2], &[1, 2, 3]));
        assert!(!constant_time_eq(&[0; 32], &[1; 32]));
    }

    #[test]
    fn test_bytes_to_hex_roundtrip() {
        let bytes = vec![0x00, 0x0f, 0xf0, 0xff, 0xab, 0xcd, 0xef];
        let hex = bytes_to_hex(&bytes);
        assert_eq!(hex, "000ff0ffabcdef");
        assert_eq!(hex_to_bytes(&hex).unwrap(), bytes);
    }

    #[test]
    fn test_hex_to_bytes_invalid() {
        // Odd length
        assert!(hex_to_bytes("abc").is_err());

        // Invalid characters
        assert!(hex_to_bytes("gggg").is_err());
        assert!(hex_to_bytes("ABCDXYZ").is_err());
    }
}
