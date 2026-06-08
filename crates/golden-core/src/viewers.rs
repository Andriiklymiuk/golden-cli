//! Response viewers: Set-Cookie extraction + HTML preview temp-file writing.

use std::path::PathBuf;

/// Pull all Set-Cookie header values (case-insensitive name match), in order.
pub fn extract_set_cookies(headers: &[(String, String)]) -> Vec<String> {
    headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
        .map(|(_, v)| v.clone())
        .collect()
}

/// True if the response looks like HTML by content-type or body sniff.
pub fn is_html(content_type: &str, body: &[u8]) -> bool {
    if content_type.to_ascii_lowercase().contains("text/html") {
        return true;
    }
    let prefix: String = String::from_utf8_lossy(&body[..body.len().min(64)])
        .trim_start()
        .to_ascii_lowercase();
    prefix.starts_with("<!doctype html") || prefix.starts_with("<html")
}

/// Write HTML bytes to a persistent temp file ending in `.html`, return its path.
/// The file is NOT auto-deleted (the browser needs it after this returns).
pub fn write_html_temp(body: &[u8]) -> Result<PathBuf, String> {
    let mut builder = tempfile::Builder::new();
    builder.prefix("golden-preview-").suffix(".html");
    let file = builder.tempfile().map_err(|e| e.to_string())?;
    std::fs::write(file.path(), body).map_err(|e| e.to_string())?;
    // keep() returns the path and prevents deletion on drop
    let (_f, path) = file.keep().map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_all_set_cookie_headers_case_insensitively() {
        let headers = vec![
            ("Content-Type".to_string(), "text/html".to_string()),
            ("set-cookie".to_string(), "a=1; Path=/".to_string()),
            ("Set-Cookie".to_string(), "b=2; HttpOnly".to_string()),
        ];
        let cookies = extract_set_cookies(&headers);
        assert_eq!(
            cookies,
            vec!["a=1; Path=/".to_string(), "b=2; HttpOnly".to_string()]
        );
    }

    #[test]
    fn returns_empty_when_no_cookies() {
        let headers = vec![("Content-Type".to_string(), "text/html".to_string())];
        assert!(extract_set_cookies(&headers).is_empty());
    }

    #[test]
    fn detects_html_by_content_type_or_body() {
        assert!(is_html("text/html; charset=utf-8", b"<html></html>"));
        assert!(is_html("", b"<!DOCTYPE html><html></html>"));
        assert!(!is_html("application/json", b"{\"a\":1}"));
    }

    #[test]
    fn writes_html_to_temp_file_with_html_extension() {
        let path = write_html_temp(b"<html><body>hi</body></html>").unwrap();
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("html"));
        let content = std::fs::read(&path).unwrap();
        assert_eq!(content, b"<html><body>hi</body></html>");
        // path persists on disk for the browser to open
        assert!(path.exists());
        std::fs::remove_file(&path).ok();
    }
}
