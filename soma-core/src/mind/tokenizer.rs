//! SOMA Tokenizer — word-level vocabulary lookup.

use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

pub const PAD_IDX: i64 = 0;
pub const UNK_IDX: i64 = 1;
pub const NULL_IDX: i64 = 2;

pub struct Tokenizer {
    word2idx: HashMap<String, i64>,
    idx2word: HashMap<i64, String>,
}

impl Tokenizer {
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)?;
        let map: HashMap<String, Value> = serde_json::from_str(&data)?;

        let mut word2idx = HashMap::new();
        let mut idx2word = HashMap::new();
        for (word, val) in &map {
            let idx = val.as_i64().unwrap_or(0);
            word2idx.insert(word.clone(), idx);
            idx2word.insert(idx, word.clone());
        }

        Ok(Self { word2idx, idx2word })
    }

    pub fn tokenize(&self, text: &str) -> Vec<String> {
        text.to_lowercase()
            .split_whitespace()
            .map(|s| s.to_string())
            .collect()
    }

    pub fn encode(&self, text: &str) -> Vec<i64> {
        self.tokenize(text)
            .iter()
            .map(|t| *self.word2idx.get(t).unwrap_or(&UNK_IDX))
            .collect()
    }

    /// Encode with NULL prefix (for decoder span extraction)
    pub fn encode_with_null(&self, text: &str) -> Vec<i64> {
        let mut ids = vec![NULL_IDX];
        ids.extend(self.encode(text));
        ids
    }

    pub fn vocab_size(&self) -> usize {
        self.word2idx.len()
    }

    /// Extract text from token span (offset by 1 for NULL prefix)
    pub fn extract_span(&self, tokens: &[String], start: usize, end: usize) -> String {
        if start == 0 && end == 0 {
            return String::new();
        }
        let s = if start > 0 { start - 1 } else { 0 };
        let e = if end > 0 { end } else { s + 1 };
        tokens[s..e.min(tokens.len())].join(" ")
    }
}
