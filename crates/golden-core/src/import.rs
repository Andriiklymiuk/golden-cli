//! `golden import`: normalize Postman / raw-json into a v2.1 Collection and
//! apply add|replace|skip merge strategies. Ports welcomeViewProvider drop logic.

use std::path::{Path, PathBuf};

use crate::model::{Collection, Item, Request};

/// What to do when the destination collection file already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStrategy {
    /// Concatenate items of the incoming collection into the existing one.
    Add,
    /// Overwrite the existing collection with the incoming one.
    Replace,
    /// Leave the existing collection untouched.
    Skip,
}

/// Normalize arbitrary JSON into a v2.1 Collection named `name`.
/// - Full collection (has info+item): keep items, set info.name.
/// - Bare request (has method+url): wrap into a single-item collection.
pub fn normalize(json_text: &str, name: &str) -> Result<Collection, String> {
    let value: serde_json::Value =
        serde_json::from_str(json_text).map_err(|e| format!("invalid JSON: {e}"))?;
    let is_collection = value.get("info").is_some() && value.get("item").is_some();
    if is_collection {
        let mut coll: Collection =
            serde_json::from_value(value).map_err(|e| format!("invalid collection: {e}"))?;
        coll.info.name = name.to_string();
        Ok(coll)
    } else {
        // A bare request: prefer its own "name" for the wrapped item, falling
        // back to the collection name. Pull "name" out before parsing the Request
        // so it doesn't leak into the request's flattened `extra`.
        let mut value = value;
        let item_name = value
            .as_object_mut()
            .and_then(|o| o.remove("name"))
            .and_then(|v| v.as_str().map(str::to_string))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| name.to_string());
        let request: Request = serde_json::from_value(value)
            .map_err(|e| format!("not a collection or request: {e}"))?;
        let item = Item {
            name: item_name,
            description: None,
            item: None,
            request: Some(request),
            event: Vec::new(),
            extra: serde_json::Map::new(),
        };
        let coll_json = serde_json::json!({ "info": { "name": name }, "item": [] });
        let mut coll: Collection = serde_json::from_value(coll_json).map_err(|e| e.to_string())?;
        coll.item.push(item);
        Ok(coll)
    }
}

/// Parse a (single-line) curl command into a one-request collection.
/// Supports -X, -H, -d/--data/--data-raw, and a quoted URL. Best-effort.
pub fn curl_to_collection(command: &str, name: &str) -> Result<Collection, String> {
    let tokens = tokenize(command);
    if tokens.first().map(|s| s.as_str()) != Some("curl") {
        return Err("not a curl command".to_string());
    }
    let mut method = String::new();
    let mut url = String::new();
    let mut headers: Vec<crate::model::Header> = Vec::new();
    let mut body: Option<String> = None;
    let mut i = 1;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "-X" | "--request" => {
                i += 1;
                if i < tokens.len() {
                    method = tokens[i].clone();
                }
            }
            "-H" | "--header" => {
                i += 1;
                if i < tokens.len() {
                    if let Some((k, v)) = tokens[i].split_once(':') {
                        headers.push(crate::model::Header {
                            key: k.trim().to_string(),
                            value: v.trim().to_string(),
                            disabled: false,
                            extra: serde_json::Map::new(),
                        });
                    }
                }
            }
            "-d" | "--data" | "--data-raw" => {
                i += 1;
                if i < tokens.len() {
                    body = Some(tokens[i].clone());
                }
            }
            t if t.starts_with("http://") || t.starts_with("https://") => {
                url = t.to_string();
            }
            _ => {}
        }
        i += 1;
    }
    if url.is_empty() {
        return Err("no URL found in curl command".to_string());
    }
    if method.is_empty() {
        method = if body.is_some() {
            "POST".into()
        } else {
            "GET".into()
        };
    }
    let request = Request {
        method,
        url: crate::model::Url::Raw(url),
        header: headers,
        body: body.map(|raw| crate::model::Body {
            mode: "raw".into(),
            raw: Some(serde_json::Value::String(raw)),
            graphql: None,
            formdata: Vec::new(),
        }),
    };
    let coll_json = serde_json::json!({ "info": { "name": name }, "item": [] });
    let mut coll: Collection = serde_json::from_value(coll_json).map_err(|e| e.to_string())?;
    coll.item.push(Item {
        name: name.to_string(),
        description: None,
        item: None,
        request: Some(request),
        event: Vec::new(),
        extra: serde_json::Map::new(),
    });
    Ok(coll)
}

