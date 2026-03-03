use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::{
    config::MemorySecurityConfig,
    error::MemoryError,
    traits::MemoryStore,
    types::{EpisodicEntry, EpisodicFilter, MemoryId, MemoryItem},
};

/// Wraps any `MemoryStore` to enforce security policies.
///
/// **Safe Mode** (`MemorySecurityConfig::safe_mode()`):
/// - All write/read operations are restricted to `self.namespace`.
/// - A per-namespace byte quota is atomically checked before each `store()` call.
///
/// **Power Mode** (`MemorySecurityConfig::power_mode()`):
/// - No restrictions; all calls are forwarded unchanged.
pub struct SecureMemoryStore {
    inner: Arc<dyn MemoryStore>,
    config: MemorySecurityConfig,
    /// The owning namespace.  In Safe Mode every item is forced into this namespace.
    namespace: String,
    /// Lock to ensure atomic quota check and store operation.
    /// This protects against race conditions when multiple tasks attempt
    /// to store items concurrently while checking quota limits.
    quota_lock: Mutex<()>,
}

impl SecureMemoryStore {
    pub fn new(
        inner: Arc<dyn MemoryStore>,
        config: MemorySecurityConfig,
        namespace: impl Into<String>,
    ) -> Self {
        Self {
            inner,
            config,
            namespace: namespace.into(),
            quota_lock: Mutex::new(()),
        }
    }

    /// If isolation is enabled, rewrite `item.namespace` to `self.namespace`.
    fn enforce_namespace(&self, mut item: MemoryItem) -> MemoryItem {
        if self.config.namespace_isolation {
            item.namespace = self.namespace.clone();
        }
        item
    }

    /// Calculate the estimated byte size of a memory item.
    fn estimate_size(&self, item: &MemoryItem) -> u64 {
        item.content.len() as u64
            + item.namespace.len() as u64
            + item
                .embedding
                .as_ref()
                .map(|e| e.len() as u64 * 4)
                .unwrap_or(0)
    }
}

#[async_trait]
impl MemoryStore for SecureMemoryStore {
    // ------------------------------------------------------------------
    // store — ATOMIC quota check + namespace enforcement
    // ------------------------------------------------------------------
    async fn store(&self, item: MemoryItem) -> Result<MemoryId, MemoryError> {
        // Estimate the byte footprint of this item before writing.
        let estimated_size = self.estimate_size(&item);

        // Enforce namespace before checking quota
        let item = self.enforce_namespace(item);

        // Power Mode: skip quota check and lock
        if self.config.quota_bytes == u64::MAX {
            return self.inner.store(item).await;
        }

        // Safe Mode: acquire lock to ensure atomic quota check + store
        // This prevents race conditions when multiple concurrent stores
        // would each see available quota and exceed the limit collectively.
        let _guard = self.quota_lock.lock().await;

        // Use atomic quota check if available (SQLite backend)
        // This performs the check and store in a single database transaction
        self.inner
            .store_with_quota_check(item, estimated_size, self.config.quota_bytes)
            .await
    }

    // ------------------------------------------------------------------
    // retrieve — namespace filter in Safe Mode
    // ------------------------------------------------------------------
    async fn retrieve(&self, id: &MemoryId) -> Result<Option<MemoryItem>, MemoryError> {
        let result = self.inner.retrieve(id).await?;
        if self.config.namespace_isolation {
            Ok(result.filter(|item| item.namespace == self.namespace))
        } else {
            Ok(result)
        }
    }

    // ------------------------------------------------------------------
    // search_episodic — force namespace in Safe Mode
    // ------------------------------------------------------------------
    async fn search_episodic(
        &self,
        filter: &EpisodicFilter,
    ) -> Result<Vec<EpisodicEntry>, MemoryError> {
        if self.config.namespace_isolation {
            // Override (or set) the namespace in the filter.
            let mut f = filter.clone();
            f.namespace = Some(self.namespace.clone());
            self.inner.search_episodic(&f).await
        } else {
            self.inner.search_episodic(filter).await
        }
    }

