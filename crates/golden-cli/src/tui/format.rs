//! Pure formatting helpers for the response pane: JSON pretty-print, header list,
//! Set-Cookie extraction, status color.

use ratatui::style::Color;

/// Pretty-print a response body. If it parses as JSON, indent it; if it is valid
/// UTF-8 text, return it as-is; otherwise return a binary note with the byte count.
pub fn pretty_body(body: &[u8], content_type: Option<&str>) -> String {
    if let Ok(text) = std::str::from_utf8(body) {
        let looks_json = content_type.map(|c| c.contains("json")).unwrap_or(false)
            || text.trim_start().starts_with('{')
            || text.trim_start().starts_with('[');
        if looks_json {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
                return serde_json::to_string_pretty(&value).unwrap_or_else(|_| text.to_string());
            }
        }
        return text.to_string();
    }
    format!("<binary: {} bytes>", body.len())
}

/// Color a status code: 2xx green, 3xx cyan, 4xx yellow, 5xx red, else gray.
pub fn status_color(status: u16) -> Color {
    match status {
        200..=299 => Color::Green,
        300..=399 => Color::Cyan,
        400..=499 => Color::Yellow,
        500..=599 => Color::Red,
        _ => Color::Gray,
    }
}

/// Extract Set-Cookie header values (case-insensitive key match).
pub fn cookies(headers: &[(String, String)]) -> Vec<String> {
    headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
        .map(|(_, v)| v.clone())
        .collect()
}

/// Look up a header value (case-insensitive), first match wins.
pub fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_prints_json_body() {
        let out = pretty_body(br#"{"b":2,"a":1}"#, Some("application/json"));
        assert!(out.contains('\n'));
        assert!(out.contains("\"b\": 2"));
    }

    #[test]
    fn passes_through_plain_text() {
        let out = pretty_body(b"hello world", Some("text/plain"));
        assert_eq!(out, "hello world");
    }

    #[test]
    fn notes_binary_when_not_utf8() {
        let out = pretty_body(&[0xff, 0xfe, 0x00], None);
        assert_eq!(out, "<binary: 3 bytes>");
    }

    #[test]
    fn status_colors_by_class() {
        assert_eq!(status_color(204), Color::Green);
        assert_eq!(status_color(301), Color::Cyan);
        assert_eq!(status_color(404), Color::Yellow);
        assert_eq!(status_color(500), Color::Red);
    }

    #[test]
    fn extracts_set_cookies_case_insensitively() {
        let h = vec![
            ("Content-Type".into(), "text/html".into()),
            ("set-cookie".into(), "a=1".into()),
            ("Set-Cookie".into(), "b=2".into()),
        ];
        let c = cookies(&h);
        assert_eq!(c, vec!["a=1".to_string(), "b=2".to_string()]);
        assert_eq!(header_value(&h, "content-type"), Some("text/html"));
    }
}
