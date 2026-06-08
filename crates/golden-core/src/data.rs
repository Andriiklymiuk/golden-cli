//! Data-driven runs: parse a JSON-array or CSV data file into one row per
//! iteration. Each row feeds `pm.iterationData` and overlays the variable set
//! (data wins over env/collection) for that iteration.

use std::collections::HashMap;

/// Parse a data file's text into rows. A leading `[` selects JSON (array of
/// objects); anything else is treated as CSV (header row + data rows).
pub fn parse_data(text: &str) -> Result<Vec<HashMap<String, String>>, String> {
    if text.trim_start().starts_with('[') {
        parse_json(text)
    } else {
        parse_csv(text)
    }
}

fn parse_json(text: &str) -> Result<Vec<HashMap<String, String>>, String> {
    let rows: Vec<serde_json::Map<String, serde_json::Value>> =
        serde_json::from_str(text).map_err(|e| format!("invalid JSON data file: {e}"))?;
    Ok(rows
        .into_iter()
        .map(|obj| {
            obj.into_iter()
                .map(|(k, v)| {
                    let s = match v {
                        serde_json::Value::String(s) => s,
                        serde_json::Value::Null => String::new(),
                        other => other.to_string(),
                    };
                    (k, s)
                })
                .collect()
        })
        .collect())
}

fn parse_csv(text: &str) -> Result<Vec<HashMap<String, String>>, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = match lines.next() {
        Some(h) => split_csv_line(h),
        None => return Ok(Vec::new()),
    };
    let mut rows = Vec::new();
    for line in lines {
        let fields = split_csv_line(line);
        let mut row = HashMap::new();
        for (i, key) in header.iter().enumerate() {
            row.insert(key.clone(), fields.get(i).cloned().unwrap_or_default());
        }
        rows.push(row);
    }
    Ok(rows)
}

/// Split one CSV line on commas, honoring double-quoted fields (with `""`
/// escapes and embedded commas). Minimal RFC-4180-ish, single line.
fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                fields.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    fields.push(cur.trim().to_string());
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_array_of_objects() {
        let rows = parse_data(r#"[{"userId":"1","name":"Ann"},{"userId":2,"name":"Bo"}]"#).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["userId"], "1");
        assert_eq!(rows[0]["name"], "Ann");
        assert_eq!(rows[1]["userId"], "2"); // number coerced to string
    }

    #[test]
    fn parses_csv_header_and_rows() {
        let rows = parse_data("userId,name\n1,Ann\n2,Bo\n").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["userId"], "1");
        assert_eq!(rows[1]["name"], "Bo");
    }

    #[test]
    fn csv_honors_quoted_commas() {
        let rows = parse_data("id,label\n1,\"a,b\"\n").unwrap();
        assert_eq!(rows[0]["label"], "a,b");
    }

    #[test]
    fn invalid_json_array_errors() {
        assert!(parse_data("[not json").is_err());
    }
}
