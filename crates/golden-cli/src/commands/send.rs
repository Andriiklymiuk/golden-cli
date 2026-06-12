//! `golden send <collection> <request>`: locate one request across discovered
//! collections, send it via golden_core, and print the response.

use std::io::{self, Write};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use golden_core::download::{resolve_target, OverwritePolicy};
use golden_core::env::{parse_env, resolve, VarScopes};
use golden_core::http::{download_to_file, send as core_send, HttpConfig};
use golden_core::model::{Collection, Item, Request};
use golden_core::subst::substitute;
use golden_core::viewers::{extract_set_cookies, is_html, write_html_temp};
use serde::Serialize;

use crate::cli::{SendArgs, SendReporterKind};
use crate::discovery::{discover, env_paths};
use crate::exit::FATAL;
use crate::load::{load, Loaded};

/// One request located in a collection: the index path into the item tree
/// (consumable by `golden_core::runner::run_single`), the item name, and the
/// request itself.
#[derive(Debug)]
pub struct RequestMatch<'a> {
    pub path: Vec<usize>,
    pub name: String,
    pub request: &'a Request,
}

/// Find every request matching `name`, in depth-first order. A plain name
/// matches request items anywhere in the tree. When nothing matches and the
/// name contains '/', it is retried as folder-qualified
/// ("Folder/Sub/Request"): the last segment is the request name and the
/// preceding segments must equal its ancestor folder names from the root.
/// Plain matches win, so request names that themselves contain '/' keep working.
pub fn find_requests<'a>(collection: &'a Collection, name: &str) -> Vec<RequestMatch<'a>> {
    fn walk<'a>(
        items: &'a [Item],
        prefix: &mut Vec<usize>,
        folders: &mut Vec<String>,
        matches: &dyn Fn(&Item, &[String]) -> bool,
        out: &mut Vec<RequestMatch<'a>>,
    ) {
        for (idx, item) in items.iter().enumerate() {
            prefix.push(idx);
            if let Some(request) = &item.request {
                if matches(item, folders) {
                    out.push(RequestMatch {
                        path: prefix.clone(),
                        name: item.name.clone(),
                        request,
                    });
                }
            }
            if let Some(children) = &item.item {
                folders.push(item.name.clone());
                walk(children, prefix, folders, matches, out);
                folders.pop();
            }
            prefix.pop();
        }
    }

    let mut out = Vec::new();
    walk(
        &collection.item,
        &mut Vec::new(),
        &mut Vec::new(),
        &|item, _| item.name == name,
        &mut out,
    );
    if out.is_empty() && name.contains('/') {
        let segments: Vec<&str> = name.split('/').collect();
        let (leaf, dirs) = segments.split_last().expect("split on non-empty name");
        walk(
            &collection.item,
            &mut Vec::new(),
            &mut Vec::new(),
            &|item, folders| {
                item.name == *leaf
                    && folders.len() == dirs.len()
                    && folders.iter().zip(dirs.iter()).all(|(f, d)| f == d)
            },
            &mut out,
        );
    }
    out
}

/// Select the `index`-th (1-based) request matching `name` in the collection.
pub fn select_request<'a>(
    collection: &'a Collection,
    name: &str,
    index: usize,
) -> Result<RequestMatch<'a>, String> {
    let mut matches = find_requests(collection, name);
    if matches.is_empty() {
        return Err(format!("request '{name}' not found"));
    }
    if index == 0 || index > matches.len() {
        return Err(format!(
            "--index {index} out of range: {} request(s) named '{name}'",
            matches.len()
        ));
    }
    Ok(matches.swap_remove(index - 1))
}

/// Depth-first search for a request by its item name (first match).
pub fn find_request<'a>(collection: &'a Collection, name: &str) -> Option<&'a Request> {
    find_requests(collection, name)
        .into_iter()
        .next()
        .map(|m| m.request)
}

/// Match a discovered collection by info.name OR by file stem.
fn collection_matches(loaded: &Loaded, selector: &str) -> bool {
    if loaded.collection.info.name == selector {
        return true;
    }
    loaded
        .path
        .file_stem()
        .map(|s| s.to_string_lossy() == selector)
        .unwrap_or(false)
}

