//! Ngram-based local embedding provider.
//!
//! Wraps [`claw_memory::embedding::NgramEmbedder`] to implement the
//! [`EmbeddingProvider`] trait without any external API calls.

use async_trait::async_trait;
use claw_memory::{embedding::NgramEmbedder, traits::Embedder};

use crate::{error::ProviderError, traits::EmbeddingProvider, types::Embedding};

/// An [`EmbeddingProvider`] backed by the deterministic character n-gram
/// embedder from `claw-memory`.
///
/// Produces 64-dimensional float vectors without any network access.
pub struct NgramEmbeddingProvider {
    embedder: NgramEmbedder,
}

impl NgramEmbeddingProvider {
    /// Create a new provider with the default n-gram embedder.
    pub fn new() -> Self {
        Self {
            embedder: NgramEmbedder::new(),
        }
    }
}

impl Default for NgramEmbeddingProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EmbeddingProvider for NgramEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Embedding, ProviderError> {
        Ok(self.embedder.embed(text))
    }

    async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Embedding>, ProviderError> {
        Ok(texts.iter().map(|t| self.embedder.embed(t)).collect())
    }

    fn dimensions(&self) -> usize {
        self.embedder.dimensions()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ngram_embedding_provider_new() {
        let provider = NgramEmbeddingProvider::new();
        assert_eq!(provider.dimensions(), 64);
    }

    #[tokio::test]
    async fn test_ngram_embedding_provider_embed() {
        let provider = NgramEmbeddingProvider::new();
        let embedding = provider.embed("hello").await.expect("embed should succeed");
        assert_eq!(embedding.len(), 64, "expected 64-dimensional embedding");
    }

    #[tokio::test]
    async fn test_ngram_embedding_provider_embed_batch() {
        let provider = NgramEmbeddingProvider::new();
        let texts = vec!["a".to_string(), "b".to_string()];
        let embeddings = provider
            .embed_batch(texts)
            .await
            .expect("embed_batch should succeed");
        assert_eq!(embeddings.len(), 2, "expected 2 embeddings");
        assert_eq!(embeddings[0].len(), 64);
        assert_eq!(embeddings[1].len(), 64);
    }

    #[tokio::test]
    async fn test_ngram_embedding_provider_dimensions() {
        let provider = NgramEmbeddingProvider::new();
        assert_eq!(provider.dimensions(), 64);
    }
}