    // ------------------------------------------------------------------
    // semantic_search — results filtered to own namespace in Safe Mode
    // ------------------------------------------------------------------
    async fn semantic_search(
        &self,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<MemoryItem>, MemoryError> {
        let results = self.inner.semantic_search(query_embedding, top_k).await?;
        if self.config.namespace_isolation {
            Ok(results
                .into_iter()
                .filter(|item| item.namespace == self.namespace)
                .collect())
        } else {
            Ok(results)
        }
    }

    // ------------------------------------------------------------------
    // delete — no special restriction (item ownership is by ID)
    // ------------------------------------------------------------------
    async fn delete(&self, id: &MemoryId) -> Result<(), MemoryError> {
        self.inner.delete(id).await
    }

    // ------------------------------------------------------------------
    // clear_namespace — in Safe Mode, only allow clearing own namespace
    // ------------------------------------------------------------------
    async fn clear_namespace(&self, namespace: &str) -> Result<usize, MemoryError> {
        if self.config.namespace_isolation && namespace != self.namespace {
            return Err(MemoryError::AccessDenied(format!(
                "cannot clear namespace '{namespace}'; only '{}' is allowed",
                self.namespace
            )));
        }
        self.inner.clear_namespace(namespace).await
    }

    // ------------------------------------------------------------------
    // namespace_usage — pass through (no restriction needed)
    // ------------------------------------------------------------------
    async fn namespace_usage(&self, namespace: &str) -> Result<u64, MemoryError> {
        self.inner.namespace_usage(namespace).await
    }
}

// ============================================================
// Tests
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::MemorySecurityConfig, sqlite::SqliteMemoryStore, types::MemoryItem};

    fn make_item_with_id(ns: &str, content: &str, id: &str) -> MemoryItem {
        let mut item = MemoryItem::new(ns, content);
        item.id = MemoryId::new(id);
        item
    }

    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_secure_store_safe_mode_namespace_isolation() {
        let inner = Arc::new(SqliteMemoryStore::in_memory().unwrap());
        let secure =
            SecureMemoryStore::new(inner.clone(), MemorySecurityConfig::safe_mode(), "agent-A");

        // Store item tagged for "agent-B" — should be rewritten to "agent-A"
        let item = make_item_with_id("agent-B", "secret", "iso-1");
        let id = secure.store(item).await.unwrap();

        // Retrieve from the underlying store to inspect the actual namespace
        let stored = inner.retrieve(&id).await.unwrap().unwrap();
        assert_eq!(stored.namespace, "agent-A", "namespace must be rewritten");
    }

    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_secure_store_quota_exceeded() {
        let inner = Arc::new(SqliteMemoryStore::in_memory().unwrap());

        // Set a quota of 1 byte — any real content exceeds it.
        // First store something so usage > 0.
        let item0 = make_item_with_id("quota-ns", "hello", "q0");
        inner.store(item0).await.unwrap();

        let config = MemorySecurityConfig {
            namespace_isolation: true,
            quota_bytes: 1, // 1 byte — immediately exceeded
            semantic_search_enabled: true,
            max_embedding_dims: 64,
        };
        let secure = SecureMemoryStore::new(inner, config, "quota-ns");

        let item = make_item_with_id("quota-ns", "overflow", "q1");
        let result = secure.store(item).await;
        assert!(
            matches!(result, Err(MemoryError::QuotaExceeded { .. })),
            "expected QuotaExceeded, got {result:?}"
        );
    }

    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_secure_store_power_mode_no_restriction() {
        let inner = Arc::new(SqliteMemoryStore::in_memory().unwrap());
        let secure =
            SecureMemoryStore::new(inner.clone(), MemorySecurityConfig::power_mode(), "agent-A");

        // Namespace is NOT rewritten in Power Mode.
        let item = make_item_with_id("agent-B", "data", "pow-1");
        secure.store(item).await.unwrap();

        let stored = inner
            .retrieve(&MemoryId::new("pow-1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.namespace, "agent-B",
            "power mode must not rewrite namespace"
        );
    }

    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_secure_store_retrieve_only_own_namespace() {
        let inner = Arc::new(SqliteMemoryStore::in_memory().unwrap());

        // Store one item in ns-A and one in ns-B directly.
        inner
            .store(make_item_with_id("ns-A", "mine", "my-item"))
            .await
            .unwrap();
        inner
            .store(make_item_with_id("ns-B", "not mine", "other-item"))
            .await
            .unwrap();

        let secure = SecureMemoryStore::new(inner, MemorySecurityConfig::safe_mode(), "ns-A");

        // Can retrieve own item
        let mine = secure.retrieve(&MemoryId::new("my-item")).await.unwrap();
        assert!(mine.is_some());

        // Cannot retrieve item from another namespace
        let other = secure.retrieve(&MemoryId::new("other-item")).await.unwrap();
        assert!(other.is_none(), "should not see items from other namespace");
    }

    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_secure_store_namespace_usage() {
        let inner = Arc::new(SqliteMemoryStore::in_memory().unwrap());
        let secure = SecureMemoryStore::new(inner, MemorySecurityConfig::safe_mode(), "usage-ns");

        let item = make_item_with_id("usage-ns", "content data", "u1");
        secure.store(item).await.unwrap();

        let usage = secure.namespace_usage("usage-ns").await.unwrap();
        assert!(usage >= "content data".len() as u64);
    }

    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_secure_store_quota_precheck() {
        // quota_bytes = 50, item.content = "a" * 100 → estimated_size = 100 + ns.len() > 50
        // → QuotaExceeded even though used == 0
        let inner = Arc::new(SqliteMemoryStore::in_memory().unwrap());
        let config = MemorySecurityConfig {
            namespace_isolation: true,
            quota_bytes: 50,
            semantic_search_enabled: true,
            max_embedding_dims: 64,
        };
        let secure = SecureMemoryStore::new(inner, config, "pre-ns");

        let big_content = "a".repeat(100);
        let item = make_item_with_id("pre-ns", &big_content, "pre-1");
        let result = secure.store(item).await;
        assert!(
            matches!(result, Err(MemoryError::QuotaExceeded { .. })),
            "expected QuotaExceeded (precheck), got {result:?}"
        );
    }

    // ------------------------------------------------------------------
    #[tokio::test]
    async fn test_atomic_quota_check_no_partial_write() {
        // Test that atomic quota check prevents partial writes
        // (i.e., if quota would be exceeded, nothing is written)
        let inner = Arc::new(SqliteMemoryStore::in_memory().unwrap());

        // First, store a small item to establish baseline usage
        let baseline = make_item_with_id("atomic-ns", "baseline", "base-1");
        inner.store(baseline).await.unwrap();

        // Set quota to be just barely exceeded by a large item
        let config = MemorySecurityConfig {
            namespace_isolation: true,
            quota_bytes: 50, // Very small quota
            semantic_search_enabled: true,
            max_embedding_dims: 64,
        };
        let secure = SecureMemoryStore::new(inner.clone(), config, "atomic-ns");

        // Try to store a large item that exceeds quota
        let big_content = "a".repeat(100); // 100 bytes > 50 quota
        let big_item = make_item_with_id("atomic-ns", &big_content, "big-1");
        let result = secure.store(big_item).await;

        // Should fail with QuotaExceeded
        assert!(
            matches!(result, Err(MemoryError::QuotaExceeded { .. })),
            "expected QuotaExceeded, got {result:?}"
        );

        // Verify the item was NOT partially written
        let not_stored = inner.retrieve(&MemoryId::new("big-1")).await.unwrap();
        assert!(
            not_stored.is_none(),
            "item that exceeds quota should not be stored"
        );

        // Verify baseline item still exists
        let baseline_still_there = inner.retrieve(&MemoryId::new("base-1")).await.unwrap();
        assert!(
            baseline_still_there.is_some(),
            "baseline item should still exist"
        );
    }
}
