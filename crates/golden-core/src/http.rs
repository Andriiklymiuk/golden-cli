//! HTTP execution via reqwest (blocking). Substitutes variables, maps errors.

use std::collections::HashMap;
use std::error::Error;
use std::io::Read;
use std::path::Path;
use std::time::Instant;

use crate::model::{Body, Request};
use crate::subst::substitute;

#[derive(Debug, Clone, Default)]
pub struct HttpConfig {
    /// Disable TLS verification for ALL hosts (CLI --insecure).
    pub insecure: bool,
    /// Per-request timeout in milliseconds.
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub time_ms: u128,
}

/// Send a request, substituting {{var}} in URL, headers and body.
/// Returns Err(message) on transport failure (ECONNREFUSED mapped to a friendly string).
pub fn send(
    req: &Request,
    vars: &HashMap<String, String>,
    cfg: &HttpConfig,
) -> Result<HttpResponse, String> {
    let url = substitute(req.url.raw(), vars);
    let insecure = cfg.insecure || is_localhost(&url);

    let mut builder = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .danger_accept_invalid_certs(insecure);
    if let Some(ms) = cfg.timeout_ms {
        builder = builder.timeout(std::time::Duration::from_millis(ms));
    }
    let client = builder.build().map_err(|e| e.to_string())?;

    let method = reqwest::Method::from_bytes(req.method.to_uppercase().as_bytes())
        .map_err(|e| e.to_string())?;
    let mut rb = client.request(method, &url);

    for h in &req.header {
        if h.disabled {
            continue;
        }
        rb = rb.header(substitute(&h.key, vars), substitute(&h.value, vars));
    }

    if let Some(b) = &req.body {
        if b.mode == "formdata" {
            let plan = crate::multipart::plan_form(b, vars)?;
            let form = crate::multipart::build_reqwest_form(&plan)?;
            rb = rb.multipart(form);
        } else if let Some(payload) = build_body(b, vars) {
            rb = rb.body(payload);
        }
    }

    let start = Instant::now();
    let resp = rb.send().map_err(map_err)?;
    let status = resp.status().as_u16();
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body = resp.bytes().map_err(map_err)?.to_vec();
    Ok(HttpResponse {
        status,
        headers,
        body,
        time_ms: start.elapsed().as_millis(),
    })
}

/// Build the request body, substituting variables.
/// `raw` is a serde_json::Value: a JSON string (typical) is taken as-is; an object/array
/// (some collections store the parsed body) is serialized to JSON text first. In both
/// cases `substitute` then does the single-pass walk + re-stringify (discrepancy #3).
fn build_body(b: &Body, vars: &HashMap<String, String>) -> Option<String> {
    match b.mode.as_str() {
        "graphql" => {
            let g = b.graphql.as_ref()?;
            let query = substitute(&g.query, vars);
            let variables = match &g.variables {
                Some(v) => {
                    let raw = substitute(&v.to_string(), vars);
                    serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null)
                }
                None => serde_json::Value::Null,
            };
            Some(serde_json::json!({ "query": query, "variables": variables }).to_string())
        }
        _ => b.raw.as_ref().map(|r| {
            let text = match r {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            substitute(&text, vars)
        }),
    }
}

/// Result of a file download.
#[derive(Debug, Clone)]
pub struct DownloadInfo {
    pub status: u16,
    pub bytes_written: u64,
    pub suggested_filename: Option<String>,
}

/// Stream a response body to `target`. Enforces `size_cap` (bytes) if set — on
/// overflow the partial file is removed and an Err is returned. Returns the
/// server-suggested filename (from Content-Disposition) for the caller to honor.
/// The existing `send` function is NOT changed; this is a separate path.
pub fn download_to_file(
    req: &Request,
    vars: &HashMap<String, String>,
    cfg: &HttpConfig,
    target: &Path,
    size_cap: Option<u64>,
) -> Result<DownloadInfo, String> {
    let url = substitute(req.url.raw(), vars);
    let insecure = cfg.insecure || is_localhost(&url);
    let mut builder = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .danger_accept_invalid_certs(insecure);
    if let Some(ms) = cfg.timeout_ms {
        builder = builder.timeout(std::time::Duration::from_millis(ms));
    }
    let client = builder.build().map_err(|e| e.to_string())?;
    let method = reqwest::Method::from_bytes(req.method.to_uppercase().as_bytes())
        .map_err(|e| e.to_string())?;
    let mut rb = client.request(method, &url);
    for h in &req.header {
        if h.disabled {
            continue;
        }
        rb = rb.header(substitute(&h.key, vars), substitute(&h.value, vars));
    }
    let mut resp = rb.send().map_err(map_err)?;
    let status = resp.status().as_u16();
    let suggested = resp
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(crate::download::filename_from_disposition);

    let mut file = std::fs::File::create(target).map_err(|e| e.to_string())?;
    let mut buf = [0u8; 8192];
    let mut total: u64 = 0;
    loop {
        let n = resp.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if let Some(cap) = size_cap {
            if total > cap {
                drop(file);
                let _ = std::fs::remove_file(target);
                return Err(format!("download exceeds size cap of {cap} bytes"));
            }
        }
        use std::io::Write;
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
    }
    Ok(DownloadInfo {
        status,
        bytes_written: total,
        suggested_filename: suggested,
    })
}