/// Minimal shell-ish tokenizer: splits on whitespace, respects single/double quotes.
fn tokenize(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in input.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => cur.push(ch),
            None if ch == '\'' || ch == '"' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            None => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Convert a (JSON) OpenAPI 3.x or Swagger 2.0 spec into a collection: one request
/// per path+method. Base URL = `servers[0].url` (OpenAPI 3.x) or
/// `schemes[0]://host + basePath` (Swagger 2.0).
/// Not supported: YAML input, $ref resolution, parameter extraction.
pub fn openapi_to_collection(spec_json: &str) -> Result<Collection, String> {
    let v: serde_json::Value =
        serde_json::from_str(spec_json).map_err(|e| format!("invalid OpenAPI JSON: {e}"))?;
    let title = v
        .get("info")
        .and_then(|i| i.get("title"))
        .and_then(|t| t.as_str())
        .unwrap_or("OpenAPI Import");
    let base = match v
        .get("servers")
        .and_then(|s| s.as_array())
        .and_then(|a| a.first())
        .and_then(|s| s.get("url"))
        .and_then(|u| u.as_str())
    {
        Some(url) => url.trim_end_matches('/').to_string(),
        None => {
            let host = v.get("host").and_then(|h| h.as_str()).unwrap_or("");
            if host.is_empty() {
                String::new()
            } else {
                let scheme = v
                    .get("schemes")
                    .and_then(|s| s.as_array())
                    .and_then(|a| a.first())
                    .and_then(|s| s.as_str())
                    .unwrap_or("https");
                let base_path = v.get("basePath").and_then(|b| b.as_str()).unwrap_or("");
                format!("{scheme}://{host}{base_path}")
                    .trim_end_matches('/')
                    .to_string()
            }
        }
    };

    let coll_json = serde_json::json!({ "info": { "name": title }, "item": [] });
    let mut coll: Collection = serde_json::from_value(coll_json).map_err(|e| e.to_string())?;

    let paths = v.get("paths").and_then(|p| p.as_object());
    if let Some(paths) = paths {
        for (path, methods) in paths {
            if let Some(ops) = methods.as_object() {
                for method in ops.keys() {
                    let m = method.to_uppercase();
                    if !["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"]
                        .contains(&m.as_str())
                    {
                        continue;
                    }
                    let url = format!("{base}{path}");
                    let request = Request {
                        method: m.clone(),
                        url: crate::model::Url::Raw(url),
                        header: Vec::new(),
                        body: None,
                    };
                    coll.item.push(Item {
                        name: format!("{m} {path}"),
                        description: None,
                        item: None,
                        request: Some(request),
                        event: Vec::new(),
                        extra: serde_json::Map::new(),
                    });
                }
            }
        }
    }
    Ok(coll)
}

/// Load all *.json files from a directory as collections. Invalid JSON and
/// non-collection files are skipped. Returns (source_path, Collection) pairs.
pub fn load_folder(dir: &Path) -> Result<Vec<(PathBuf, Collection)>, String> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("imported");
        if let Ok(coll) = normalize(&raw, name) {
            // restore the real info.name if the file was a full collection
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(real) = v
                    .get("info")
                    .and_then(|i| i.get("name"))
                    .and_then(|n| n.as_str())
                {
                    let mut coll = coll;
                    coll.info.name = real.to_string();
                    out.push((path, coll));
                    continue;
                }
            }
            out.push((path, coll));
        }
    }
    Ok(out)
}

