//! Shared utilities for goal text processing.

/// Extract a filesystem path from natural-language goal text.
///
/// Tries preposition phrases first ("in /tmp", "from /var/log"), then
/// falls back to any whitespace-delimited token starting with `/`.
pub fn extract_path(text: &str) -> Option<String> {
    // Look for "in /path", "from /path", "to /path"
    for prefix in &["in ", "from ", "to ", "at "] {
        if let Some(idx) = text.find(prefix) {
            let after = &text[idx + prefix.len()..];
            if let Some(path) = extract_absolute_path(after) {
                return Some(path);
            }
        }
    }

    // Look for any absolute path in the text
    for word in text.split_whitespace() {
        if word.starts_with('/') {
            let cleaned = word.trim_end_matches([',', '.', ';']);
            return Some(cleaned.to_string());
        }
    }

    None
}

/// Extract an absolute path starting from the given text position.
fn extract_absolute_path(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    // Take until whitespace or end
    let path: String = trimmed
        .chars()
        .take_while(|c| !c.is_whitespace())
        .collect();
    let cleaned = path.trim_end_matches([',', '.', ';']);
    Some(cleaned.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_path_preposition_in() {
        assert_eq!(extract_path("list files in /tmp"), Some("/tmp".to_string()));
    }

    #[test]
    fn test_extract_path_preposition_from() {
        assert_eq!(
            extract_path("read logs from /var/log"),
            Some("/var/log".to_string())
        );
    }

    #[test]
    fn test_extract_path_preposition_to() {
        assert_eq!(
            extract_path("write data to /output/dir"),
            Some("/output/dir".to_string())
        );
    }

    #[test]
    fn test_extract_path_preposition_at() {
        assert_eq!(
            extract_path("look at /etc/hosts"),
            Some("/etc/hosts".to_string())
        );
    }

    #[test]
    fn test_extract_path_bare_absolute() {
        assert_eq!(
            extract_path("check /usr/local/bin"),
            Some("/usr/local/bin".to_string())
        );
    }

    #[test]
    fn test_extract_path_trailing_punctuation() {
        assert_eq!(
            extract_path("files in /tmp, please"),
            Some("/tmp".to_string())
        );
    }

    #[test]
    fn test_extract_path_none() {
        assert_eq!(extract_path("send an email"), None);
    }
}
