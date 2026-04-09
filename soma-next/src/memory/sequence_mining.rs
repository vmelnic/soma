use std::collections::HashMap;

/// A frequent subsequence discovered by PrefixSpan, with its absolute and relative support.
#[derive(Debug, Clone)]
pub struct FrequentSequence {
    pub pattern: Vec<String>,
    pub support: usize,
    pub support_ratio: f64,
}

/// Mine frequent subsequences from a collection of string sequences using the PrefixSpan algorithm.
///
/// `min_support` is a ratio in [0.0, 1.0] representing the fraction of sequences that must
/// contain a pattern for it to be considered frequent.
///
/// Returns all frequent subsequences sorted by pattern (alphabetically, then by length).
pub fn prefix_span(sequences: &[Vec<String>], min_support: f64) -> Vec<FrequentSequence> {
    if sequences.is_empty() {
        return Vec::new();
    }

    let min_count = (min_support * sequences.len() as f64).ceil() as usize;
    if min_count == 0 {
        return Vec::new();
    }

    let total = sequences.len();
    let mut results = Vec::new();

    // Count length-1 item frequencies (deduplicated per sequence)
    let mut item_counts: HashMap<String, usize> = HashMap::new();
    for seq in sequences {
        let mut seen = std::collections::HashSet::new();
        for item in seq {
            if seen.insert(item.clone()) {
                *item_counts.entry(item.clone()).or_insert(0) += 1;
            }
        }
    }

    // Collect frequent items, sorted alphabetically for determinism
    let mut frequent_items: Vec<(String, usize)> = item_counts
        .into_iter()
        .filter(|(_, count)| *count >= min_count)
        .collect();
    frequent_items.sort_by(|a, b| a.0.cmp(&b.0));

    let max_pattern_len: usize = 20;
    let max_results: usize = 1000;

    for (item, support) in &frequent_items {
        if results.len() >= max_results {
            break;
        }

        let prefix = vec![item.clone()];
        results.push(FrequentSequence {
            pattern: prefix.clone(),
            support: *support,
            support_ratio: *support as f64 / total as f64,
        });

        // Build projected database: suffix after first occurrence of item in each sequence
        let projected_db = build_projected_db(sequences, item);

        mine_recursive(
            &prefix,
            &projected_db,
            min_count,
            total,
            &mut results,
            max_pattern_len,
            max_results,
        );
    }

    results
}

/// Return the longest frequent subsequence. Ties are broken by highest support.
pub fn longest_frequent_subsequence(
    sequences: &[Vec<String>],
    min_support: f64,
) -> Option<FrequentSequence> {
    let results = prefix_span(sequences, min_support);
    results.into_iter().max_by(|a, b| {
        a.pattern
            .len()
            .cmp(&b.pattern.len())
            .then_with(|| a.support.cmp(&b.support))
    })
}

/// Build the projected database for a given item: for each sequence containing the item,
/// take the suffix strictly after the first occurrence.
fn build_projected_db(sequences: &[Vec<String>], item: &str) -> Vec<Vec<String>> {
    let mut projected = Vec::new();
    for seq in sequences {
        if let Some(pos) = seq.iter().position(|s| s == item) {
            let suffix = seq[pos + 1..].to_vec();
            if !suffix.is_empty() {
                projected.push(suffix);
            }
        }
    }
    projected
}

