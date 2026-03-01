//! Security module for claw-kernel.
//!
//! Provides PowerKey validation, Argon2 hashing, and mode transition guards.
//! Implements the dual-mode security model described in ADR-003.

use crate::traits::sandbox::ExecutionMode;
use argon2::Config;
use std::fmt;

/// Minimum key length for Power Key (2026 security standard).
pub const MIN_KEY_LENGTH: usize = 12;

/// Minimum number of distinct character types required (uppercase/lowercase/digit/special).
pub const MIN_CHAR_TYPES: usize = 2;

/// Generate a random 16-byte salt using OS entropy.
fn rand_salt() -> [u8; 16] {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut salt = [0u8; 16];
    // Use two different RandomState instances for 128 bits of entropy
    let s1 = RandomState::new();
    let s2 = RandomState::new();
    let h1 = s1.build_hasher().finish().to_ne_bytes();
    let h2 = s2.build_hasher().finish().to_ne_bytes();
    salt[..8].copy_from_slice(&h1);
    salt[8..].copy_from_slice(&h2);
    salt
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
#[derive(Debug, Clone)]
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

    // ===== SecurityError display tests =====

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
}
