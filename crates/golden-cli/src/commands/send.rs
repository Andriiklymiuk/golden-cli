//! `golden send <collection> <request>`: locate one request across discovered
//! collections, send it via golden_core, and print the response.

use std::io::{self, Write};

use golden_core::download::{resolve_target, OverwritePolicy};
use golden_core::env::{parse_env, resolve, VarScopes};
use golden_core::http::{download_to_file, send as core_send, HttpConfig};
use golden_core::model::{Collection, Item, Request};
use golden_core::subst::substitute;
use golden_core::viewers::{extract_set_cookies, is_html, write_html_temp};

use crate::cli::SendArgs;
use crate::discovery::{discover, env_paths};
use crate::exit::FATAL;
use crate::load::{load, Loaded};

/// Depth-first search for a request by its item name.
pub fn find_request<'a>(collection: &'a Collection, name: &str) -> Option<&'a Request> {
    fn walk<'a>(items: &'a [Item], name: &str) -> Option<&'a Request> {
        for item in items {
            if item.is_request() && item.name == name {
                return item.request.as_ref();
            }
            if let Some(children) = &item.item {
                if let Some(found) = walk(children, name) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(&collection.item, name)
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

    let Some(request) = find_request(&loaded.collection, &args.request) else {
        eprintln!(
            "golden: request '{}' not found in collection '{}'",
            args.request, args.collection
        );
        return FATAL;
    };

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