/// Execute the send command. Returns the process exit code.
pub fn execute(args: &SendArgs, collections_override: &[String]) -> i32 {
    let workspace = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("golden: cannot read current dir: {e}");
            return FATAL;
        }
    };

    let files = discover(&workspace, collections_override, env_paths());
    if files.is_empty() {
        eprintln!("golden: no collections found");
        return FATAL;
    }

    // Find the target collection.
    let mut target: Option<Loaded> = None;
    for file in &files {
        match load(file) {
            Ok(l) => {
                if collection_matches(&l, &args.collection) {
                    target = Some(l);
                    break;
                }
            }
            Err(e) => {
                eprintln!("golden: {e}");
                return FATAL;
            }
        }
    }

    let Some(loaded) = target else {
        eprintln!("golden: collection '{}' not found", args.collection);
        return FATAL;
    };

    let found = match select_request(&loaded.collection, &args.request, args.index) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("golden: {e} in collection '{}'", args.collection);
            return FATAL;
        }
    };
    let request = found.request;

    let mut scopes = resolve(
        &loaded.workspace,
        &loaded.collections_root,
        &loaded.collection.variable,
    );
    if let Some(env_sel) = &args.env {
        apply_env_override(&loaded.workspace, env_sel, &mut scopes);
    }

    let cfg = HttpConfig {
        insecure: args.insecure,
        timeout_ms: args.timeout,
    };

    // --reporter json: machine output, one JSON object on stdout, nothing else.
    if args.reporter == SendReporterKind::Json {
        return execute_json(&loaded, &found, &scopes, &cfg, &workspace);
    }

    // --output: stream response body to a file instead of printing to stdout.
    if let Some(out_path) = &args.output {
        let policy = if args.force {
            OverwritePolicy::Overwrite
        } else {
            OverwritePolicy::Fail
        };
        let target = match resolve_target(out_path, policy) {
            Ok(t) => t,
            Err(_) => {
                // Interactive overwrite confirm when the file already exists.
                eprint!("file '{}' exists. overwrite? [y/N] ", out_path.display());
                let _ = io::stderr().flush();
                let mut answer = String::new();
                io::stdin().read_line(&mut answer).ok();
                if answer.trim().eq_ignore_ascii_case("y") {
                    out_path.clone()
                } else {
                    eprintln!("golden: aborted");
                    return FATAL;
                }
            }
        };
        return match download_to_file(request, scopes.as_map(), &cfg, &target, args.max_size) {
            Ok(info) => {
                println!(
                    "downloaded {} bytes -> {}",
                    info.bytes_written,
                    target.display()
                );
                if let Some(name) = info.suggested_filename {
                    eprintln!("server suggested filename: {name}");
                }
                0
            }
            Err(e) => {
                eprintln!("golden: {e}");
                FATAL
            }
        };
    }

    let stdout = io::stdout();
    let mut out = stdout.lock();
    match core_send(request, scopes.as_map(), &cfg) {
        Ok(resp) => {
            let url = substitute(request.url.raw(), scopes.as_map());
            // Record into request history (no-op when recording is disabled).
            // Unmasked so entries stay replayable (replay resends stored headers).
            // Use the cwd workspace so `golden history list/replay` (which key off
            // current_dir) read back exactly what `send` recorded.
            let entry = golden_core::history::HistoryEntry::from_sent(
                request,
                scopes.as_map(),
                Some(resp.status),
                resp.time_ms,
            );
            let _ = golden_core::history::append(&workspace, entry, false);
            let _ = writeln!(
                out,
                "{} {} -> {} ({}ms)",
                request.method, url, resp.status, resp.time_ms
            );
            for (k, v) in &resp.headers {
                let _ = writeln!(out, "{k}: {v}");
            }
            let _ = writeln!(out);
            let _ = out.write_all(&resp.body);
            let _ = writeln!(out);

            // --cookies: print Set-Cookie headers after the body.
            if args.cookies {
                let cs = extract_set_cookies(&resp.headers);
                if cs.is_empty() {
                    eprintln!("(no Set-Cookie headers)");
                } else {
                    eprintln!("Set-Cookie:");
                    for c in &cs {
                        eprintln!("  {c}");
                    }
                }
            }

            // --open: write HTML body to a temp file and open it in the browser.
            if args.open {
                let ct = resp
                    .headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
                if is_html(ct, &resp.body) {
                    match write_html_temp(&resp.body) {
                        Ok(path) => {
                            eprintln!("opening {}", path.display());
                            if let Err(e) = open::that(&path) {
                                eprintln!("could not open browser: {e}");
                            }
                        }
                        Err(e) => eprintln!("could not write preview: {e}"),
                    }
                } else {
                    eprintln!("--open ignored: response is not HTML");
                }
            }

            // exit 0 on success, 2 if the response is an error status
            if resp.status >= 400 {
                2
            } else {
                0
            }
        }
        Err(e) => {
            eprintln!("golden: {e}");
            FATAL
        }
    }
}

