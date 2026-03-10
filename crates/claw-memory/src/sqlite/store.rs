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
/// self-contained (no SQLite native extensions required for core functionality).
///
/// Note: The `sqlite-vec` dependency is available for future vector search
/// optimizations but is not currently active in v0.1.0.
pub struct SqliteMemoryStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteMemoryStore {
    /// Open (or create) a database at the given path.
    ///
    /// # Example
    ///
    /// ```rust
    /// use claw_memory::SqliteMemoryStore;
    /// use std::path::Path;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// // Create a temporary directory for the test
    /// let temp_dir = tempfile::tempdir()?;
    /// let db_path = temp_dir.path().join("memory.db");
    ///
    /// // Open or create the database
    /// let store = SqliteMemoryStore::open(&db_path)?;
    ///
    /// // The database is now ready to use
    /// // The temp_dir will be automatically cleaned up when it goes out of scope
    /// # Ok(())
    /// # }
    /// ```
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MemoryError> {
        let conn = Connection::open(path).map_err(|e| MemoryError::Storage(e.to_string()))?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.init_schema()?;
        Ok(store)
    }

    /// In-memory database – useful for tests.
    pub fn in_memory() -> Result<Self, MemoryError> {
        let conn = Connection::open_in_memory().map_err(|e| MemoryError::Storage(e.to_string()))?;
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

            CREATE VIRTUAL TABLE IF NOT EXISTS memory_items_fts USING fts5(
                content,
                content='memory_items',
                content_rowid='rowid',
                tokenize='unicode61'
            );

            CREATE TRIGGER IF NOT EXISTS memory_items_ai AFTER INSERT ON memory_items BEGIN
                INSERT INTO memory_items_fts(rowid, content) VALUES (new.rowid, new.content);
            END;

            CREATE TRIGGER IF NOT EXISTS memory_items_ad AFTER DELETE ON memory_items BEGIN
                INSERT INTO memory_items_fts(memory_items_fts, rowid, content) VALUES ('delete', old.rowid, old.content);
            END;

            CREATE TRIGGER IF NOT EXISTS memory_items_au AFTER UPDATE ON memory_items BEGIN
                INSERT INTO memory_items_fts(memory_items_fts, rowid, content) VALUES ('delete', old.rowid, old.content);
                INSERT INTO memory_items_fts(rowid, content) VALUES (new.rowid, new.content);
            END;
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
        tracing::warn!(len_a = a.len(), len_b = b.len(), "cosine_similarity: dimension mismatch, returning 0.0");
        return 0.0;
    }
    if a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    // FIX-10: use epsilon comparison instead of == 0.0 to handle floating-point
    // precision edge cases, and clamp result to [-1, 1] to guard against numerical drift.
    const EPSILON: f32 = 1e-10;
    if norm_a < EPSILON || norm_b < EPSILON {
        0.0
    } else {
        (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
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

        // FIX-29: propagate serialization errors instead of silently replacing with empty string.
        let embedding_json = item
            .embedding
            .as_ref()
            .map(|e| {
                serde_json::to_string(e)
                    .map_err(|err| MemoryError::Storage(format!("Failed to serialize embedding: {err}")))
            })
            .transpose()?;
        let tags_json = serde_json::to_string(&item.tags)
            .map_err(|e| MemoryError::Storage(format!("failed to serialize tags: {}", e)))?;

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
        // FIX-02: use parameterized binding for LIMIT to prevent any SQL injection risk.
        if filter.limit.is_some() {
            sql.push_str(" LIMIT ?");
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
        // FIX-02: LIMIT is now parameterized — append the value last.
        if let Some(lim) = filter.limit {
            param_values.push(Box::new(lim as i64));
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
    /// Searches memory by semantic similarity using cosine distance.
    ///
    /// ⚠️ **Performance Warning**: O(n) full-table scan — all embeddings are loaded
    /// into memory for comparison. Suitable for < 10,000 items.
    /// For larger datasets, consider using sqlite-vec (tracked in GitHub issue).
    ///
    /// # Arguments
    /// * `query_embedding` - The query vector to compare against stored embeddings
    /// * `top_k` - Maximum number of results to return
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

        // Filter out NaN scores before sorting to ensure stable ordering.
        scored.retain(|(score, _)| !score.is_nan());

        // Sort descending by similarity; NaN-safe via unwrap_or(Less) so any residual
        // NaN values sink to the bottom rather than producing undefined order.
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Less));
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

    // ------------------------------------------------------------------
    // store_with_quota_check - ATOMIC implementation
    // ------------------------------------------------------------------
    async fn store_with_quota_check(
        &self,
        item: MemoryItem,
        estimated_size: u64,
        quota_bytes: u64,
    ) -> Result<MemoryId, MemoryError> {
        let id = item.id.clone();
        let mut conn = self.conn.lock().unwrap();

        // Use a transaction for atomicity
        let tx = conn
            .transaction()
            .map_err(|e| MemoryError::Storage(e.to_string()))?;

        // Calculate current usage within the transaction
        let current_usage: i64 = tx
            .query_row(
                "SELECT COALESCE(SUM(LENGTH(content) + COALESCE(LENGTH(embedding), 0)), 0)
                 FROM memory_items WHERE namespace = ?1",
                params![&item.namespace],
                |row| row.get(0),
            )
            .map_err(|e| MemoryError::Storage(e.to_string()))?;

        // Check if adding this item would exceed quota
        let current_usage = current_usage as u64;
        if current_usage.saturating_add(estimated_size) > quota_bytes {
            return Err(MemoryError::QuotaExceeded {
                namespace: item.namespace.clone(),
                used: current_usage,
                limit: quota_bytes,
            });
        }

        // Within quota - proceed with insert
        // FIX-29: propagate serialization errors instead of silently replacing with empty string.
        let embedding_json = item
            .embedding
            .as_ref()
            .map(|e| {
                serde_json::to_string(e)
                    .map_err(|err| MemoryError::Storage(format!("Failed to serialize embedding: {err}")))
            })
            .transpose()?;
        let tags_json = serde_json::to_string(&item.tags)
            .map_err(|e| MemoryError::Storage(format!("failed to serialize tags: {}", e)))?;

        tx.execute(
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

        // Commit the transaction
        tx.commit()
            .map_err(|e| MemoryError::Storage(e.to_string()))?;

        Ok(id)
    }

    // ------------------------------------------------------------------
    // keyword_search — SQLite FTS5 BM25 全文检索
    // ------------------------------------------------------------------
    async fn keyword_search(
        &self,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<MemoryItem>, MemoryError> {
        // 对 FTS5 MATCH 查询中的特殊字符进行转义：
        // FTS5 把 '"' 用于短语查询，把 '*' 用于前缀查询等；
        // 直接包装成短语查询可以安全地处理任意输入。
        let safe_query = format!("\"{}\"", query.replace('"', "\"\""));

        let conn = self.conn.lock().unwrap();
        let top_k_i64 = top_k as i64;

        let mut stmt = conn
            .prepare(
                "SELECT mi.id, mi.namespace, mi.content, mi.embedding, mi.tags,
                        mi.created_at_ms, mi.accessed_at_ms, mi.importance
                 FROM memory_items mi
                 JOIN memory_items_fts fts ON mi.rowid = fts.rowid
                 WHERE memory_items_fts MATCH ?1
                 ORDER BY rank
                 LIMIT ?2",
            )
            .map_err(|e| MemoryError::Storage(e.to_string()))?;

        let items: Result<Vec<MemoryItem>, rusqlite::Error> = stmt
            .query_map(params![safe_query, top_k_i64], row_to_memory_item)
            .map_err(|e| MemoryError::Storage(e.to_string()))?
            .collect();

        items.map_err(|e| MemoryError::Storage(e.to_string()))
    }

    // ------------------------------------------------------------------
    // hybrid_search — 语义搜索 + BM25 分数融合
    // ------------------------------------------------------------------
    async fn hybrid_search(
        &self,
        query: &str,
        query_embedding: &[f32],
        top_k: usize,
        alpha: f32,
    ) -> Result<Vec<MemoryItem>, MemoryError> {
        // 1. 并行获取两路候选（各取 top_k * 2 保证融合后有足够候选）
        let fetch_k = top_k.saturating_mul(2).max(1);
        let semantic_results = self.semantic_search(query_embedding, fetch_k).await?;
        let keyword_results = self.keyword_search(query, fetch_k).await?;

        // 2. 用排名归一化（Reciprocal Rank Fusion 风格的简化版）
        //    排名越靠前归一化分数越高（1.0 → 0.0）
        use std::collections::HashMap;
        let mut scores: HashMap<String, f32> = HashMap::new();

        let sem_count = semantic_results.len() as f32;
        for (rank, item) in semantic_results.iter().enumerate() {
            let normalized = if sem_count > 1.0 {
                1.0 - (rank as f32 / (sem_count - 1.0))
            } else {
                1.0
            };
            scores.insert(item.id.0.clone(), alpha * normalized);
        }

        let kw_count = keyword_results.len() as f32;
        for (rank, item) in keyword_results.iter().enumerate() {
            let normalized = if kw_count > 1.0 {
                1.0 - (rank as f32 / (kw_count - 1.0))
            } else {
                1.0
            };
            let entry = scores.entry(item.id.0.clone()).or_insert(0.0);
            *entry += (1.0 - alpha) * normalized;
        }

        // 3. 合并两路结果并去重
        let mut seen = std::collections::HashSet::new();
        let mut combined: Vec<(f32, MemoryItem)> = semantic_results
            .into_iter()
            .chain(keyword_results)
            .filter_map(|item| {
                if seen.insert(item.id.0.clone()) {
                    let score = scores.get(&item.id.0).copied().unwrap_or(0.0);
                    Some((score, item))
                } else {
                    None
                }
            })
            .collect();

        // 4. 按混合分数降序排列，取 top_k
        combined.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        combined.truncate(top_k);

        Ok(combined.into_iter().map(|(_, item)| item).collect())
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

    let embedding: Option<Vec<f32>> =
        embedding_json.and_then(|json| serde_json::from_str(&json).ok());
    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();

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

    #[allow(dead_code)]
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
        store
            .store(make_item_with_id("ns-a", "item 1", "a1"))
            .await
            .unwrap();
        store
            .store(make_item_with_id("ns-a", "item 2", "a2"))
            .await
            .unwrap();
        store
            .store(make_item_with_id("ns-b", "item 3", "b1"))
            .await
            .unwrap();

        let deleted = store.clear_namespace("ns-a").await.unwrap();
        assert_eq!(deleted, 2);

        // ns-b item still exists
        assert!(store
            .retrieve(&MemoryId::new("b1"))
            .await
            .unwrap()
            .is_some());
        assert!(store
            .retrieve(&MemoryId::new("a1"))
            .await
            .unwrap()
            .is_none());
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
        let item =
            make_item_with_id("ns1", "embedded item", "emb-1").with_embedding(embedding.clone());

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
        let item = make_item_with_id("ns1", "relevant item", "sem-1").with_embedding(emb.clone());
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
        let close =
            make_item_with_id("ns1", "close", "close-1").with_embedding(vec![0.9f32, 0.1, 0.0]);
        // far match
        let far = make_item_with_id("ns1", "far", "far-1").with_embedding(vec![0.0f32, 0.0, 1.0]);

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