/// Apply a merge strategy. Returns None for Skip (caller writes nothing).
pub fn merge(
    mut existing: Collection,
    incoming: Collection,
    strategy: MergeStrategy,
) -> Option<Collection> {
    match strategy {
        MergeStrategy::Add => {
            existing.item.extend(incoming.item);
            Some(existing)
        }
        MergeStrategy::Replace => Some(incoming),
        MergeStrategy::Skip => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn coll(name: &str, items: &[&str]) -> Collection {
        let items_json: Vec<String> = items
            .iter()
            .map(|n| {
                format!(r#"{{"name":"{n}","request":{{"method":"GET","url":"https://x/{n}"}}}}"#)
            })
            .collect();
        let json = format!(
            r#"{{"info":{{"name":"{name}"}},"item":[{}]}}"#,
            items_json.join(",")
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn normalize_wraps_bare_request_json() {
        // A raw request (no info/item) is wrapped into a single-item collection.
        let raw = r#"{"method":"POST","url":"https://x/login"}"#;
        let c = normalize(raw, "Login").unwrap();
        assert_eq!(c.info.name, "Login");
        assert_eq!(c.item.len(), 1);
        assert_eq!(c.item[0].name, "Login");
        assert!(c.item[0].request.is_some());
    }

    #[test]
    fn normalize_bare_request_uses_its_own_name() {
        // A bare request carrying its own "name" → the wrapped item takes that
        // name, not the collection's --name. This is what makes adding a single
        // request to an existing collection (import --strategy add) name it right.
        let raw = r#"{"name":"Health","method":"GET","url":"https://x/health"}"#;
        let c = normalize(raw, "Core").unwrap();
        assert_eq!(c.info.name, "Core"); // collection still takes --name
        assert_eq!(c.item.len(), 1);
        assert_eq!(c.item[0].name, "Health"); // item takes the request's own name
        let r = c.item[0].request.as_ref().unwrap();
        assert_eq!(r.method, "GET");
    }

    #[test]
    fn normalize_keeps_existing_collection_but_sets_name() {
        let raw = r#"{"info":{"name":"Old"},"item":[{"name":"a","request":{"method":"GET","url":"https://x/a"}}]}"#;
        let c = normalize(raw, "New").unwrap();
        assert_eq!(c.info.name, "New");
        assert_eq!(c.item.len(), 1);
    }

    #[test]
    fn merge_add_concatenates_items() {
        let existing = coll("E", &["a"]);
        let incoming = coll("I", &["b", "c"]);
        let merged = merge(existing, incoming, MergeStrategy::Add);
        assert_eq!(merged.unwrap().item.len(), 3);
    }

    #[test]
    fn merge_replace_returns_incoming() {
        let existing = coll("E", &["a"]);
        let incoming = coll("I", &["b"]);
        let merged = merge(existing, incoming, MergeStrategy::Replace).unwrap();
        assert_eq!(merged.info.name, "I");
        assert_eq!(merged.item.len(), 1);
    }

    #[test]
    fn merge_skip_returns_none() {
        let existing = coll("E", &["a"]);
        let incoming = coll("I", &["b"]);
        assert!(merge(existing, incoming, MergeStrategy::Skip).is_none());
    }

    #[test]
    fn curl_command_imports_as_single_request() {
        let cmd = "curl -X POST 'https://api.test/login' -H 'Content-Type: application/json' -d '{\"u\":\"x\"}'";
        let c = curl_to_collection(cmd, "Login").unwrap();
        assert_eq!(c.item.len(), 1);
        let r = c.item[0].request.as_ref().unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.url.raw(), "https://api.test/login");
        assert_eq!(r.header[0].key, "Content-Type");
        assert_eq!(
            r.body
                .as_ref()
                .unwrap()
                .raw
                .as_ref()
                .and_then(|v| v.as_str()),
            Some(r#"{"u":"x"}"#)
        );
    }

    #[test]
    fn openapi_paths_import_as_requests() {
        let spec = r#"{
          "openapi":"3.0.0",
          "info":{"title":"API","version":"1"},
          "servers":[{"url":"https://api.test"}],
          "paths":{
            "/users":{"get":{"summary":"List"},"post":{"summary":"Create"}}
          }
        }"#;
        let c = openapi_to_collection(spec).unwrap();
        assert_eq!(c.info.name, "API");
        let names: Vec<&str> = c.item.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"GET /users"));
        assert!(names.contains(&"POST /users"));
        let get = c.item.iter().find(|i| i.name == "GET /users").unwrap();
        assert_eq!(
            get.request.as_ref().unwrap().url.raw(),
            "https://api.test/users"
        );
    }

    #[test]
    fn openapi_31_imports_via_servers() {
        let spec = r#"{"openapi":"3.1.0","info":{"title":"V31"},"servers":[{"url":"https://api.test/v3"}],"paths":{"/ping":{"get":{}}}}"#;
        let c = openapi_to_collection(spec).unwrap();
        assert_eq!(c.item.len(), 1);
        assert_eq!(
            c.item[0].request.as_ref().unwrap().url.raw(),
            "https://api.test/v3/ping"
        );
    }

    #[test]
    fn swagger_2_imports_via_host_basepath_schemes() {
        let spec = r#"{"swagger":"2.0","info":{"title":"V2"},"host":"api.test","basePath":"/v2","schemes":["https"],"paths":{"/ping":{"get":{}}}}"#;
        let c = openapi_to_collection(spec).unwrap();
        assert_eq!(c.item.len(), 1);
        assert_eq!(
            c.item[0].request.as_ref().unwrap().url.raw(),
            "https://api.test/v2/ping"
        );
    }

    #[test]
    fn import_folder_loads_all_json_skipping_invalid() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("one.json"),
            r#"{"info":{"name":"One"},"item":[]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("two.json"),
            r#"{"info":{"name":"Two"},"item":[]}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("bad.json"), "not json").unwrap();
        std::fs::write(dir.path().join("ignore.txt"), "x").unwrap();
        let loaded = load_folder(dir.path()).unwrap();
        let mut names: Vec<String> = loaded.iter().map(|(_, c)| c.info.name.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["One".to_string(), "Two".to_string()]);
    }
}