/// Cap on the response body inlined into the JSON report (5 MB).
const MAX_INLINE_BODY: usize = 5 * 1024 * 1024;

/// The machine-readable report printed by `send --reporter json`.
#[derive(Debug, Serialize)]
struct SendReport {
    request: RequestReport,
    response: Option<ResponseReport>,
    tests: Vec<TestReport>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct RequestReport {
    name: String,
    method: String,
    url: String,
    headers: Vec<KeyValue>,
}

#[derive(Debug, Serialize)]
struct ResponseReport {
    status: u16,
    status_text: String,
    headers: Vec<KeyValue>,
    cookies: Vec<CookieReport>,
    body_base64: String,
    body_truncated: bool,
    time_ms: u64,
    size_bytes: u64,
}

#[derive(Debug, Serialize)]
struct KeyValue {
    key: String,
    value: String,
}

#[derive(Debug, Serialize)]
struct CookieReport {
    name: String,
    value: String,
    raw: String,
}

#[derive(Debug, Serialize)]
struct TestReport {
    name: String,
    passed: bool,
    error: Option<String>,
}

/// Base64-encode a response body, inlining at most MAX_INLINE_BODY bytes.
/// Returns the encoded prefix and whether the body was truncated.
fn encode_body(body: &[u8]) -> (String, bool) {
    let truncated = body.len() > MAX_INLINE_BODY;
    let slice = if truncated {
        &body[..MAX_INLINE_BODY]
    } else {
        body
    };
    (BASE64.encode(slice), truncated)
}

/// Canonical HTTP reason phrase for a status code ("OK", "Not Found", ...).
fn status_text(status: u16) -> String {
    reqwest::StatusCode::from_u16(status)
        .ok()
        .and_then(|s| s.canonical_reason())
        .unwrap_or("")
        .to_string()
}

/// Split a raw Set-Cookie value into name/value, keeping the raw string too.
fn parse_cookie(raw: &str) -> CookieReport {
    let pair = raw.split(';').next().unwrap_or("");
    let (name, value) = match pair.split_once('=') {
        Some((n, v)) => (n.trim().to_string(), v.trim().to_string()),
        None => (pair.trim().to_string(), String::new()),
    };
    CookieReport {
        name,
        value,
        raw: raw.to_string(),
    }
}

/// `send --reporter json`: execute the request through the same script pipeline
/// as `golden run` (pre-request + test events via the sandbox) and print exactly
/// one pretty JSON object on stdout. Values are NOT masked — the machine
/// consumer decides what to hide.
fn execute_json(
    loaded: &Loaded,
    found: &RequestMatch,
    scopes: &VarScopes,
    cfg: &HttpConfig,
    workspace: &std::path::Path,
) -> i32 {
    let outcome = golden_core::runner::run_single(&loaded.collection, &found.path, scopes, cfg);
    let result = &outcome.result;

    // Record into request history exactly like the pretty path does.
    let entry = golden_core::history::HistoryEntry::from_sent(
        found.request,
        scopes.as_map(),
        result.status,
        result.time_ms,
    );
    let _ = golden_core::history::append(workspace, entry, false);

    let headers = found
        .request
        .header
        .iter()
        .filter(|h| !h.disabled)
        .map(|h| KeyValue {
            key: substitute(&h.key, scopes.as_map()),
            value: substitute(&h.value, scopes.as_map()),
        })
        .collect();

    let response = outcome.response.as_ref().map(|resp| {
        let (body_base64, body_truncated) = encode_body(&resp.body);
        ResponseReport {
            status: resp.status,
            status_text: status_text(resp.status),
            headers: resp
                .headers
                .iter()
                .map(|(k, v)| KeyValue {
                    key: k.clone(),
                    value: v.clone(),
                })
                .collect(),
            cookies: extract_set_cookies(&resp.headers)
                .iter()
                .map(|raw| parse_cookie(raw))
                .collect(),
            body_base64,
            body_truncated,
            time_ms: resp.time_ms as u64,
            size_bytes: resp.body.len() as u64,
        }
    });

    let report = SendReport {
        request: RequestReport {
            name: found.name.clone(),
            method: found.request.method.clone(),
            url: result.url.clone(),
            headers,
        },
        response,
        tests: result
            .assertions
            .iter()
            .map(|a| TestReport {
                name: a.name.clone(),
                passed: a.passed,
                error: a.error.clone(),
            })
            .collect(),
        error: result.error.clone(),
    };

    match serde_json::to_string_pretty(&report) {
        Ok(s) => println!("{s}"),
        Err(e) => {
            eprintln!("golden: {e}");
            return FATAL;
        }
    }

    // Mirror exit::code_for_result: 1 = assertion failure, 2 = transport/script
    // error or HTTP >= 400, 0 = clean.
    if result.assertions.iter().any(|a| !a.passed) {
        1
    } else if result.error.is_some() || result.status.map(|s| s >= 400).unwrap_or(true) {
        2
    } else {
        0
    }
}

fn apply_env_override(workspace: &std::path::Path, sel: &str, scopes: &mut VarScopes) {
    let candidate = {
        let direct = std::path::Path::new(sel);
        if direct.is_file() {
            direct.to_path_buf()
        } else {
            workspace.join(format!(".env.{sel}"))
        }
    };
    if let Ok(content) = std::fs::read_to_string(&candidate) {
        for (k, v) in parse_env(&content) {
            if !v.is_empty() {
                scopes.set(k, v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golden_core::download::{resolve_target, OverwritePolicy};
    use golden_core::http::{download_to_file, HttpConfig};
    use golden_core::model::{Collection, Request, Url};
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn coll() -> Collection {
        let json = r#"{
          "info":{"name":"Sample"},
          "item":[
            {"name":"auth","item":[
              {"name":"login","request":{"method":"POST","url":"https://x/login"}}
            ]},
            {"name":"health","request":{"method":"GET","url":"https://x/health"}}
          ]
        }"#;
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn finds_top_level_request_by_name() {
        let c = coll();
        let r = find_request(&c, "health").unwrap();
        assert_eq!(r.method, "GET");
    }

    #[test]
    fn finds_nested_request_by_name() {
        let c = coll();
        let r = find_request(&c, "login").unwrap();
        assert_eq!(r.method, "POST");
    }

    #[test]
    fn missing_request_returns_none() {
        let c = coll();
        assert!(find_request(&c, "nope").is_none());
    }

    /// Two folders each holding a request named "status", plus a top-level
    /// duplicate, and a request whose literal name contains '/'.
    fn dup_coll() -> Collection {
        let json = r#"{
          "info":{"name":"Dups"},
          "item":[
            {"name":"status","request":{"method":"GET","url":"https://x/top"}},
            {"name":"Users","item":[
              {"name":"status","request":{"method":"POST","url":"https://x/users/status"}},
              {"name":"Admin","item":[
                {"name":"status","request":{"method":"PUT","url":"https://x/admin/status"}}
              ]}
            ]},
            {"name":"a/b","request":{"method":"DELETE","url":"https://x/literal"}}
          ]
        }"#;
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn find_requests_returns_all_duplicates_in_depth_first_order() {
        let c = dup_coll();
        let all = find_requests(&c, "status");
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].path, vec![0]); // top-level
        assert_eq!(all[1].path, vec![1, 0]); // Users/status
        assert_eq!(all[2].path, vec![1, 1, 0]); // Users/Admin/status
    }

    #[test]
    fn select_request_picks_nth_match_one_based() {
        let c = dup_coll();
        assert_eq!(
            select_request(&c, "status", 1).unwrap().request.method,
            "GET"
        );
        assert_eq!(
            select_request(&c, "status", 2).unwrap().request.method,
            "POST"
        );
        assert_eq!(
            select_request(&c, "status", 3).unwrap().request.method,
            "PUT"
        );
    }

    #[test]
    fn select_request_rejects_out_of_range_index() {
        let c = dup_coll();
        let err = select_request(&c, "status", 4).unwrap_err();
        assert!(err.contains("out of range"), "{err}");
        assert!(err.contains("3 request(s)"), "{err}");
        assert!(select_request(&c, "status", 0).is_err());
        assert_eq!(
            select_request(&c, "nope", 1).unwrap_err(),
            "request 'nope' not found"
        );
    }

    #[test]
    fn folder_qualified_name_matches_exact_folder_path() {
        let c = dup_coll();
        let m = find_requests(&c, "Users/status");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].request.method, "POST");
        let m = find_requests(&c, "Users/Admin/status");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].request.method, "PUT");
        // Folder path must match from the root, not a suffix.
        assert!(find_requests(&c, "Admin/status").is_empty());
        assert!(find_requests(&c, "Nope/status").is_empty());
    }

    #[test]
    fn literal_request_name_with_slash_wins_over_qualified_lookup() {
        let c = dup_coll();
        let m = find_requests(&c, "a/b");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].request.method, "DELETE");
    }

    #[test]
    fn find_requests_returns_paths_usable_by_run_single() {
        let c = coll();
        let nested = find_requests(&c, "login");
        assert_eq!(nested.len(), 1);
        assert_eq!(nested[0].path, vec![0, 0]); // auth folder -> login
        assert_eq!(nested[0].name, "login");
        let top = find_requests(&c, "health");
        assert_eq!(top[0].path, vec![1]);
    }

    #[test]
    fn encode_body_inlines_small_bodies_without_truncation() {
        let (b64, truncated) = encode_body(b"hello");
        assert_eq!(b64, "aGVsbG8=");
        assert!(!truncated);
    }

    #[test]
    fn encode_body_caps_at_five_megabytes() {
        let body = vec![0u8; MAX_INLINE_BODY + 1];
        let (b64, truncated) = encode_body(&body);
        assert!(truncated);
        // The encoded prefix decodes back to exactly MAX_INLINE_BODY bytes.
        use base64::Engine as _;
        let decoded = super::BASE64.decode(b64).unwrap();
        assert_eq!(decoded.len(), MAX_INLINE_BODY);
    }

    #[test]
    fn status_text_maps_known_codes_and_blanks_unknown() {
        assert_eq!(status_text(200), "OK");
        assert_eq!(status_text(404), "Not Found");
        assert_eq!(status_text(599), "");
    }

    #[test]
    fn parse_cookie_splits_name_value_and_keeps_raw() {
        let c = parse_cookie("session=abc123; Path=/; HttpOnly");
        assert_eq!(c.name, "session");
        assert_eq!(c.value, "abc123");
        assert_eq!(c.raw, "session=abc123; Path=/; HttpOnly");
        // value containing '=' is kept whole
        let c = parse_cookie("tok=a=b; Secure");
        assert_eq!(c.value, "a=b");
        // no '=' at all -> name only
        let c = parse_cookie("weird");
        assert_eq!(c.name, "weird");
        assert_eq!(c.value, "");
    }

    /// The JSON report serializes with the exact field shape the VS Code
    /// extension consumes (request/response/tests/error).
    #[test]
    fn send_report_serializes_expected_shape() {
        let report = SendReport {
            request: RequestReport {
                name: "login".into(),
                method: "POST".into(),
                url: "https://api.test/login".into(),
                headers: vec![KeyValue {
                    key: "Authorization".into(),
                    value: "Bearer abc".into(),
                }],
            },
            response: Some(ResponseReport {
                status: 200,
                status_text: "OK".into(),
                headers: vec![],
                cookies: vec![parse_cookie("s=1; Path=/")],
                body_base64: "e30=".into(),
                body_truncated: false,
                time_ms: 12,
                size_bytes: 2,
            }),
            tests: vec![TestReport {
                name: "status 200".into(),
                passed: true,
                error: None,
            }],
            error: None,
        };
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string_pretty(&report).unwrap()).unwrap();
        assert_eq!(v["request"]["headers"][0]["key"], "Authorization");
        assert_eq!(v["response"]["status"], 200);
        assert_eq!(v["response"]["status_text"], "OK");
        assert_eq!(v["response"]["cookies"][0]["name"], "s");
        assert_eq!(v["response"]["body_truncated"], false);
        assert_eq!(v["tests"][0]["passed"], true);
        assert_eq!(v["error"], serde_json::Value::Null);
    }

    /// The download path resolves a non-existing target directly, writes to it,
    /// and reports bytes_written. This exercises the send --output flow end-to-end
    /// using golden_core primitives (no clap dispatch needed).
    #[test]
    fn download_branch_writes_response_to_file() {
        use httpmock::prelude::*;
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/data");
            then.status(200)
                .header("content-disposition", "attachment; filename=\"result.bin\"")
                .body("hello-world");
        });

        let dir = tempdir().unwrap();
        let out_path = dir.path().join("result.bin");

        // Non-existing target: resolve_target with Fail policy returns the path.
        let target = resolve_target(&out_path, OverwritePolicy::Fail).unwrap();
        assert_eq!(target, out_path);

        let req = Request {
            method: "GET".into(),
            url: Url::Raw(format!("{}/data", server.base_url())),
            header: vec![],
            body: None,
        };
        let info =
            download_to_file(&req, &HashMap::new(), &HttpConfig::default(), &target, None).unwrap();
        assert_eq!(info.bytes_written, 11);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hello-world");
        assert_eq!(info.suggested_filename.as_deref(), Some("result.bin"));
    }

    /// Existing file + Fail policy returns an Err — the caller should prompt.
    #[test]
    fn download_branch_fail_policy_errors_on_existing_file() {
        let dir = tempdir().unwrap();
        let out_path = dir.path().join("existing.bin");
        std::fs::write(&out_path, b"old").unwrap();

        let err = resolve_target(&out_path, OverwritePolicy::Fail).unwrap_err();
        assert!(
            err.contains("already exists"),
            "expected 'already exists' in: {err}"
        );
    }

    /// Existing file + Overwrite policy succeeds (same path returned).
    #[test]
    fn download_branch_force_overwrites_existing_file() {
        let dir = tempdir().unwrap();
        let out_path = dir.path().join("existing.bin");
        std::fs::write(&out_path, b"old").unwrap();

        let target = resolve_target(&out_path, OverwritePolicy::Overwrite).unwrap();
        assert_eq!(target, out_path);
    }

    /// --cookies flag: extract_set_cookies returns values from response headers.
    #[test]
    fn cookies_flag_extracts_set_cookie_headers() {
        use golden_core::viewers::extract_set_cookies;
        let headers = vec![
            ("Content-Type".to_string(), "text/html".to_string()),
            (
                "set-cookie".to_string(),
                "session=abc; Path=/; HttpOnly".to_string(),
            ),
            ("Set-Cookie".to_string(), "theme=dark; Path=/".to_string()),
        ];
        let cookies = extract_set_cookies(&headers);
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies[0], "session=abc; Path=/; HttpOnly");
        assert_eq!(cookies[1], "theme=dark; Path=/");
    }

    /// --open flag: write_html_temp creates a persistent .html file with correct content.
    #[test]
    fn open_flag_writes_html_temp_file() {
        use golden_core::viewers::write_html_temp;
        let body = b"<html><body><h1>Preview</h1></body></html>";
        let path = write_html_temp(body).unwrap();
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("html"));
        assert!(path.exists());
        let content = std::fs::read(&path).unwrap();
        assert_eq!(content, body);
        std::fs::remove_file(&path).ok();
    }

    /// --open flag: is_html detection works for content-type and body sniff.
    #[test]
    fn open_flag_html_detection() {
        use golden_core::viewers::is_html;
        assert!(is_html("text/html", b"<html></html>"));
        assert!(is_html("", b"<!DOCTYPE html><html></html>"));
        assert!(!is_html("application/json", b"{\"key\":\"value\"}"));
    }
}
