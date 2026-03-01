use serde::{Deserialize, Serialize};

/// Unique identifier for a memory item.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemoryId(pub String);

impl MemoryId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MemoryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Unique identifier for an episodic memory session/episode.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EpisodeId(pub String);

impl EpisodeId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A single memory item (working memory or long-term).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: MemoryId,
    /// Agent or namespace that owns this memory.
    pub namespace: String,
    /// Memory content (text or serialized JSON).
    pub content: String,
    /// Optional embedding vector (None if not yet embedded).
    pub embedding: Option<Vec<f32>>,
    /// Tags for filtering.
    pub tags: Vec<String>,
    /// Creation timestamp (Unix ms).
    pub created_at_ms: u64,
    /// Last accessed timestamp.
    pub accessed_at_ms: u64,
    /// Importance score (0.0–1.0). Higher = more likely to be retained.
    pub importance: f32,
}

impl MemoryItem {
    pub fn new(namespace: impl Into<String>, content: impl Into<String>) -> Self {
        let now_ms = current_time_ms();
        Self {
            id: MemoryId::new(format!("mem-{}", now_ms)),
            namespace: namespace.into(),
            content: content.into(),
            embedding: None,
            tags: Vec::new(),
            created_at_ms: now_ms,
            accessed_at_ms: now_ms,
            importance: 0.5,
        }
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
    pub fn with_importance(mut self, score: f32) -> Self {
        self.importance = score.clamp(0.0, 1.0);
        self
    }
    pub fn with_embedding(mut self, v: Vec<f32>) -> Self {
        self.embedding = Some(v);
        self
    }
}

fn current_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// An episodic memory entry (archived conversation turn).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodicEntry {
    pub id: MemoryId,
    pub episode_id: EpisodeId,
    pub namespace: String,
    /// Role: "user" or "assistant".
    pub role: String,
    pub content: String,
    pub timestamp_ms: u64,
    pub turn_index: u32,
}

/// Filter for querying episodic memory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EpisodicFilter {
    /// Filter by episode ID.
    pub episode_id: Option<EpisodeId>,
    /// Filter by namespace.
    pub namespace: Option<String>,
    /// Only entries after this timestamp.
    pub after_ms: Option<u64>,
    /// Only entries before this timestamp.
    pub before_ms: Option<u64>,
    /// Maximum results to return.
    pub limit: Option<usize>,
}

impl EpisodicFilter {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn for_namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = Some(ns.into());
        self
    }
    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }
}

// ============================================================
// Tests
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_id_new() {
        let id = MemoryId::new("abc-123");
        assert_eq!(id.as_str(), "abc-123");
        assert_eq!(id.to_string(), "abc-123");
        // Equality
        let id2 = MemoryId::new("abc-123");
        assert_eq!(id, id2);
    }

    #[test]
    fn test_memory_item_new() {
        let item = MemoryItem::new("agent-1", "hello world");
        assert_eq!(item.namespace, "agent-1");
        assert_eq!(item.content, "hello world");
        assert!(item.id.as_str().starts_with("mem-"));
        assert!(item.embedding.is_none());
        assert!(item.tags.is_empty());
        assert_eq!(item.importance, 0.5);
        assert!(item.created_at_ms > 0);
        assert_eq!(item.created_at_ms, item.accessed_at_ms);
    }

    #[test]
    fn test_memory_item_builder() {
        let embedding = vec![0.1_f32, 0.2, 0.3];
        let item = MemoryItem::new("ns", "content")
            .with_tags(vec!["tag1".to_string(), "tag2".to_string()])
            .with_importance(0.9)
            .with_embedding(embedding.clone());

        assert_eq!(item.tags, vec!["tag1", "tag2"]);
        assert!((item.importance - 0.9).abs() < 1e-6);
        assert_eq!(item.embedding, Some(embedding));
    }

    #[test]
    fn test_episodic_filter_default() {
        let f = EpisodicFilter::default();
        assert!(f.episode_id.is_none());
        assert!(f.namespace.is_none());
        assert!(f.after_ms.is_none());
        assert!(f.before_ms.is_none());
        assert!(f.limit.is_none());

        // Builder helpers
        let f2 = EpisodicFilter::new().for_namespace("agent-1").limit(10);
        assert_eq!(f2.namespace, Some("agent-1".to_string()));
        assert_eq!(f2.limit, Some(10));
    }

    #[test]
    fn test_memory_item_importance_clamped() {
        // Overshoot — clamped to 1.0
        let item_high = MemoryItem::new("ns", "x").with_importance(5.0);
        assert!((item_high.importance - 1.0).abs() < 1e-6);

        // Undershoot — clamped to 0.0
        let item_low = MemoryItem::new("ns", "x").with_importance(-3.0);
        assert!((item_low.importance - 0.0).abs() < 1e-6);

        // Normal value unchanged
        let item_mid = MemoryItem::new("ns", "x").with_importance(0.7);
        assert!((item_mid.importance - 0.7).abs() < 1e-6);
    }
}