/// Recursively extend the prefix by mining the projected database.
fn mine_recursive(
    prefix: &[String],
    projected_db: &[Vec<String>],
    min_count: usize,
    total: usize,
    results: &mut Vec<FrequentSequence>,
    max_pattern_len: usize,
    max_results: usize,
) {
    if prefix.len() >= max_pattern_len || results.len() >= max_results || projected_db.is_empty() {
        return;
    }

    // Count item frequencies in the projected database (deduplicated per sequence)
    let mut item_counts: HashMap<String, usize> = HashMap::new();
    for seq in projected_db {
        let mut seen = std::collections::HashSet::new();
        for item in seq {
            if seen.insert(item.clone()) {
                *item_counts.entry(item.clone()).or_insert(0) += 1;
            }
        }
    }

    // Frequent items sorted alphabetically
    let mut frequent_items: Vec<(String, usize)> = item_counts
        .into_iter()
        .filter(|(_, count)| *count >= min_count)
        .collect();
    frequent_items.sort_by(|a, b| a.0.cmp(&b.0));

    for (item, support) in &frequent_items {
        if results.len() >= max_results {
            return;
        }

        let mut new_prefix = prefix.to_vec();
        new_prefix.push(item.clone());

        results.push(FrequentSequence {
            pattern: new_prefix.clone(),
            support: *support,
            support_ratio: *support as f64 / total as f64,
        });

        // Project further
        let new_projected = build_projected_db(projected_db, item);

        mine_recursive(
            &new_prefix,
            &new_projected,
            min_count,
            total,
            results,
            max_pattern_len,
            max_results,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn pattern_strs(fs: &FrequentSequence) -> Vec<&str> {
        fs.pattern.iter().map(|s| s.as_str()).collect()
    }

    #[test]
    fn basic_frequent_patterns() {
        let sequences = vec![s(&["a", "b", "c"]), s(&["a", "b"]), s(&["a", "c", "b"])];
        let results = prefix_span(&sequences, 0.66);

        let patterns: Vec<Vec<&str>> = results.iter().map(|r| pattern_strs(r)).collect();

        // "a" should be frequent (appears in all 3 sequences)
        assert!(patterns.contains(&vec!["a"]));
        // "a","b" should be frequent (appears in all 3: a->b, a->b, a->c->b)
        assert!(patterns.contains(&vec!["a", "b"]));
    }

    #[test]
    fn all_identical() {
        let sequences = vec![
            s(&["x", "y", "z"]),
            s(&["x", "y", "z"]),
            s(&["x", "y", "z"]),
        ];
        let results = prefix_span(&sequences, 1.0);

        let patterns: Vec<Vec<&str>> = results.iter().map(|r| pattern_strs(r)).collect();

        // The full sequence should be frequent
        assert!(patterns.contains(&vec!["x", "y", "z"]));
        // All sub-prefixes too
        assert!(patterns.contains(&vec!["x"]));
        assert!(patterns.contains(&vec!["x", "y"]));

        // Check that the full pattern has support 3 and ratio 1.0
        let full = results.iter().find(|r| r.pattern.len() == 3).unwrap();
        assert_eq!(full.support, 3);
        assert!((full.support_ratio - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn empty_input() {
        let results = prefix_span(&[], 0.5);
        assert!(results.is_empty());
    }

    #[test]
    fn single_element_sequences() {
        let sequences = vec![s(&["a"]), s(&["a"]), s(&["b"])];
        let results = prefix_span(&sequences, 0.66);

        let patterns: Vec<Vec<&str>> = results.iter().map(|r| pattern_strs(r)).collect();

        // "a" appears in 2/3 >= 0.66, so it should be frequent
        assert!(patterns.contains(&vec!["a"]));
        // "b" appears in 1/3 < 0.66, so it should NOT be frequent
        assert!(!patterns.contains(&vec!["b"]));
        // No multi-element patterns possible from single-element sequences
        assert!(results.iter().all(|r| r.pattern.len() == 1));
    }

    #[test]
    fn min_support_1_0() {
        let sequences = vec![
            s(&["a", "b", "c"]),
            s(&["a", "b"]),
            s(&["a", "c", "b"]),
        ];
        let results = prefix_span(&sequences, 1.0);

        // Only patterns present in ALL 3 sequences should appear
        for r in &results {
            assert_eq!(r.support, 3, "pattern {:?} has support {}", r.pattern, r.support);
        }

        let patterns: Vec<Vec<&str>> = results.iter().map(|r| pattern_strs(r)).collect();
        assert!(patterns.contains(&vec!["a"]));
        assert!(patterns.contains(&vec!["a", "b"]));
    }

    #[test]
    fn support_counts_correct() {
        let sequences = vec![s(&["a", "b", "c"]), s(&["a", "b"]), s(&["a", "c", "b"])];
        let results = prefix_span(&sequences, 0.5);

        for r in &results {
            // support_ratio must equal support / total
            let expected_ratio = r.support as f64 / sequences.len() as f64;
            assert!(
                (r.support_ratio - expected_ratio).abs() < f64::EPSILON,
                "pattern {:?}: ratio {} != expected {}",
                r.pattern,
                r.support_ratio,
                expected_ratio
            );
        }

        // "a" appears in all 3
        let a = results.iter().find(|r| r.pattern == s(&["a"])).unwrap();
        assert_eq!(a.support, 3);

        // "b" appears in all 3
        let b = results.iter().find(|r| r.pattern == s(&["b"])).unwrap();
        assert_eq!(b.support, 3);
    }

    #[test]
    fn longest_subsequence() {
        let sequences = vec![
            s(&["a", "b", "c"]),
            s(&["a", "b", "c"]),
            s(&["a", "b", "c"]),
        ];
        let longest = longest_frequent_subsequence(&sequences, 1.0);
        assert!(longest.is_some());
        let longest = longest.unwrap();
        assert_eq!(pattern_strs(&longest), vec!["a", "b", "c"]);
        assert_eq!(longest.support, 3);
    }

    #[test]
    fn max_pattern_length_cap() {
        // Create sequences of length 30 with repeating elements across all sequences
        let long_seq: Vec<String> = (0..30).map(|i| format!("item{:02}", i)).collect();
        let sequences = vec![long_seq.clone(), long_seq.clone(), long_seq.clone()];
        let results = prefix_span(&sequences, 1.0);

        // No pattern should exceed length 20
        for r in &results {
            assert!(
                r.pattern.len() <= 20,
                "pattern length {} exceeds cap of 20",
                r.pattern.len()
            );
        }
        // There should be patterns of various lengths
        assert!(!results.is_empty());
    }

    #[test]
    fn no_frequent_items() {
        // All unique items, each appears in only 1 of 4 sequences
        let sequences = vec![s(&["a"]), s(&["b"]), s(&["c"]), s(&["d"])];
        let results = prefix_span(&sequences, 0.5);
        assert!(results.is_empty());
    }

    #[test]
    fn performance_test() {
        // 50 sequences of length 8 with items drawn from a small vocabulary
        let vocab = ["alpha", "beta", "gamma", "delta", "epsilon"];
        let sequences: Vec<Vec<String>> = (0..50)
            .map(|i| {
                (0..8)
                    .map(|j| vocab[(i * 3 + j * 7) % vocab.len()].to_string())
                    .collect()
            })
            .collect();

        let results = prefix_span(&sequences, 0.3);
        // Should complete without hanging and produce some results
        assert!(
            !results.is_empty(),
            "expected results from 50 sequences with small vocabulary"
        );
    }
}
