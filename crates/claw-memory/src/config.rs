use serde::{Deserialize, Serialize};

/// Memory security configuration for Safe Mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySecurityConfig {
    /// Namespace isolation: each agent can only access its own namespace.
    pub namespace_isolation: bool,
    /// Per-namespace storage quota in bytes (default: 50 MB).
    pub quota_bytes: u64,
    /// Whether semantic search is enabled.
    pub semantic_search_enabled: bool,
    /// Maximum embedding dimensions to store.
    pub max_embedding_dims: usize,
}

impl Default for MemorySecurityConfig {
    fn default() -> Self {
        Self {
            namespace_isolation: true,
            quota_bytes: 50 * 1024 * 1024, // 50 MB
            semantic_search_enabled: true,
            max_embedding_dims: 64,
        }
    }
}

impl MemorySecurityConfig {
    pub fn safe_mode() -> Self {
        Self::default()
    }

    /// Power Mode: no restrictions (same struct, different values).
    pub fn power_mode() -> Self {
        Self {
            namespace_isolation: false,
            quota_bytes: u64::MAX,
            semantic_search_enabled: true,
            max_embedding_dims: 1024,
        }
    }
}

// ============================================================
// Tests
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_mode_config() {
        let cfg = MemorySecurityConfig::safe_mode();
        assert!(cfg.namespace_isolation);
        assert_eq!(cfg.quota_bytes, 50 * 1024 * 1024);
        assert!(cfg.semantic_search_enabled);
        assert_eq!(cfg.max_embedding_dims, 64);
    }

    #[test]
    fn test_power_mode_config() {
        let cfg = MemorySecurityConfig::power_mode();
        assert!(!cfg.namespace_isolation);
        assert_eq!(cfg.quota_bytes, u64::MAX);
        assert!(cfg.semantic_search_enabled);
        assert_eq!(cfg.max_embedding_dims, 1024);
    }
}
