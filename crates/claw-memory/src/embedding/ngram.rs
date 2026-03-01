use crate::traits::Embedder;

/// Character n-gram embedding (bigrams + trigrams).
///
/// Produces a 64-dimensional float vector by accumulating bigram and trigram
/// character frequency counts hashed into 64 buckets, then L2-normalizing.
/// Deterministic, no external API, no allocations beyond the output vector.
pub struct NgramEmbedder {
    dims: usize,
}

impl NgramEmbedder {
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

// ============================================================
// Tests
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;

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
        // Should be within floating-point tolerance of 1.0
        assert!((norm - 1.0).abs() < 1e-5, "norm = {norm}");
    }

    #[test]
    fn test_ngram_embed_same_text_returns_same_vector() {
        let e = NgramEmbedder::new();
        let v1 = e.embed("deterministic test");
        let v2 = e.embed("deterministic test");
        assert_eq!(v1, v2, "embedding must be deterministic");
    }

    #[test]
    fn test_ngram_embed_different_texts_different_vectors() {
        let e = NgramEmbedder::new();
        let v1 = e.embed("apple");
        let v2 = e.embed("banana");
        assert_ne!(v1, v2, "different texts must produce different vectors");
    }

    #[test]
    fn test_ngram_embed_empty_string() {
        let e = NgramEmbedder::new();
        let v = e.embed("");
        // Must not panic; all values should be 0.0 (zero input → zero vector)
        assert_eq!(v.len(), 64);
        assert!(
            v.iter().all(|&x| x == 0.0),
            "empty string should produce zero vector"
        );
    }
}
