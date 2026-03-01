use async_trait::async_trait;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::{
    error::MemoryError,
    traits::MemoryStore,
    types::{EpisodeId, EpisodicEntry, EpisodicFilter, MemoryId, MemoryItem},
};

/// SQLite-backed memory store.
///
/// All rows are keyed by a text ID.  Embedding vectors are stored as JSON
/// arrays in a TEXT column; cosine similarity is computed in-process when a
/// semantic search is requested, making the implementation entirely
/// self-contained with no native extensions.
pub struct SqliteMemoryStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteMemoryStore {
    /// Open (or create) a database at the given path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MemoryError> {
        let conn =
            Connection::open(path).map_err(|e| MemoryError::Storage(e.to_string()))?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.init_schema()?;
        Ok(store)
    }

    /// In-memory database – useful for tests.
    pub fn in_memory() -> Result<Self, MemoryError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| MemoryError::Storage(e.to_string()))?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<(), MemoryError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS memory_items (
                id              TEXT PRIMARY KEY,
                namespace       TEXT NOT NULL,
                content         TEXT NOT NULL,
                embedding       TEXT,           -- JSON array of f32, or NULL
                tags            TEXT NOT NULL,  -- JSON array of strings
                created_at_ms   INTEGER NOT NULL,
                accessed_at_ms  INTEGER NOT NULL,
                importance      REAL NOT NULL DEFAULT 0.5
            );

            CREATE TABLE IF NOT EXISTS episodic_entries (
                id              TEXT PRIMARY KEY,
                episode_id      TEXT NOT NULL,
                namespace       TEXT NOT NULL,
                role            TEXT NOT NULL,
                content         TEXT NOT NULL,
                timestamp_ms    INTEGER NOT NULL,
                turn_index      INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_memory_namespace
                ON memory_items(namespace);
            CREATE INDEX IF NOT EXISTS idx_episodic_namespace
                ON episodic_entries(namespace);
            CREATE INDEX IF NOT EXISTS idx_episodic_episode
                ON episodic_entries(episode_id);
            ",
        )
        .map_err(|e| MemoryError::Storage(e.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Internal helper
// ---------------------------------------------------------------------------

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

// ---------------------------------------------------------------------------
// MemoryStore implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl MemoryStore for SqliteMemoryStore {
    // ------------------------------------------------------------------
    // store
    // ------------------------------------------------------------------
    async fn store(&self, item: MemoryItem) -> Result<MemoryId, MemoryError> {
        let id = item.id.clone();
        let conn = self.conn.lock().unwrap();

        let embedding_json = item
            .embedding
            .as_ref()
            .map(|e| serde_json::to_string(e).unwrap_or_default());
        let tags_json =
            serde_json::to_string(&item.tags).unwrap_or_else(|_| "[]".to_string());

        conn.execute(
            "INSERT OR REPLACE INTO memory_items
                 (id, namespace, content, embedding, tags,
                  created_at_ms, accessed_at_ms, importance)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                item.id.0,
                item.namespace,
                item.content,
                embedding_json,
                tags_json,
                item.created_at_ms as i64,
                item.accessed_at_ms as i64,
                item.importance,
            ],
        )
        .map_err(|e| MemoryError::Storage(e.to_string()))?;

        Ok(id)
    }

    // ------------------------------------------------------------------
    // retrieve
    // ------------------------------------------------------------------
    async fn retrieve(&self, id: &MemoryId) -> Result<Option<MemoryItem>, MemoryError> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn
            .prepare(
                "SELECT id, namespace, content, embedding, tags,
                        created_at_ms, accessed_at_ms, importance
                 FROM memory_items WHERE id = ?1",
            )
            .map_err(|e| MemoryError::Storage(e.to_string()))?;

        let mut rows = stmt
            .query_map(params![id.0], row_to_memory_item)
            .map_err(|e| MemoryError::Storage(e.to_string()))?;

        match rows.next() {
            None => Ok(None),
            Some(result) => {
                let item = result.map_err(|e| MemoryError::Storage(e.to_string()))?;
                Ok(Some(item))
            }
        }
    }

    // ------------------------------------------------------------------
    // search_episodic
    // ------------------------------------------------------------------
    async fn search_episodic(
        &self,
        filter: &EpisodicFilter,
    ) -> Result<Vec<EpisodicEntry>, MemoryError> {
        let conn = self.conn.lock().unwrap();

        // Build query dynamically based on which filter fields are set.
        let mut sql = String::from(
            "SELECT id, episode_id, namespace, role, content, timestamp_ms, turn_index
             FROM episodic_entries WHERE 1=1",
        );
        let mut conditions: Vec<String> = Vec::new();

        if filter.namespace.is_some() {
            conditions.push("namespace = ?".to_string());
        }
        if filter.episode_id.is_some() {
            conditions.push("episode_id = ?".to_string());
        }
        if filter.after_ms.is_some() {
            conditions.push("timestamp_ms >= ?".to_string());
        }
        if filter.before_ms.is_some() {
            conditions.push("timestamp_ms <= ?".to_string());
        }

        for cond in &conditions {
            sql.push_str(" AND ");
            sql.push_str(cond);
        }
        sql.push_str(" ORDER BY timestamp_ms ASC");
        if let Some(lim) = filter.limit {
            sql.push_str(&format!(" LIMIT {}", lim));
        }

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| MemoryError::Storage(e.to_string()))?;

        // Collect positional parameter values in order.
        let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(ref ns) = filter.namespace {
            param_values.push(Box::new(ns.clone()));
        }
        if let Some(ref ep) = filter.episode_id {
            param_values.push(Box::new(ep.0.clone()));
        }
        if let Some(after) = filter.after_ms {
            param_values.push(Box::new(after as i64));
        }
        if let Some(before) = filter.before_ms {
            param_values.push(Box::new(before as i64));
        }

        // rusqlite does not support dynamic heterogeneous params directly;
        // we use the `params_from_iter` helper with boxed ToSql values.
        let rows: Result<Vec<EpisodicEntry>, rusqlite::Error> = stmt
            .query_map(
                rusqlite::params_from_iter(param_values.iter().map(|b| b.as_ref())),
                |row| {
                    let id_str: String = row.get(0)?;
                    let ep_id_str: String = row.get(1)?;
                    let namespace: String = row.get(2)?;
                    let role: String = row.get(3)?;
                    let content: String = row.get(4)?;
                    let timestamp_ms: i64 = row.get(5)?;
                    let turn_index: i64 = row.get(6)?;
                    Ok(EpisodicEntry {
                        id: MemoryId::new(id_str),
                        episode_id: EpisodeId::new(ep_id_str),
                        namespace,
                        role,
                        content,
                        timestamp_ms: timestamp_ms as u64,
                        turn_index: turn_index as u32,
                    })
                },
            )
            .map_err(|e| MemoryError::Storage(e.to_string()))?
            .collect();

        rows.map_err(|e| MemoryError::Storage(e.to_string()))
    }

    // ------------------------------------------------------------------
    // semantic_search
    // ------------------------------------------------------------------
    async fn semantic_search(
        &self,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<MemoryItem>, MemoryError> {
        let conn = self.conn.lock().unwrap();

        // Load all items that have an embedding stored.
        let mut stmt = conn
            .prepare(
                "SELECT id, namespace, content, embedding, tags,
                        created_at_ms, accessed_at_ms, importance
                 FROM memory_items WHERE embedding IS NOT NULL",
            )
            .map_err(|e| MemoryError::Storage(e.to_string()))?;

        let items: Result<Vec<MemoryItem>, rusqlite::Error> = stmt
            .query_map([], row_to_memory_item)
            .map_err(|e| MemoryError::Storage(e.to_string()))?
            .collect();

        let items = items.map_err(|e| MemoryError::Storage(e.to_string()))?;

        // Compute cosine similarity in process and pick the top-k.
        let mut scored: Vec<(f32, MemoryItem)> = items
            .into_iter()
            .filter_map(|item| {
                let emb = item.embedding.as_ref()?;
                let sim = cosine_similarity(query_embedding, emb);
                Some((sim, item))
            })
            .collect();

        // Sort descending by similarity.
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);

        Ok(scored.into_iter().map(|(_, item)| item).collect())
    }

    // ------------------------------------------------------------------
    // delete
    // ------------------------------------------------------------------
    async fn delete(&self, id: &MemoryId) -> Result<(), MemoryError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM memory_items WHERE id = ?1", params![id.0])
            .map_err(|e| MemoryError::Storage(e.to_string()))?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // clear_namespace
    // ------------------------------------------------------------------
    async fn clear_namespace(&self, namespace: &str) -> Result<usize, MemoryError> {
        let conn = self.conn.lock().unwrap();
        let count = conn
            .execute(
                "DELETE FROM memory_items WHERE namespace = ?1",
                params![namespace],
            )
            .map_err(|e| MemoryError::Storage(e.to_string()))?;
        Ok(count)
    }

    // ------------------------------------------------------------------
    // namespace_usage
    // ------------------------------------------------------------------
    async fn namespace_usage(&self, namespace: &str) -> Result<u64, MemoryError> {
        let conn = self.conn.lock().unwrap();
        let total: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(LENGTH(content) + COALESCE(LENGTH(embedding), 0)), 0)
                 FROM memory_items WHERE namespace = ?1",
                params![namespace],
                |row| row.get(0),
            )
            .map_err(|e| MemoryError::Storage(e.to_string()))?;
        Ok(total as u64)
    }
}

