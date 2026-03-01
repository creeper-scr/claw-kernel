use async_trait::async_trait;
use crate::{
    error::MemoryError,
    types::{EpisodicEntry, EpisodicFilter, MemoryId, MemoryItem},
};

/// Persistent memory storage backend.
///
/// The concrete implementation (Phase D) uses SQLite + sqlite-vec.
#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// Store a memory item. Returns the assigned ID.
    async fn store(&self, item: MemoryItem) -> Result<MemoryId, MemoryError>;

    /// Retrieve a specific item by ID.
    async fn retrieve(&self, id: &MemoryId) -> Result<Option<MemoryItem>, MemoryError>;

    /// Search episodic history with a filter.
    async fn search_episodic(
        &self,
        filter: &EpisodicFilter,
    ) -> Result<Vec<EpisodicEntry>, MemoryError>;

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
}

/// Local (synchronous) text embedding.
///
/// The concrete implementation (Phase D) uses n-gram character hashing.
/// No external API calls — must be fast and deterministic.
pub trait Embedder: Send + Sync {
    /// Embed a single text into a fixed-length vector.
    fn embed(&self, text: &str) -> Vec<f32>;

    /// Embed multiple texts (may be parallelized).
    fn embed_batch(&self, texts: &[&str]) -> Vec<Vec<f32>> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    /// Number of dimensions in the output vector.
    fn dimensions(&self) -> usize;
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
