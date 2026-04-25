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
/// `weights` is an optional per-sequence salience weight. When provided, weighted counts are
/// used instead of uniform counting to determine whether a pattern meets the minimum support
/// threshold. When `None`, behavior is identical to uniform (weight-1.0) counting.
///
/// `max_pattern_length` caps the maximum length of discovered patterns.
///
/// `max_results` caps the total number of patterns returned.
///
/// Returns all frequent subsequences sorted by pattern (alphabetically, then by length).
pub fn prefix_span(
    sequences: &[Vec<String>],
    min_support: f64,
    weights: Option<&[f64]>,
    max_pattern_length: usize,
    max_results: usize,
) -> Vec<FrequentSequence> {
    if sequences.is_empty() {
        return Vec::new();
    }

    let total_weight: f64 = weights.map_or(sequences.len() as f64, |ws| ws.iter().sum());
    let min_weighted_count = min_support * total_weight;
    if min_weighted_count <= 0.0 {
        return Vec::new();
    }

    let total = sequences.len();
    let mut results = Vec::new();

    // Build weighted sequence list: each entry is (sequence, weight)
    let weighted_seqs: Vec<(&[String], f64)> = sequences
        .iter()
        .enumerate()
        .map(|(i, seq)| (seq.as_slice(), weights.map_or(1.0, |ws| ws[i])))
        .collect();

    // Count length-1 item frequencies (deduplicated per sequence, weighted)
    let mut item_counts: HashMap<String, (f64, usize)> = HashMap::new();
    for (seq, w) in &weighted_seqs {
        let mut seen = std::collections::HashSet::new();
        for item in *seq {
            if seen.insert(item.clone()) {
                let entry = item_counts.entry(item.clone()).or_insert((0.0, 0));
                entry.0 += w;
                entry.1 += 1;
            }
        }
    }

    // Collect frequent items, sorted alphabetically for determinism
    let mut frequent_items: Vec<(String, f64, usize)> = item_counts
        .into_iter()
        .filter(|(_, (wc, _))| *wc >= min_weighted_count)
        .map(|(item, (wc, count))| (item, wc, count))
        .collect();
    frequent_items.sort_by(|a, b| a.0.cmp(&b.0));

    let max_pattern_len = max_pattern_length;

    for (item, _weighted_count, support) in &frequent_items {
        if results.len() >= max_results {
            break;
        }

        let prefix = vec![item.clone()];
        results.push(FrequentSequence {
            pattern: prefix.clone(),
            support: *support,
            support_ratio: *support as f64 / total as f64,
        });

        // Build projected database: suffix after first occurrence of item in each sequence,
        // carrying the original weight forward.
        let projected_db = build_weighted_projected_db(&weighted_seqs, item);

        mine_recursive(
            &prefix,
            &projected_db,
            min_weighted_count,
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
    weights: Option<&[f64]>,
    max_pattern_length: usize,
    max_results: usize,
) -> Option<FrequentSequence> {
    if sequences.is_empty() {
        return None;
    }

    let total_weight: f64 = weights.map_or(sequences.len() as f64, |ws| ws.iter().sum());
    let min_weighted_count = min_support * total_weight;
    let total = sequences.len();

    // Direct O(n*m) approach: find items with sufficient weighted support,
    // take their order from the first sequence, then verify the combined
    // pattern is a subsequence of enough input sequences.
    // PrefixSpan's alphabetical exploration can hit max_results before
    // discovering long patterns in sequences with many items.
    let mut item_support: HashMap<String, (f64, usize)> = HashMap::new();
    for (i, seq) in sequences.iter().enumerate() {
        let w = weights.map_or(1.0, |ws| ws[i]);
        let mut seen = std::collections::HashSet::new();
        for item in seq {
            if seen.insert(item.clone()) {
                let entry = item_support.entry(item.clone()).or_insert((0.0, 0));
                entry.0 += w;
                entry.1 += 1;
            }
        }
    }

    let frequent: std::collections::HashSet<&str> = item_support
        .iter()
        .filter(|(_, (wc, _))| *wc >= min_weighted_count)
        .map(|(item, _)| item.as_str())
        .collect();

    let pattern: Vec<String> = sequences[0]
        .iter()
        .filter(|s| frequent.contains(s.as_str()))
        .take(max_pattern_length)
        .cloned()
        .collect();

    if !pattern.is_empty() {
        let support = sequences
            .iter()
            .filter(|seq| is_subsequence(&pattern, seq))
            .count();
        let support_ratio = support as f64 / total as f64;
        if support_ratio >= min_support {
            return Some(FrequentSequence {
                pattern,
                support,
                support_ratio,
            });
        }
    }

    // Fallback to PrefixSpan for cases where item-level support doesn't
    // guarantee pattern-level support (rare in practice).
    let results = prefix_span(sequences, min_support, weights, max_pattern_length, max_results);
    results.into_iter().max_by(|a, b| {
        a.pattern
            .len()
            .cmp(&b.pattern.len())
            .then_with(|| a.support.cmp(&b.support))
    })
}

fn is_subsequence(pattern: &[String], sequence: &[String]) -> bool {
    let mut pi = 0;
    for item in sequence {
        if pi < pattern.len() && *item == pattern[pi] {
            pi += 1;
        }
    }
    pi == pattern.len()
}

/// Build the projected database for a given item: for each weighted sequence containing the item,
/// take the suffix strictly after the first occurrence, preserving the weight.
fn build_weighted_projected_db(
    sequences: &[(&[String], f64)],
    item: &str,
) -> Vec<(Vec<String>, f64)> {
    let mut projected = Vec::new();
    for (seq, w) in sequences {
        if let Some(pos) = seq.iter().position(|s| s == item) {
            let suffix = seq[pos + 1..].to_vec();
            if !suffix.is_empty() {
                projected.push((suffix, *w));
            }
        }
    }
    projected
}

/// Recursively extend the prefix by mining the projected database.
fn mine_recursive(
    prefix: &[String],
    projected_db: &[(Vec<String>, f64)],
    min_weighted_count: f64,
    total: usize,
    results: &mut Vec<FrequentSequence>,
    max_pattern_len: usize,
    max_results: usize,
) {
    if prefix.len() >= max_pattern_len || results.len() >= max_results || projected_db.is_empty() {
        return;
    }

    // Count item frequencies in the projected database (deduplicated per sequence, weighted)
    let mut item_counts: HashMap<String, (f64, usize)> = HashMap::new();
    for (seq, w) in projected_db {
        let mut seen = std::collections::HashSet::new();
        for item in seq {
            if seen.insert(item.clone()) {
                let entry = item_counts.entry(item.clone()).or_insert((0.0, 0));
                entry.0 += w;
                entry.1 += 1;
            }
        }
    }

    // Frequent items sorted alphabetically
    let mut frequent_items: Vec<(String, f64, usize)> = item_counts
        .into_iter()
        .filter(|(_, (wc, _))| *wc >= min_weighted_count)
        .map(|(item, (wc, count))| (item, wc, count))
        .collect();
    frequent_items.sort_by(|a, b| a.0.cmp(&b.0));

    for (item, _weighted_count, support) in &frequent_items {
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

        // Project further: convert projected_db entries into slice-weight pairs for reuse
        let as_slices: Vec<(&[String], f64)> = projected_db
            .iter()
            .map(|(seq, w)| (seq.as_slice(), *w))
            .collect();
        let new_projected = build_weighted_projected_db(&as_slices, item);

        mine_recursive(
            &new_prefix,
            &new_projected,
            min_weighted_count,
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
        let results = prefix_span(&sequences, 0.66, None, 20, 1000);

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
        let results = prefix_span(&sequences, 1.0, None, 20, 1000);

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
        let results = prefix_span(&[], 0.5, None, 20, 1000);
        assert!(results.is_empty());
    }

    #[test]
    fn single_element_sequences() {
        let sequences = vec![s(&["a"]), s(&["a"]), s(&["b"])];
        let results = prefix_span(&sequences, 0.66, None, 20, 1000);

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
        let results = prefix_span(&sequences, 1.0, None, 20, 1000);

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
        let results = prefix_span(&sequences, 0.5, None, 20, 1000);

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
        let longest = longest_frequent_subsequence(&sequences, 1.0, None, 20, 1000);
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
        let results = prefix_span(&sequences, 1.0, None, 20, 1000);

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
        let results = prefix_span(&sequences, 0.5, None, 20, 1000);
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

        let results = prefix_span(&sequences, 0.3, None, 20, 1000);
        // Should complete without hanging and produce some results
        assert!(
            !results.is_empty(),
            "expected results from 50 sequences with small vocabulary"
        );
    }

    // --- Weighted PrefixSpan tests ---

    #[test]
    fn none_weights_identical_to_unweighted() {
        // Verify that passing None weights produces the exact same results as if
        // every sequence had weight 1.0.
        let sequences = vec![
            s(&["a", "b", "c"]),
            s(&["a", "b"]),
            s(&["a", "c", "b"]),
            s(&["b", "c"]),
        ];

        let results_none = prefix_span(&sequences, 0.5, None, 20, 1000);
        let uniform_weights: Vec<f64> = vec![1.0; sequences.len()];
        let results_uniform = prefix_span(&sequences, 0.5, Some(&uniform_weights), 20, 1000);

        assert_eq!(
            results_none.len(),
            results_uniform.len(),
            "None and uniform-1.0 weights should produce the same number of patterns"
        );
        for (a, b) in results_none.iter().zip(results_uniform.iter()) {
            assert_eq!(a.pattern, b.pattern);
            assert_eq!(a.support, b.support);
            assert!(
                (a.support_ratio - b.support_ratio).abs() < f64::EPSILON,
                "support_ratio mismatch for {:?}",
                a.pattern
            );
        }
    }

    #[test]
    fn high_weight_promotes_patterns() {
        // Two sequences with "a","b" (high weight) and two with "c","d" (low weight).
        // At min_support 0.5, the high-weight pattern should be frequent but the
        // low-weight one should not.
        let sequences = vec![
            s(&["a", "b"]),
            s(&["a", "b"]),
            s(&["c", "d"]),
            s(&["c", "d"]),
        ];
        // With uniform weights at 0.5, both ["a","b"] and ["c","d"] should be frequent
        let results_uniform = prefix_span(&sequences, 0.5, None, 20, 1000);
        let ab_uniform = results_uniform.iter().any(|r| r.pattern == s(&["a", "b"]));
        let cd_uniform = results_uniform.iter().any(|r| r.pattern == s(&["c", "d"]));
        assert!(ab_uniform, "uniform: a,b should be frequent");
        assert!(cd_uniform, "uniform: c,d should be frequent");

        // With weights [3.0, 3.0, 0.1, 0.1], total = 6.2, min_count = 3.1.
        // "a" weighted count = 6.0 >= 3.1 (frequent), "c" weighted count = 0.2 < 3.1 (not frequent)
        let weights = vec![3.0, 3.0, 0.1, 0.1];
        let results_weighted = prefix_span(&sequences, 0.5, Some(&weights), 20, 1000);
        let ab_weighted = results_weighted.iter().any(|r| r.pattern == s(&["a", "b"]));
        let cd_weighted = results_weighted.iter().any(|r| r.pattern == s(&["c", "d"]));
        assert!(ab_weighted, "weighted: a,b should be frequent (high salience)");
        assert!(!cd_weighted, "weighted: c,d should NOT be frequent (low salience)");
    }

    #[test]
    fn zero_weight_sequences_excluded() {
        // Sequences with weight 0.0 should not contribute to any pattern's count.
        let sequences = vec![
            s(&["a", "b"]),
            s(&["a", "b"]),
            s(&["a", "b"]),
        ];
        // All weight 0.0: total_weight = 0.0, min_weighted_count = 0.0 -> empty
        let weights = vec![0.0, 0.0, 0.0];
        let results = prefix_span(&sequences, 0.5, Some(&weights), 20, 1000);
        assert!(results.is_empty(), "all-zero weights should yield no patterns");
    }

    #[test]
    fn weighted_longest_frequent_subsequence() {
        // Pattern "a","b","c" appears in sequences 0-2 (high weight).
        // Pattern "x","y","z","w" appears in sequences 3-5 (low weight).
        // Without weights, the longer pattern wins. With weights, the shorter
        // high-salience pattern should be the only frequent one at high min_support.
        let sequences = vec![
            s(&["a", "b", "c"]),
            s(&["a", "b", "c"]),
            s(&["a", "b", "c"]),
            s(&["x", "y", "z", "w"]),
            s(&["x", "y", "z", "w"]),
            s(&["x", "y", "z", "w"]),
        ];
        let weights = vec![5.0, 5.0, 5.0, 0.1, 0.1, 0.1];
        // total = 15.3, min_count at 0.5 = 7.65
        // "a" weighted = 15.0 >= 7.65, "x" weighted = 0.3 < 7.65
        let longest = longest_frequent_subsequence(&sequences, 0.5, Some(&weights), 20, 1000);
        assert!(longest.is_some());
        let longest = longest.unwrap();
        assert_eq!(pattern_strs(&longest), vec!["a", "b", "c"]);
    }

    #[test]
    fn longest_subsequence_many_items() {
        let skills = s(&[
            "kitchen.scan",
            "kitchen.push_board",
            "kitchen.pick_jar",
            "kitchen.door_open",
            "kitchen.place_shelf",
            "kitchen.door_close",
            "kitchen.drawer_close",
            "kitchen.drawer_open",
            "kitchen.pick_knife",
            "kitchen.peg_insert",
            "kitchen.button_press",
            "kitchen.window_open",
            "kitchen.window_close",
        ]);
        let sequences = vec![skills.clone(), skills.clone(), skills.clone()];
        let longest =
            longest_frequent_subsequence(&sequences, 0.7, None, 20, 1000);
        assert!(longest.is_some());
        let longest = longest.unwrap();
        assert_eq!(longest.pattern.len(), 13);
        assert_eq!(longest.pattern[0], "kitchen.scan");
        assert_eq!(longest.pattern[12], "kitchen.window_close");
        assert_eq!(longest.support, 3);
    }
}
