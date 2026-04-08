//! Tokenizer supporting word-level and BPE modes, auto-detected from the JSON format.
//!
//! - **Word-level**: flat `{"token": idx, ...}` -- splits on whitespace, direct lookup.
//! - **BPE**: `{"vocab": {...}, "merges": ["X Y", ...]}` -- splits on whitespace, then
//!   applies Byte Pair Encoding merges for subword tokenization with character-level
//!   fallback for OOV words.

use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

#[allow(dead_code)]
pub const PAD_IDX: i64 = 0;
/// Unknown token index -- used for OOV words and unrecognized BPE subwords.
pub const UNK_IDX: i64 = 1;
/// Null prefix token prepended by `encode_with_null()` to offset span indices by 1.
pub const NULL_IDX: i64 = 2;

/// Bidirectional vocabulary mapping with optional BPE merge rules.
pub struct Tokenizer {
    word2idx: HashMap<String, i64>,
    idx2word: HashMap<i64, String>,
    /// BPE merge rules in priority order (lowest index = highest priority).
    /// Empty when operating in word-level mode.
    merges: Vec<(String, String)>,
}

impl Tokenizer {
    /// Load tokenizer from a JSON file.
    ///
    /// Auto-detects format:
    /// - If the JSON contains a `"merges"` key, BPE mode is activated.
    ///   Expected format: `{"vocab": {"<PAD>": 0, ...}, "merges": ["h e", "he l", ...]}`
    /// - Otherwise, word-level mode: `{"<PAD>": 0, "<UNK>": 1, ...}`
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)?;
        let root: Value = serde_json::from_str(&data)?;

        let mut word2idx = HashMap::new();
        let mut idx2word = HashMap::new();
        let mut merges = Vec::new();

        if let Some(merges_val) = root.get("merges") {
            let vocab_obj = root.get("vocab")
                .and_then(|v| v.as_object())
                .ok_or_else(|| anyhow::anyhow!("BPE tokenizer.json has 'merges' but missing 'vocab' object"))?;

            for (word, val) in vocab_obj {
                let idx = val.as_i64().unwrap_or(0);
                word2idx.insert(word.clone(), idx);
                idx2word.insert(idx, word.clone());
            }

            if let Some(merges_arr) = merges_val.as_array() {
                for entry in merges_arr {
                    if let Some(rule) = entry.as_str()
                        && let Some(space_pos) = rule.find(' ') {
                            let left = rule[..space_pos].to_string();
                            let right = rule[space_pos + 1..].to_string();
                            merges.push((left, right));
                        }
                }
            }
        } else {
            let map: HashMap<String, Value> = serde_json::from_value(root)?;
            for (word, val) in &map {
                let idx = val.as_i64().unwrap_or(0);
                word2idx.insert(word.clone(), idx);
                idx2word.insert(idx, word.clone());
            }
        }

        Ok(Self { word2idx, idx2word, merges })
    }

    pub const fn is_bpe(&self) -> bool {
        !self.merges.is_empty()
    }

    /// Apply BPE merges to a single word, returning subword tokens.
    ///
    /// Starts with individual characters, then repeatedly merges the highest-priority
    /// adjacent pair (lowest index in the merge table) until no merges apply.
    /// All occurrences of the winning pair are merged in a single pass (left-to-right).
    pub fn bpe_tokenize(&self, word: &str) -> Vec<String> {
        let mut symbols: Vec<String> = word.chars().map(|c| c.to_string()).collect();
        if symbols.len() <= 1 {
            return symbols;
        }

        loop {
            let mut best_merge_idx: Option<usize> = None;
            let mut best_pair_pos: Option<usize> = None;

            for i in 0..symbols.len() - 1 {
                if let Some(merge_idx) = self.find_merge(&symbols[i], &symbols[i + 1])
                    && (best_merge_idx.is_none() || merge_idx < best_merge_idx.unwrap()) {
                        best_merge_idx = Some(merge_idx);
                        best_pair_pos = Some(i);
                    }
            }

            match (best_merge_idx, best_pair_pos) {
                (Some(_), Some(_)) => {
                    let (ref left, ref right) = self.merges[best_merge_idx.unwrap()];
                    let merged = format!("{left}{right}");
                    let mut new_symbols = Vec::with_capacity(symbols.len());
                    let mut i = 0;
                    while i < symbols.len() {
                        if i < symbols.len() - 1 && symbols[i] == *left && symbols[i + 1] == *right {
                            new_symbols.push(merged.clone());
                            i += 2;
                        } else {
                            new_symbols.push(symbols[i].clone());
                            i += 1;
                        }
                    }
                    symbols = new_symbols;
                    if symbols.len() <= 1 {
                        break;
                    }
                }
                _ => break,
            }
        }

        symbols
    }

    fn find_merge(&self, left: &str, right: &str) -> Option<usize> {
        self.merges.iter().position(|(l, r)| l == left && r == right)
    }

    /// Tokenize text into subword or word-level tokens (lowercased).
    pub fn tokenize(&self, text: &str) -> Vec<String> {
        let lower = text.to_lowercase();
        if self.is_bpe() {
            lower.split_whitespace()
                .flat_map(|word| self.bpe_tokenize(word))
                .collect()
        } else {
            lower.split_whitespace()
                .map(std::string::ToString::to_string)
                .collect()
        }
    }

    /// Encode text to vocabulary indices. Unknown tokens map to `UNK_IDX`.
    pub fn encode(&self, text: &str) -> Vec<i64> {
        self.tokenize(text)
            .iter()
            .map(|t| *self.word2idx.get(t).unwrap_or(&UNK_IDX))
            .collect()
    }

    #[allow(dead_code)]
    pub fn encode_bpe(&self, text: &str) -> Vec<i64> {
        let lower = text.to_lowercase();
        lower.split_whitespace()
            .flat_map(|word| self.bpe_tokenize(word))
            .map(|t| *self.word2idx.get(&t).unwrap_or(&UNK_IDX))
            .collect()
    }

    /// Encode with a `NULL_IDX` prefix so span pointer index 0 means "no span"
    /// and actual tokens start at index 1.
    pub fn encode_with_null(&self, text: &str) -> Vec<i64> {
        let mut ids = vec![NULL_IDX];
        ids.extend(self.encode(text));
        ids
    }

    pub fn vocab_size(&self) -> usize {
        self.word2idx.len()
    }

    /// Map a vocabulary index back to its string. Returns `"<UNK>"` if not found.
    pub fn decode_index(&self, idx: usize) -> String {
        #[allow(clippy::cast_possible_wrap)] // vocab indices are small positive values
        self.idx2word.get(&(idx as i64)).cloned().unwrap_or_else(|| "<UNK>".to_string())
    }

    /// Extract text from token span indices. Indices are 1-based (0 = NULL prefix),
    /// so `start=1, end=2` extracts the first two tokens.
    #[allow(clippy::unused_self)] // method semantically belongs to Tokenizer
    pub fn extract_span(&self, tokens: &[String], start: usize, end: usize) -> String {
        if start == 0 && end == 0 {
            return String::new();
        }
        let s = if start > 0 { start - 1 } else { 0 };
        let e = if end > 0 { end } else { s + 1 };
        tokens[s..e.min(tokens.len())].join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn load_from_json(json: &str) -> Tokenizer {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokenizer.json");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(json.as_bytes()).unwrap();
        Tokenizer::load(&path).unwrap()
    }

    #[test]
    fn test_word_level_load() {
        let tok = load_from_json(r#"{"<PAD>": 0, "<UNK>": 1, "<NULL>": 2, "hello": 3, "world": 4}"#);
        assert!(!tok.is_bpe());
        assert_eq!(tok.vocab_size(), 5);
        assert_eq!(tok.encode("hello world"), vec![3, 4]);
        assert_eq!(tok.encode("unknown"), vec![UNK_IDX]);
    }

    #[test]
    fn test_bpe_load_and_detect() {
        let json = r#"{
            "vocab": {"<PAD>": 0, "<UNK>": 1, "<NULL>": 2, "h": 3, "e": 4, "l": 5, "o": 6, "he": 7, "hel": 8, "hell": 9, "hello": 10},
            "merges": ["h e", "he l", "hel l", "hell o"]
        }"#;
        let tok = load_from_json(json);
        assert!(tok.is_bpe());
        assert_eq!(tok.merges.len(), 4);
        assert_eq!(tok.merges[0], ("h".to_string(), "e".to_string()));
        assert_eq!(tok.merges[3], ("hell".to_string(), "o".to_string()));
    }

    #[test]
    fn test_bpe_tokenize_single_word() {
        let json = r#"{
            "vocab": {"<PAD>": 0, "<UNK>": 1, "h": 3, "e": 4, "l": 5, "o": 6, "he": 7, "hel": 8, "hell": 9, "hello": 10},
            "merges": ["h e", "he l", "hel l", "hell o"]
        }"#;
        let tok = load_from_json(json);
        // "hello" should be fully merged into a single token
        assert_eq!(tok.bpe_tokenize("hello"), vec!["hello"]);
    }

    #[test]
    fn test_bpe_tokenize_partial_merge() {
        // Only provide merges for "h e" -> "he", no further merges
        let json = r#"{
            "vocab": {"<PAD>": 0, "<UNK>": 1, "h": 3, "e": 4, "l": 5, "o": 6, "he": 7},
            "merges": ["h e"]
        }"#;
        let tok = load_from_json(json);
        // "hello" -> ['h','e','l','l','o'] -> ['he','l','l','o']
        assert_eq!(tok.bpe_tokenize("hello"), vec!["he", "l", "l", "o"]);
    }

    #[test]
    fn test_bpe_tokenize_no_applicable_merges() {
        let json = r#"{
            "vocab": {"<PAD>": 0, "<UNK>": 1, "x": 3, "y": 4},
            "merges": ["x y"]
        }"#;
        let tok = load_from_json(json);
        // "abc" has no applicable merges, stays as characters
        assert_eq!(tok.bpe_tokenize("abc"), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_bpe_encode() {
        let json = r#"{
            "vocab": {"<PAD>": 0, "<UNK>": 1, "h": 3, "e": 4, "l": 5, "o": 6, "he": 7, "hel": 8, "hell": 9, "hello": 10},
            "merges": ["h e", "he l", "hel l", "hell o"]
        }"#;
        let tok = load_from_json(json);
        assert_eq!(tok.encode("hello"), vec![10]);
        assert_eq!(tok.encode_bpe("hello"), vec![10]);
    }

    #[test]
    fn test_bpe_unknown_subwords_all_unk() {
        // "he" is produced by merge but not in vocab -> both subwords map to UNK
        let json = r#"{
            "vocab": {"<PAD>": 0, "<UNK>": 1, "h": 3, "e": 4},
            "merges": ["h e"]
        }"#;
        let tok = load_from_json(json);
        // "hex" -> ['h','e','x'] -> ['he','x']. Neither "he" nor "x" in vocab -> UNK, UNK
        assert_eq!(tok.encode("hex"), vec![UNK_IDX, UNK_IDX]);
    }

    #[test]
    fn test_bpe_unknown_subwords_partial() {
        let json = r#"{
            "vocab": {"<PAD>": 0, "<UNK>": 1, "h": 3, "e": 4, "he": 7},
            "merges": ["h e"]
        }"#;
        let tok = load_from_json(json);
        // "hex" -> ['h','e','x'] -> ['he','x']. 'he' in vocab (7), 'x' not -> UNK
        assert_eq!(tok.encode("hex"), vec![7, UNK_IDX]);
    }

    #[test]
    fn test_bpe_multiword_sentence() {
        let json = r#"{
            "vocab": {"<PAD>": 0, "<UNK>": 1, "h": 3, "e": 4, "l": 5, "o": 6, "w": 10, "r": 11, "d": 12,
                       "he": 13, "hel": 14, "hell": 15, "hello": 16,
                       "wo": 17, "wor": 18, "worl": 19, "world": 20},
            "merges": ["h e", "he l", "hel l", "hell o", "w o", "wo r", "wor l", "worl d"]
        }"#;
        let tok = load_from_json(json);
        assert_eq!(tok.tokenize("Hello World"), vec!["hello", "world"]);
        assert_eq!(tok.encode("Hello World"), vec![16, 20]);
    }

    #[test]
    fn test_bpe_single_char_word() {
        let json = r#"{
            "vocab": {"<PAD>": 0, "<UNK>": 1, "a": 3},
            "merges": []
        }"#;
        let tok = load_from_json(json);
        // Empty merges means word-level mode (is_bpe returns false)
        assert!(!tok.is_bpe());
    }

    #[test]
    fn test_bpe_merge_priority() {
        // Two possible pairs: "a b" (index 1) and "b c" (index 0).
        // "b c" has lower index, so it should be merged first.
        let json = r#"{
            "vocab": {"<PAD>": 0, "<UNK>": 1, "a": 3, "b": 4, "c": 5, "bc": 6, "ab": 7},
            "merges": ["b c", "a b"]
        }"#;
        let tok = load_from_json(json);
        // "abc" -> ['a','b','c'] -> best pair is "b c" (index 0) -> ['a', 'bc']
        // Next iteration: pair "a bc" not in merges -> done
        assert_eq!(tok.bpe_tokenize("abc"), vec!["a", "bc"]);
    }

    #[test]
    fn test_encode_with_null_bpe() {
        let json = r#"{
            "vocab": {"<PAD>": 0, "<UNK>": 1, "<NULL>": 2, "h": 3, "i": 4, "hi": 5},
            "merges": ["h i"]
        }"#;
        let tok = load_from_json(json);
        assert_eq!(tok.encode_with_null("hi"), vec![NULL_IDX, 5]);
    }

    #[test]
    fn test_decode_index_bpe() {
        let json = r#"{
            "vocab": {"<PAD>": 0, "<UNK>": 1, "hello": 10},
            "merges": ["h e"]
        }"#;
        let tok = load_from_json(json);
        assert_eq!(tok.decode_index(10), "hello");
        assert_eq!(tok.decode_index(999), "<UNK>");
    }

    #[test]
    fn test_extract_span_unchanged() {
        let json = r#"{"<PAD>": 0, "<UNK>": 1, "hello": 3, "world": 4}"#;
        let tok = load_from_json(json);
        let tokens = vec!["hello".to_string(), "world".to_string()];
        assert_eq!(tok.extract_span(&tokens, 1, 1), "hello");
        assert_eq!(tok.extract_span(&tokens, 1, 2), "hello world");
        assert_eq!(tok.extract_span(&tokens, 0, 0), "");
    }

    #[test]
    fn test_bpe_repeated_pairs() {
        // Word "aab" with merge "a a" -> "aa"
        let json = r#"{
            "vocab": {"<PAD>": 0, "<UNK>": 1, "a": 3, "b": 4, "aa": 5},
            "merges": ["a a"]
        }"#;
        let tok = load_from_json(json);
        // "aab" -> ['a','a','b'] -> merge "a a" -> ['aa','b']
        assert_eq!(tok.bpe_tokenize("aab"), vec!["aa", "b"]);
    }

    #[test]
    fn test_bpe_empty_string() {
        let json = r#"{
            "vocab": {"<PAD>": 0, "<UNK>": 1},
            "merges": ["a b"]
        }"#;
        let tok = load_from_json(json);
        assert_eq!(tok.tokenize(""), Vec::<String>::new());
        assert_eq!(tok.encode(""), Vec::<i64>::new());
    }
}
