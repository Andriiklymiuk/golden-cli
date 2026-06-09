//! Response search: case-insensitive substring filter over rendered lines.

/// Whether `line` matches `query` (case-insensitive substring). An empty query
/// matches every line, so callers fall back to showing everything unfiltered.
pub fn matches(line: &str, query: &str) -> bool {
    query.is_empty() || line.to_lowercase().contains(&query.to_lowercase())
}

/// Keep only lines (with their original 0-based index) containing `query`
/// (case-insensitive). Empty query returns all lines unfiltered.
pub fn filter_lines(lines: &[String], query: &str) -> Vec<(usize, String)> {
    if query.is_empty() {
        return lines.iter().cloned().enumerate().collect();
    }
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| matches(l, query))
        .map(|(i, l)| (i, l.clone()))
        .collect()
}

/// Count of matching lines for the status hint.
pub fn match_count(lines: &[String], query: &str) -> usize {
    filter_lines(lines, query).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<String> {
        vec![
            "\"id\": 1".into(),
            "\"name\": \"Alice\"".into(),
            "\"role\": \"admin\"".into(),
        ]
    }

    #[test]
    fn empty_query_returns_all() {
        assert_eq!(filter_lines(&sample(), "").len(), 3);
    }

    #[test]
    fn filters_case_insensitively_and_keeps_indices() {
        let out = filter_lines(&sample(), "ALICE");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, 1);
        assert!(out[0].1.contains("Alice"));
        assert_eq!(match_count(&sample(), "i"), 3); // id, Alice, admin all contain 'i'
    }

    #[test]
    fn matches_is_case_insensitive_and_empty_query_matches_all() {
        assert!(matches("\"name\": \"Alice\"", "alice"));
        assert!(matches("\"name\": \"Alice\"", "ALICE"));
        assert!(!matches("\"name\": \"Alice\"", "bob"));
        assert!(matches("anything", "")); // empty query matches everything
    }
}
