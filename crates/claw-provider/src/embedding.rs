//! Local embedding utilities for claw-provider.
//!
//! Provides the [`Embedder`] trait and the [`NgramEmbedder`] implementation,
//! a deterministic character n-gram embedder that requires no external API.
//! Also provides [`NgramEmbeddingProvider`], which wraps [`NgramEmbedder`] to
//! implement the async [`EmbeddingProvider`] trait.

use async_trait::async_trait;

use crate::{error::ProviderError, traits::EmbeddingProvider, types::Embedding};

// ─── Embedder trait ──────────────────────────────────────────────────────────

/// Synchronous, local text embedding.
///
/// Implementations must be deterministic, `Send + Sync`, and free of external
/// network calls. The output length must always equal [`Embedder::dimensions`].
pub trait Embedder: Send + Sync {
    /// Embed a single text into a fixed-length float vector.
    fn embed(&self, text: &str) -> Vec<f32>;

    /// Embed multiple texts. Default implementation calls [`embed`] sequentially.
    fn embed_batch(&self, texts: &[&str]) -> Vec<Vec<f32>> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    /// Number of dimensions in the output vector.
    fn dimensions(&self) -> usize;
}

// ─── NgramEmbedder ───────────────────────────────────────────────────────────

/// Character n-gram embedding (bigrams + trigrams).
///
/// Produces a 64-dimensional float vector by accumulating bigram and trigram
/// character frequency counts hashed into 64 buckets, then L2-normalizing.
/// Deterministic, no external API, no allocations beyond the output vector.
pub struct NgramEmbedder {
    dims: usize,
}

impl NgramEmbedder {
    /// Create a new embedder with 64 dimensions.
    pub fn new() -> Self {
        Self { dims: 64 }
    }
}

impl Default for NgramEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

impl Embedder for NgramEmbedder {
    fn embed(&self, text: &str) -> Vec<f32> {
        let mut vec = vec![0.0f32; self.dims];
        let lower = text.to_lowercase();
        let chars: Vec<char> = lower.chars().collect();

        // Bigrams
        for i in 0..chars.len().saturating_sub(1) {
            let bigram = [chars[i] as u32, chars[i + 1] as u32];
            let hash = hash_ngram(&bigram);
            vec[(hash as usize) % self.dims] += 1.0;
        }

        // Trigrams
        for i in 0..chars.len().saturating_sub(2) {
            let trigram = [chars[i] as u32, chars[i + 1] as u32, chars[i + 2] as u32];
            let hash = hash_ngram(&trigram);
            vec[(hash as usize) % self.dims] += 1.0;
        }

        // L2 normalize
        l2_normalize(&mut vec);
        vec
    }

    fn dimensions(&self) -> usize {
        self.dims
    }
}

/// FNV-1a–style hash for a slice of u32 code points.
fn hash_ngram(chars: &[u32]) -> u64 {
    let mut hash: u64 = 14695981039346656037;
    for &c in chars {
        hash ^= c as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    hash
}

/// In-place L2 normalisation. No-op when the norm is zero.
fn l2_normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

// ─── NgramEmbeddingProvider ──────────────────────────────────────────────────

/// An [`EmbeddingProvider`] backed by the deterministic character n-gram
/// embedder. Produces 64-dimensional float vectors without any network access.
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

    // NgramEmbedder tests
    #[test]
    fn test_ngram_embedder_dimensions() {
        let e = NgramEmbedder::new();
        assert_eq!(e.dimensions(), 64);
        let v = e.embed("hello");
        assert_eq!(v.len(), 64);
    }

    #[test]
    fn test_ngram_embed_returns_normalized_vector() {
        let e = NgramEmbedder::new();
        let v = e.embed("the quick brown fox");
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm = {norm}");
    }

    #[test]
    fn test_ngram_embed_same_text_returns_same_vector() {
        let e = NgramEmbedder::new();
        let v1 = e.embed("deterministic test");
        let v2 = e.embed("deterministic test");
        assert_eq!(v1, v2);
    }

    #[test]
    fn test_ngram_embed_different_texts_different_vectors() {
        let e = NgramEmbedder::new();
        let v1 = e.embed("apple");
        let v2 = e.embed("banana");
        assert_ne!(v1, v2);
    }

    #[test]
    fn test_ngram_embed_empty_string() {
        let e = NgramEmbedder::new();
        let v = e.embed("");
        assert_eq!(v.len(), 64);
        assert!(v.iter().all(|&x| x == 0.0), "empty string should produce zero vector");
    }

    // NgramEmbeddingProvider tests
    #[tokio::test]
    async fn test_ngram_embedding_provider_new() {
        let provider = NgramEmbeddingProvider::new();
        assert_eq!(provider.dimensions(), 64);
    }

    #[tokio::test]
    async fn test_ngram_embedding_provider_embed() {
        let provider = NgramEmbeddingProvider::new();
        let embedding = provider.embed("hello").await.expect("embed should succeed");
        assert_eq!(embedding.len(), 64);
    }

    #[tokio::test]
    async fn test_ngram_embedding_provider_embed_batch() {
        let provider = NgramEmbeddingProvider::new();
        let texts = vec!["a".to_string(), "b".to_string()];
        let embeddings = provider.embed_batch(texts).await.expect("embed_batch should succeed");
        assert_eq!(embeddings.len(), 2);
        assert_eq!(embeddings[0].len(), 64);
        assert_eq!(embeddings[1].len(), 64);
    }

    #[tokio::test]
    async fn test_ngram_embedding_provider_dimensions() {
        let provider = NgramEmbeddingProvider::new();
        assert_eq!(provider.dimensions(), 64);
    }
}
