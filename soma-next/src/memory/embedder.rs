/// GoalEmbedder — trait for embedding text into fixed-dimensional vectors
/// and computing similarity between them. Used by the memory system to match
/// incoming intents against stored episodes, routines, and world knowledge.
///
/// Compute cosine similarity between two vectors.
/// Returns 0.0 if either vector has zero magnitude.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    assert_eq!(a.len(), b.len(), "vectors must have equal length");
    let dot: f64 = a.iter().zip(b.iter()).map(|(&x, &y)| x as f64 * y as f64).sum();
    let mag_a: f64 = a.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>().sqrt();
    let mag_b: f64 = b.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    dot / (mag_a * mag_b)
}

/// Trait for embedding text into fixed-dimensional float vectors.
pub trait GoalEmbedder: Send + Sync {
    /// Embed text into a fixed-dimensional vector.
    fn embed(&self, text: &str) -> Vec<f32>;

    /// Compute similarity between two embeddings (cosine similarity for unit vectors).
    fn similarity(&self, a: &[f32], b: &[f32]) -> f64;
}

/// Hash-based embedder using FNV-1a for deterministic, dependency-free text embedding.
/// Produces sparse, L2-normalized vectors via feature hashing (the "hashing trick").
pub struct HashEmbedder {
    pub dimensions: usize,
}

impl HashEmbedder {
    /// Create a HashEmbedder with the default 128 dimensions.
    pub fn new() -> Self {
        Self { dimensions: 128 }
    }

    /// Create a HashEmbedder with a custom number of dimensions.
    pub fn with_dimensions(dimensions: usize) -> Self {
        Self { dimensions }
    }
}

impl Default for HashEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

/// FNV-1a hash with a seedable offset basis.
/// Seed is XORed into the standard offset basis before hashing begins.
fn fnv1a(data: &[u8], seed: u64) -> u64 {
    let offset_basis: u64 = 14695981039346656037;
    let prime: u64 = 1099511628211;
    let mut hash = offset_basis ^ seed;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(prime);
    }
    hash
}

/// Tokenize text: lowercase, split on non-alphanumeric boundaries, drop tokens shorter than 2 chars.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_string())
        .collect()
}

impl GoalEmbedder for HashEmbedder {
    fn embed(&self, text: &str) -> Vec<f32> {
        let tokens = tokenize(text);
        let mut vec = vec![0.0f32; self.dimensions];

        for token in &tokens {
            let bytes = token.as_bytes();

            // Dimension index from hash with seed 0
            let h0 = fnv1a(bytes, 0);
            let idx = (h0 % self.dimensions as u64) as usize;

            // Sign from hash with seed 1
            let h1 = fnv1a(bytes, 1);
            let sign: f32 = if h1 & 1 == 0 { 1.0 } else { -1.0 };

            vec[idx] += sign;
        }

        // L2-normalize to unit vector
        let magnitude: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if magnitude > 0.0 {
            for x in &mut vec {
                *x /= magnitude;
            }
        }

        vec
    }

    fn similarity(&self, a: &[f32], b: &[f32]) -> f64 {
        // For unit vectors, dot product == cosine similarity.
        a.iter()
            .zip(b.iter())
            .map(|(&x, &y)| x as f64 * y as f64)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_strings_same_embedding() {
        let e = HashEmbedder::new();
        let a = e.embed("list files in directory");
        let b = e.embed("list files in directory");
        assert_eq!(a, b);
    }

    #[test]
    fn similar_strings_high_similarity() {
        let e = HashEmbedder::new();
        let a = e.embed("list files in directory");
        let b = e.embed("list directory files");
        let sim = e.similarity(&a, &b);
        assert!(
            sim > 0.5,
            "expected similarity > 0.5 for similar strings, got {sim}"
        );
    }

    #[test]
    fn dissimilar_strings_low_similarity() {
        let e = HashEmbedder::new();
        let a = e.embed("list files");
        let b = e.embed("send http post request to api");
        let sim = e.similarity(&a, &b);
        assert!(
            sim < 0.5,
            "expected similarity < 0.5 for dissimilar strings, got {sim}"
        );
    }

    #[test]
    fn empty_string_zero_vector() {
        let e = HashEmbedder::new();
        let v = e.embed("");
        assert!(v.iter().all(|&x| x == 0.0));

        let other = e.embed("hello world");
        let sim = e.similarity(&v, &other);
        assert_eq!(sim, 0.0);
    }

    #[test]
    fn unit_vector() {
        let e = HashEmbedder::new();
        let v = e.embed("some non-empty text here");
        let magnitude: f64 = v.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>().sqrt();
        assert!(
            (magnitude - 1.0).abs() < 1e-6,
            "expected unit vector (magnitude ~1.0), got {magnitude}"
        );
    }

    #[test]
    fn cosine_similarity_identical() {
        let a = vec![1.0f32, 0.0, 0.0];
        let b = vec![1.0f32, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-9, "expected 1.0, got {sim}");
    }

    #[test]
    fn cosine_similarity_orthogonal() {
        let a = vec![1.0f32, 0.0, 0.0];
        let b = vec![0.0f32, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-9, "expected 0.0, got {sim}");
    }

    #[test]
    fn with_dimensions_works() {
        let e = HashEmbedder::with_dimensions(64);
        let v = e.embed("test input");
        assert_eq!(v.len(), 64);
    }
}