// ---------------------------------------------------------------------------
// Row-mapping helper
// ---------------------------------------------------------------------------

fn row_to_memory_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryItem> {
    let id_str: String = row.get(0)?;
    let namespace: String = row.get(1)?;
    let content: String = row.get(2)?;
    let embedding_json: Option<String> = row.get(3)?;
    let tags_json: String = row.get(4)?;
    let created_at_ms: i64 = row.get(5)?;
    let accessed_at_ms: i64 = row.get(6)?;
    let importance: f32 = row.get(7)?;

    let embedding: Option<Vec<f32>> = embedding_json.and_then(|json| {
        serde_json::from_str(&json).ok()
    });
    let tags: Vec<String> =
        serde_json::from_str(&tags_json).unwrap_or_default();

    Ok(MemoryItem {
        id: MemoryId::new(id_str),
        namespace,
        content,
        embedding,
        tags,
        created_at_ms: created_at_ms as u64,
        accessed_at_ms: accessed_at_ms as u64,
        importance,
    })
}

// ============================================================
// Tests
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EpisodeId, EpisodicEntry, EpisodicFilter, MemoryItem};

    fn make_item(ns: &str, content: &str) -> MemoryItem {
        MemoryItem::new(ns, content)
    }

    fn make_item_with_id(ns: &str, content: &str, id: &str) -> MemoryItem {
        let mut item = MemoryItem::new(ns, content);
        item.id = MemoryId::new(id);
        item
    }

    // Helper: insert an episodic entry directly
    fn insert_episodic(store: &SqliteMemoryStore, entry: &EpisodicEntry) {
        let conn = store.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO episodic_entries
             (id, episode_id, namespace, role, content, timestamp_ms, turn_index)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry.id.0,
                entry.episode_id.0,
                entry.namespace,
                entry.role,
                entry.content,
                entry.timestamp_ms as i64,
                entry.turn_index as i64,
            ],
        )
        .unwrap();
    }

    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_sqlite_store_in_memory() {
        let store = SqliteMemoryStore::in_memory().unwrap();
        // Just verifying it opens without error and schema exists.
        let usage = store.namespace_usage("test-ns").await.unwrap();
        assert_eq!(usage, 0);
    }

    #[tokio::test]
    async fn test_sqlite_store_store_and_retrieve() {
        let store = SqliteMemoryStore::in_memory().unwrap();
        let item = make_item_with_id("ns1", "hello world", "id-001");
        let id = store.store(item.clone()).await.unwrap();

        assert_eq!(id.0, "id-001");

        let retrieved = store.retrieve(&id).await.unwrap().unwrap();
        assert_eq!(retrieved.content, "hello world");
        assert_eq!(retrieved.namespace, "ns1");
    }

    #[tokio::test]
    async fn test_sqlite_store_retrieve_nonexistent() {
        let store = SqliteMemoryStore::in_memory().unwrap();
        let result = store.retrieve(&MemoryId::new("no-such-id")).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_sqlite_store_delete() {
        let store = SqliteMemoryStore::in_memory().unwrap();
        let item = make_item_with_id("ns1", "to delete", "del-001");
        let id = store.store(item).await.unwrap();

        assert!(store.retrieve(&id).await.unwrap().is_some());
        store.delete(&id).await.unwrap();
        assert!(store.retrieve(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_sqlite_store_clear_namespace() {
        let store = SqliteMemoryStore::in_memory().unwrap();
        store.store(make_item_with_id("ns-a", "item 1", "a1")).await.unwrap();
        store.store(make_item_with_id("ns-a", "item 2", "a2")).await.unwrap();
        store.store(make_item_with_id("ns-b", "item 3", "b1")).await.unwrap();

        let deleted = store.clear_namespace("ns-a").await.unwrap();
        assert_eq!(deleted, 2);

        // ns-b item still exists
        assert!(store.retrieve(&MemoryId::new("b1")).await.unwrap().is_some());
        assert!(store.retrieve(&MemoryId::new("a1")).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_sqlite_store_namespace_usage() {
        let store = SqliteMemoryStore::in_memory().unwrap();
        store
            .store(make_item_with_id("ns1", "some content here", "u1"))
            .await
            .unwrap();
        let usage = store.namespace_usage("ns1").await.unwrap();
        // Must be at least the length of the content string
        assert!(usage >= "some content here".len() as u64);

        let zero = store.namespace_usage("empty-ns").await.unwrap();
        assert_eq!(zero, 0);
    }

    #[tokio::test]
    async fn test_sqlite_store_search_episodic_empty() {
        let store = SqliteMemoryStore::in_memory().unwrap();
        let results = store
            .search_episodic(&EpisodicFilter::default())
            .await
            .unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_sqlite_store_store_with_embedding() {
        let store = SqliteMemoryStore::in_memory().unwrap();
        let embedding = vec![0.1f32, 0.2, 0.3, 0.4];
        let item = make_item_with_id("ns1", "embedded item", "emb-1")
            .with_embedding(embedding.clone());

        store.store(item).await.unwrap();

        let retrieved = store
            .retrieve(&MemoryId::new("emb-1"))
            .await
            .unwrap()
            .unwrap();
        let stored_emb = retrieved.embedding.unwrap();
        assert_eq!(stored_emb.len(), 4);
        for (a, b) in stored_emb.iter().zip(embedding.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[tokio::test]
    async fn test_sqlite_store_semantic_search_basic() {
        let store = SqliteMemoryStore::in_memory().unwrap();

        let emb = vec![1.0f32, 0.0, 0.0, 0.0];
        let item = make_item_with_id("ns1", "relevant item", "sem-1")
            .with_embedding(emb.clone());
        store.store(item).await.unwrap();

        let results = store.semantic_search(&emb, 5).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.0, "sem-1");
    }

    #[tokio::test]
    async fn test_sqlite_store_semantic_search_ordering() {
        let store = SqliteMemoryStore::in_memory().unwrap();

        // query vector
        let query = vec![1.0f32, 0.0, 0.0];

        // close match
        let close = make_item_with_id("ns1", "close", "close-1")
            .with_embedding(vec![0.9f32, 0.1, 0.0]);
        // far match
        let far = make_item_with_id("ns1", "far", "far-1")
            .with_embedding(vec![0.0f32, 0.0, 1.0]);

        store.store(close).await.unwrap();
        store.store(far).await.unwrap();

        let results = store.semantic_search(&query, 2).await.unwrap();
        assert_eq!(results.len(), 2);
        // The close match should come first
        assert_eq!(results[0].id.0, "close-1");
    }

    #[tokio::test]
    async fn test_sqlite_store_multiple_items() {
        let store = SqliteMemoryStore::in_memory().unwrap();

        for i in 0..5u32 {
            let item = make_item_with_id("ns1", &format!("item {i}"), &format!("id-{i}"));
            store.store(item).await.unwrap();
        }

        for i in 0..5u32 {
            let item = store
                .retrieve(&MemoryId::new(format!("id-{i}")))
                .await
                .unwrap();
            assert!(item.is_some(), "item id-{i} should exist");
            assert_eq!(item.unwrap().content, format!("item {i}"));
        }
    }

    #[tokio::test]
    async fn test_sqlite_store_override_existing() {
        let store = SqliteMemoryStore::in_memory().unwrap();

        let item1 = make_item_with_id("ns1", "original", "dup-1");
        store.store(item1).await.unwrap();

        // Replace with same id, different content
        let item2 = make_item_with_id("ns1", "replaced", "dup-1");
        store.store(item2).await.unwrap();

        let retrieved = store
            .retrieve(&MemoryId::new("dup-1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retrieved.content, "replaced");
    }

    #[tokio::test]
    async fn test_sqlite_store_search_episodic_with_filter() {
        let store = SqliteMemoryStore::in_memory().unwrap();

        let entry1 = EpisodicEntry {
            id: MemoryId::new("ep-e1"),
            episode_id: EpisodeId::new("ep-1"),
            namespace: "ns1".to_string(),
            role: "user".to_string(),
            content: "hello".to_string(),
            timestamp_ms: 1000,
            turn_index: 0,
        };
        let entry2 = EpisodicEntry {
            id: MemoryId::new("ep-e2"),
            episode_id: EpisodeId::new("ep-1"),
            namespace: "ns1".to_string(),
            role: "assistant".to_string(),
            content: "world".to_string(),
            timestamp_ms: 2000,
            turn_index: 1,
        };
        let entry3 = EpisodicEntry {
            id: MemoryId::new("ep-e3"),
            episode_id: EpisodeId::new("ep-2"),
            namespace: "ns2".to_string(),
            role: "user".to_string(),
            content: "other".to_string(),
            timestamp_ms: 3000,
            turn_index: 0,
        };

        insert_episodic(&store, &entry1);
        insert_episodic(&store, &entry2);
        insert_episodic(&store, &entry3);

        // Filter by namespace
        let filter = EpisodicFilter::default().for_namespace("ns1");
        let results = store.search_episodic(&filter).await.unwrap();
        assert_eq!(results.len(), 2);

        // Filter by namespace + limit
        let filter2 = EpisodicFilter::default().for_namespace("ns1").limit(1);
        let results2 = store.search_episodic(&filter2).await.unwrap();
        assert_eq!(results2.len(), 1);
    }
}
