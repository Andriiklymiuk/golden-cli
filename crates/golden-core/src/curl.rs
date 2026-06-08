//! Generate a `curl` command line from a Request. Mirrors the VSCode extension's
//! generateCurlFromHistoryEntry output exactly (flags, layout, quoting).

use std::collections::HashMap;

use crate::model::Request;
use crate::subst::substitute;

/// Header names whose VALUES are always credentials. Compared case-insensitively.
const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "x-auth-token",
    "x-access-token",
    "api-key",
    "apikey",
];

/// True if this header name should have its value masked to `***`.
pub fn is_sensitive_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SENSITIVE_HEADERS.contains(&lower.as_str())
}

/// Mask a value if it LOOKS like a credential (Bearer/Basic scheme prefix).
/// Returns `***` when masked, otherwise the value unchanged.
pub fn mask_value(value: &str) -> String {
    let lower = value.trim_start().to_ascii_lowercase();
    if lower.starts_with("bearer ") || lower.starts_with("basic ") {
        "***".to_string()
    } else {
        value.to_string()
    }
}

/// Build a `curl` command from a request. `{{var}}` are substituted using `vars`.
/// Layout matches the extension: `curl -X M`, then ` \` + "\n  " before each
/// -H/-d/flag/URL token; URL is emitted last.
pub fn generate(req: &Request, vars: &HashMap<String, String>, opts: &CurlOptions) -> String {
    let method = req.method.to_uppercase();
    let mut out = format!("curl -X {method}");

    for h in &req.header {
        if h.disabled || h.key.is_empty() {
            continue;
        }
        let key = substitute(&h.key, vars);
        let mut value = substitute(&h.value, vars);
        if opts.mask && (is_sensitive_header(&key) || mask_value(&value) == "***") {
            value = "***".to_string();
        }
        out.push_str(&format!(" \\\n  -H '{key}: {value}'"));
    }

    if let Some(body) = &req.body {
        if let Some(raw) = &body.raw {
            let text = match raw {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let payload = substitute(&text, vars);
            if !payload.is_empty() && payload != "{}" {
                out.push_str(&format!(" \\\n  -d '{payload}'"));
            }
        }
    }

    if opts.follow_redirects {
        out.push_str(" \\\n  -L");
    }
    if opts.include_headers {
        out.push_str(" \\\n  -i");
    }
    if opts.silent {
        out.push_str(" \\\n  -s");
    }
    if opts.insecure {
        out.push_str(" \\\n  -k");
    }
    if opts.fail {
        out.push_str(" \\\n  -f");
    }
    if opts.compressed {
        out.push_str(" \\\n  --compressed");
    }
    if opts.timing {
        out.push_str(" \\\n  -w \"Time: %{time_total}s\"");
    }
    if opts.file_download {
        out.push_str(" \\\n  -O -J");
    }

    let url = substitute(req.url.raw(), vars);
    out.push_str(&format!(" \\\n  '{url}'"));
    out
}

/// cURL flag toggles. Maps 1:1 to the extension's curlOptions object.
#[derive(Debug, Clone, Default)]
pub struct CurlOptions {
    pub follow_redirects: bool, // -L
    pub include_headers: bool,  // -i
    pub silent: bool,           // -s
    pub insecure: bool,         // -k
    pub fail: bool,             // -f
    pub compressed: bool,       // --compressed
    pub timing: bool,           // -w "Time: %{time_total}s"
    pub file_download: bool,    // -O -J
    pub mask: bool,             // mask sensitive header values
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Body, Header, Request, Url};
    use std::collections::HashMap;

    fn req() -> Request {
        Request {
            method: "POST".into(),
            url: Url::Raw("https://api.test/login".into()),
            header: vec![Header {
                key: "Content-Type".into(),
                value: "application/json".into(),
                disabled: false,
                extra: serde_json::Map::new(),
            }],
            body: Some(Body {
                mode: "raw".into(),
                raw: Some(serde_json::Value::String(r#"{"u":"x"}"#.into())),
                graphql: None,
                formdata: vec![],
            }),
        }
    }

    #[test]
    fn default_options_are_all_false() {
        let o = CurlOptions::default();
        assert!(!o.follow_redirects);
        assert!(!o.file_download);
        assert!(!o.mask);
    }

    #[test]
    fn masks_sensitive_header_names_case_insensitively() {
        assert!(is_sensitive_header("Authorization"));
        assert!(is_sensitive_header("authorization"));
        assert!(is_sensitive_header("Cookie"));
        assert!(is_sensitive_header("X-API-Key"));
        assert!(is_sensitive_header("x-api-key"));
        assert!(is_sensitive_header("Proxy-Authorization"));
        assert!(is_sensitive_header("Set-Cookie"));
        assert!(!is_sensitive_header("Content-Type"));
        assert!(!is_sensitive_header("Accept"));
    }

    #[test]
    fn masks_credential_value_patterns() {
        // Bearer tokens / api-key-like values are masked even on a neutral header name.
        assert_eq!(mask_value("Bearer abc.def.ghi"), "***");
        assert_eq!(mask_value("Basic dXNlcjpwYXNz"), "***");
        assert_eq!(mask_value("application/json"), "application/json");
    }

    #[test]
    fn generates_curl_with_method_headers_body_and_url_last() {
        let out = generate(&req(), &HashMap::new(), &CurlOptions::default());
        assert!(out.starts_with("curl -X POST"), "got: {out}");
        assert!(
            out.contains(" \\\n  -H 'Content-Type: application/json'"),
            "got: {out}"
        );
        assert!(out.contains(" \\\n  -d '{\"u\":\"x\"}'"), "got: {out}");
        // URL is last
        assert!(
            out.trim_end().ends_with(" \\\n  'https://api.test/login'"),
            "got: {out}"
        );
    }

    #[test]
    fn appends_flags_in_extension_order() {
        let o = CurlOptions {
            follow_redirects: true,
            include_headers: true,
            silent: true,
            insecure: true,
            fail: true,
            compressed: true,
            timing: true,
            file_download: true,
            mask: false,
        };
        let out = generate(&req(), &HashMap::new(), &o);
        let l = out.find(" \\\n  -L").unwrap();
        let i = out.find(" \\\n  -i").unwrap();
        let s = out.find(" \\\n  -s").unwrap();
        let k = out.find(" \\\n  -k").unwrap();
        let f = out.find(" \\\n  -f").unwrap();
        let c = out.find(" \\\n  --compressed").unwrap();
        let w = out.find(" \\\n  -w \"Time: %{time_total}s\"").unwrap();
        let dl = out.find(" \\\n  -O -J").unwrap();
        assert!(
            l < i && i < s && s < k && k < f && f < c && c < w && w < dl,
            "order wrong: {out}"
        );
    }

    #[test]
    fn generate_masks_sensitive_headers_when_enabled() {
        let mut r = req();
        r.header.push(Header {
            key: "Authorization".into(),
            value: "Bearer secret123".into(),
            disabled: false,
            extra: serde_json::Map::new(),
        });
        r.header.push(Header {
            key: "X-API-Key".into(),
            value: "abc".into(),
            disabled: false,
            extra: serde_json::Map::new(),
        });
        let masked = generate(
            &r,
            &HashMap::new(),
            &CurlOptions {
                mask: true,
                ..Default::default()
            },
        );
        assert!(masked.contains("-H 'Authorization: ***'"), "got: {masked}");
        assert!(masked.contains("-H 'X-API-Key: ***'"), "got: {masked}");
        // non-sensitive untouched
        assert!(
            masked.contains("-H 'Content-Type: application/json'"),
            "got: {masked}"
        );

        let plain = generate(&r, &HashMap::new(), &CurlOptions::default());
        assert!(
            plain.contains("-H 'Authorization: Bearer secret123'"),
            "got: {plain}"
        );
    }

    #[test]
    fn substitutes_variables_before_emitting() {
        let mut r = req();
        r.url = Url::Raw("{{base}}/login".into());
        let vars = HashMap::from([("base".to_string(), "https://api.test".to_string())]);
        let out = generate(&r, &vars, &CurlOptions::default());
        assert!(out.contains("'https://api.test/login'"), "got: {out}");
    }
}
