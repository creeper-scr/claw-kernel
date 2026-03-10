use crate::{
    error::MemoryError,
    types::{EpisodicEntry, EpisodicFilter, MemoryId, MemoryItem},
};
use async_trait::async_trait;

/// Persistent memory storage backend.
///
/// The concrete implementation uses SQLite with in-process cosine similarity search.
#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// Store a memory item. Returns the assigned ID.
    async fn store(&self, item: MemoryItem) -> Result<MemoryId, MemoryError>;

    /// Store multiple memory items in batch. Returns the assigned IDs.
    ///
    /// The default implementation stores items sequentially, but concrete
    /// implementations can override this with optimized batch operations.
    async fn store_batch(&self, items: Vec<MemoryItem>) -> Result<Vec<MemoryId>, MemoryError> {
        // Default implementation: store items one by one
        let mut ids = Vec::new();
        for item in items {
            ids.push(self.store(item).await?);
        }
        Ok(ids)
    }

    /// Retrieve a specific item by ID.
    async fn retrieve(&self, id: &MemoryId) -> Result<Option<MemoryItem>, MemoryError>;

    /// Search episodic history with a filter.
    async fn search_episodic(
        &self,
        filter: &EpisodicFilter,
    ) -> Result<Vec<EpisodicEntry>, MemoryError>;

    /// Searches memory by semantic similarity using cosine distance.
    ///
    /// ⚠️ **Performance Warning**: O(n) full-table scan — all embeddings are loaded
    /// into memory for comparison. Suitable for < 10,000 items.
    /// For larger datasets, consider using sqlite-vec (tracked in GitHub issue).
    ///
    /// # Arguments
    /// * `query` - The search query text
    /// * `namespace` - Optional namespace filter
    /// * `limit` - Maximum number of results to return
    ///
    /// Semantic search: find items whose embeddings are closest to the query vector.
    /// Returns up to `top_k` results ordered by similarity.
    async fn semantic_search(
        &self,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<MemoryItem>, MemoryError>;

    /// Delete a memory item.
    async fn delete(&self, id: &MemoryId) -> Result<(), MemoryError>;

    /// Clear all items in a namespace.
    async fn clear_namespace(&self, namespace: &str) -> Result<usize, MemoryError>;

    /// Total storage used by a namespace, in bytes (approximate).
    async fn namespace_usage(&self, namespace: &str) -> Result<u64, MemoryError>;

    /// BM25 关键词全文检索（基于 SQLite FTS5 内置 BM25 排名）。
    ///
    /// 返回内容与 `query` 关键词最相关的 `top_k` 条记忆，按 BM25 分数降序排列。
    /// 默认实现返回空列表（未实现 FTS5 的后端可选择不覆盖）。
    async fn keyword_search(
        &self,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<MemoryItem>, MemoryError> {
        let _ = (query, top_k);
        Ok(Vec::new())
    }

    /// 混合检索：`alpha * 向量相似度 + (1-alpha) * BM25 分数`。
    ///
    /// `alpha` 范围 \[0, 1\]，建议默认 0.7（向量权重更高）。
    /// 默认实现回退到纯语义搜索（忽略 `alpha` 和 BM25 部分）。
    async fn hybrid_search(
        &self,
        query: &str,
        query_embedding: &[f32],
        top_k: usize,
        alpha: f32,
    ) -> Result<Vec<MemoryItem>, MemoryError> {
        // Default: fall back to semantic search only
        let _ = (query, alpha);
        self.semantic_search(query_embedding, top_k).await
    }

    /// Store an item only if it would not exceed the namespace quota.
    ///
    /// ⚠️ Default implementation is NOT atomic. Override in production implementations.
    /// See `SecureMemoryStore` for the recommended atomic pattern.
    ///
    /// # Atomicity Note
    ///
    /// **The default implementation is not atomic.** It performs a read-check-write
    /// sequence (`namespace_usage()` → check → `store()`). Concurrent callers may
    /// each observe available quota and collectively exceed the limit.
    ///
    /// `SecureMemoryStore` overrides this with an internal mutex to provide
    /// practical exclusive access (though not transactional atomicity).
    ///
    /// For strict quota enforcement, wrap with `SecureMemoryStore` as the outermost layer.
    ///
    /// # Arguments
    /// * `item` - The memory item to store
    /// * `estimated_size` - The estimated byte size of the item
    /// * `quota_bytes` - The maximum allowed bytes for this namespace
    ///
    /// # Returns
    /// * `Ok(MemoryId)` - Item was stored successfully
    /// * `Err(MemoryError::QuotaExceeded)` - Item would exceed quota
    async fn store_with_quota_check(
        &self,
        item: MemoryItem,
        estimated_size: u64,
        quota_bytes: u64,
    ) -> Result<MemoryId, MemoryError> {
        // Default non-atomic implementation for backward compatibility
        // Concrete stores should override with atomic implementation
        let used = self.namespace_usage(&item.namespace).await?;
        if used.saturating_add(estimated_size) > quota_bytes {
            return Err(MemoryError::QuotaExceeded {
                namespace: item.namespace.clone(),
                used,
                limit: quota_bytes,
            });
        }
        self.store(item).await
    }
}

// ============================================================
// Tests
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MockStore(Mutex<HashMap<String, MemoryItem>>);

    impl MockStore {
        fn new() -> Self {
            MockStore(Mutex::new(HashMap::new()))
        }
    }

    #[async_trait]
    impl MemoryStore for MockStore {
        async fn store(&self, item: MemoryItem) -> Result<MemoryId, MemoryError> {
            let id = item.id.clone();
            self.0.lock().unwrap().insert(id.0.clone(), item);
            Ok(id)
        }

        async fn retrieve(&self, id: &MemoryId) -> Result<Option<MemoryItem>, MemoryError> {
            Ok(self.0.lock().unwrap().get(&id.0).cloned())
        }

        async fn search_episodic(
            &self,
            _: &EpisodicFilter,
        ) -> Result<Vec<EpisodicEntry>, MemoryError> {
            Ok(vec![])
        }

        async fn semantic_search(
            &self,
            _: &[f32],
            _: usize,
        ) -> Result<Vec<MemoryItem>, MemoryError> {
            Ok(vec![])
        }

        async fn delete(&self, id: &MemoryId) -> Result<(), MemoryError> {
            self.0.lock().unwrap().remove(&id.0);
            Ok(())
        }

        async fn clear_namespace(&self, _: &str) -> Result<usize, MemoryError> {
            Ok(0)
        }

        async fn namespace_usage(&self, _: &str) -> Result<u64, MemoryError> {
            Ok(0)
        }
    }

    #[tokio::test]
    async fn test_mock_store_store_and_retrieve() {
        let store = MockStore::new();
        let item = MemoryItem::new("agent-1", "important fact");
        let stored_id = store.store(item.clone()).await.unwrap();

        // ID must match original item's ID
        assert_eq!(stored_id, item.id);

        // Can retrieve the stored item
        let retrieved = store.retrieve(&stored_id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.content, "important fact");
        assert_eq!(retrieved.namespace, "agent-1");
    }

    #[tokio::test]
    async fn test_mock_store_delete() {
        let store = MockStore::new();
        let item = MemoryItem::new("agent-2", "temporary fact");
        let id = store.store(item).await.unwrap();

        // Exists before deletion
        assert!(store.retrieve(&id).await.unwrap().is_some());

        // Delete it
        store.delete(&id).await.unwrap();

        // Gone after deletion
        assert!(store.retrieve(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_mock_store_retrieve_nonexistent() {
        let store = MockStore::new();
        let fake_id = MemoryId::new("does-not-exist");
        let result = store.retrieve(&fake_id).await.unwrap();
        assert!(result.is_none());
    }
}