fn is_localhost(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.starts_with("https://localhost") || u.starts_with("https://127.0.0.1")
}

fn map_err(e: reqwest::Error) -> String {
    // Build a full error string including all sources in the chain.
    let mut full = e.to_string();
    let mut src: Option<&dyn std::error::Error> = e.source();
    while let Some(s) = src {
        full.push_str(": ");
        full.push_str(&s.to_string());
        src = s.source();
    }

    if full.contains("Connection refused")
        || full.contains("connection refused")
        || full.contains("os error 61")
        || full.contains("tcp connect error")
    {
        return "Connection refused: Server not running or unreachable".to_string();
    }
    full
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Header, Request, Url};
    use httpmock::prelude::*;
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn req(method: &str, url: &str) -> Request {
        Request {
            method: method.into(),
            url: Url::Raw(url.into()),
            header: vec![],
            body: None,
        }
    }

    #[test]
    fn sends_get_and_captures_status_and_body() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET).path("/ping");
            then.status(200)
                .header("content-type", "application/json")
                .body("{\"ok\":true}");
        });
        let r = req("GET", &format!("{}/ping", server.base_url()));
        let resp = send(&r, &HashMap::new(), &HttpConfig::default()).unwrap();
        m.assert();
        assert_eq!(resp.status, 200);
        assert_eq!(String::from_utf8_lossy(&resp.body), "{\"ok\":true}");
    }

    #[test]
    fn substitutes_variables_in_url_and_headers() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET).path("/u/42").header("x-token", "secret");
            then.status(204);
        });
        let mut r = req("GET", &format!("{}/u/{{{{id}}}}", server.base_url()));
        r.header.push(Header {
            key: "X-Token".into(),
            value: "{{tok}}".into(),
            disabled: false,
            extra: serde_json::Map::new(),
        });
        let vars = HashMap::from([
            ("id".to_string(), "42".to_string()),
            ("tok".to_string(), "secret".to_string()),
        ]);
        let resp = send(&r, &vars, &HttpConfig::default()).unwrap();
        m.assert();
        assert_eq!(resp.status, 204);
    }

    #[test]
    fn maps_connection_refused() {
        // port 1 is reserved/unused; connection refused expected
        let r = req("GET", "http://127.0.0.1:1/nope");
        let err = send(&r, &HashMap::new(), &HttpConfig::default()).unwrap_err();
        assert!(err.contains("Connection refused"), "got: {err}");
    }

    #[test]
    fn downloads_body_to_file() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/file");
            then.status(200)
                .header("content-disposition", "attachment; filename=\"out.bin\"")
                .body("BINARYDATA");
        });
        let dir = tempdir().unwrap();
        let target = dir.path().join("out.bin");
        let r = req("GET", &format!("{}/file", server.base_url()));
        let info =
            download_to_file(&r, &HashMap::new(), &HttpConfig::default(), &target, None).unwrap();
        assert_eq!(info.status, 200);
        assert_eq!(info.bytes_written, 10);
        assert_eq!(std::fs::read(&target).unwrap(), b"BINARYDATA");
        assert_eq!(info.suggested_filename.as_deref(), Some("out.bin"));
    }

    #[test]
    fn download_respects_size_cap() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/big");
            then.status(200).body("0123456789");
        });
        let dir = tempdir().unwrap();
        let target = dir.path().join("big.bin");
        let r = req("GET", &format!("{}/big", server.base_url()));
        let err = download_to_file(
            &r,
            &HashMap::new(),
            &HttpConfig::default(),
            &target,
            Some(5),
        )
        .unwrap_err();
        assert!(err.contains("size cap"), "got: {err}");
        assert!(!target.exists(), "partial file should not remain");
    }

    #[test]
    fn sends_multipart_formdata_upload() {
        use crate::model::{Body, FormField};
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(POST)
                .path("/upload")
                .header_exists("content-type"); // multipart boundary present
            then.status(201);
        });
        let dir = tempdir().unwrap();
        let fpath = dir.path().join("u.txt");
        std::fs::write(&fpath, b"file-bytes").unwrap();

        let mut r = req("POST", &format!("{}/upload", server.base_url()));
        r.body = Some(Body {
            mode: "formdata".into(),
            raw: None,
            graphql: None,
            formdata: vec![
                FormField {
                    key: "name".into(),
                    kind: "text".into(),
                    value: "n".into(),
                    src: None,
                    disabled: false,
                },
                FormField {
                    key: "file".into(),
                    kind: "file".into(),
                    value: String::new(),
                    src: Some(fpath.to_str().unwrap().into()),
                    disabled: false,
                },
            ],
        });
        let resp = send(&r, &HashMap::new(), &HttpConfig::default()).unwrap();
        m.assert();
        assert_eq!(resp.status, 201);
    }
}
