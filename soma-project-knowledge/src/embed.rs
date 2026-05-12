const DIMENSIONS: usize = 128;

pub fn hash_embed(text: &str) -> Vec<f32> {
    let mut vec = vec![0.0f32; DIMENSIONS];
    let lower = text.to_lowercase();
    for token in lower.split(|c: char| !c.is_alphanumeric()).filter(|t| t.len() >= 2) {
        let mut h: u64 = 0xcbf29ce484222325; // FNV-1a offset basis
        for b in token.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        let idx = (h as usize) % DIMENSIONS;
        let sign = if (h >> 32) & 1 == 0 { 1.0 } else { -1.0 };
        vec[idx] += sign;
    }

    let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut vec {
            *v /= norm;
        }
    }
    vec
}
