//! `golden openapi` — convert discovered collections into an OpenAPI 3.0 spec.
//!
//! Each request becomes a path operation: method + path (with `{{var}}` / `:param` →
//! `{param}`), query + path parameters, a request body example from the raw/GraphQL body,
//! and a generic response set. Tagged by collection name so Swagger UI groups them.

use std::collections::BTreeMap;
use std::io::Write;

use golden_core::model::{Collection, Item, Request};
use serde_json::{json, Map, Value};

use crate::cli::OpenapiArgs;
use crate::discovery::{discover, env_paths, expand_paths};
use crate::exit::FATAL;
use crate::load::load;

const WRITE_METHODS: [&str; 3] = ["post", "put", "patch"];

/// Execute `golden openapi`. Returns the process exit code.
pub fn execute(args: &OpenapiArgs, collections_override: &[String]) -> i32 {
    let workspace = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("golden: cannot read current dir: {e}");
            return FATAL;
        }
    };
    let files = if args.paths.is_empty() {
        discover(&workspace, collections_override, env_paths())
    } else {
        expand_paths(&workspace, &args.paths)
    };
    if files.is_empty() {
        eprintln!("golden: no collections found");
        return FATAL;
    }

    let mut collections = Vec::new();
    for file in &files {
        match load(file) {
            Ok(loaded) => collections.push(loaded.collection),
            Err(e) => {
                eprintln!("golden: {e}");
                return FATAL;
            }
        }
    }

    let spec = build_spec(&collections, args);
    let text = serde_json::to_string_pretty(&spec).unwrap_or_default();
    match &args.output {
        Some(path) => {
            if let Err(e) = std::fs::write(path, format!("{text}\n")) {
                eprintln!("golden: cannot write {path}: {e}");
                return FATAL;
            }
            eprintln!("golden: wrote OpenAPI 3.0 spec to {path}");
        }
        None => {
            let _ = writeln!(std::io::stdout(), "{text}");
        }
    }
    0
}

fn build_spec(collections: &[Collection], args: &OpenapiArgs) -> Value {
    let title = args
        .title
        .clone()
        .or_else(|| collections.first().map(|c| c.info.name.clone()))
        .unwrap_or_else(|| "API".to_string());
    let servers: Vec<Value> = if args.server.is_empty() {
        vec![json!({ "url": "{{baseUrl}}" })]
    } else {
        args.server
            .iter()
            .map(|entry| match entry.split_once('=') {
                // `NAME=URL` → named server; a bare URL (scheme before any '=') stays as-is.
                Some((name, url)) if !name.contains("://") && url.contains("://") => {
                    json!({ "url": url, "description": name })
                }
                _ => json!({ "url": entry }),
            })
            .collect()
    };

    // path -> { method -> operation }
    let mut paths: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    // collection name -> ordered unique tag list (drives x-tagGroups two-level nav)
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for collection in collections {
        let mut tags = Vec::new();
        add_items(
            &collection.item,
            &collection.info.name,
            None,
            &mut tags,
            &mut paths,
            &mut skipped,
        );
        if !tags.is_empty() {
            groups.push((collection.info.name.clone(), tags));
        }
    }
    for (key, count) in &skipped {
        // Many requests can legitimately share one path+method (e.g. GraphQL endpoints:
        // every query/mutation is a POST to the same URL). OpenAPI allows one operation
        // per path+method, so the first request maps and the rest are summarized here.
        eprintln!(
            "golden: openapi: {key} maps {count} extra request(s) — kept the first operation only"
        );
    }

    // x-tagGroups: collection -> its folder tags. Renderers that support it (Scalar,
    // Redoc) show a two-level sidebar; others ignore the vendor extension. The tag
    // declarations carry x-displayName so the sidebar shows just the folder name
    // while the tag itself stays globally unique.
    let tag_groups: Vec<Value> = groups
        .iter()
        .map(|(name, tags)| json!({ "name": name, "tags": tags }))
        .collect();
    let tag_decls: Vec<Value> = groups
        .iter()
        .flat_map(|(name, tags)| {
            tags.iter().map(move |tag| {
                let display = tag
                    .strip_prefix(name.as_str())
                    .and_then(|rest| rest.strip_prefix(" / "))
                    .unwrap_or(tag);
                json!({ "name": tag, "x-displayName": display })
            })
        })
        .collect();

    json!({
        "openapi": "3.0.3",
        "info": {
            "title": title,
            "version": "1.0.0",
            "description": "Generated from golden collections by `golden openapi`."
        },
        "servers": servers,
        "tags": tag_decls,
        "paths": paths,
        "x-tagGroups": tag_groups,
    })
}

#[allow(clippy::too_many_arguments)]
fn add_items(
    items: &[Item],
    collection: &str,
    folder: Option<&str>,
    tags: &mut Vec<String>,
    paths: &mut BTreeMap<String, Map<String, Value>>,
    skipped: &mut BTreeMap<String, usize>,
) {
    for item in items {
        match &item.item {
            Some(children) => {
                add_items(children, collection, Some(&item.name), tags, paths, skipped);
            }
            None => {
                if let Some(request) = &item.request {
                    // Tag = "<collection> / <folder>" — folder names repeat across
                    // collections, so the prefix keeps tags globally unique.
                    let tag = match folder {
                        Some(f) => format!("{collection} / {f}"),
                        None => collection.to_string(),
                    };
                    if !tags.contains(&tag) {
                        tags.push(tag.clone());
                    }
                    add_operation(&item.name, request, &tag, paths, skipped);
                }
            }
        }
    }
}

fn add_operation(
    name: &str,
    request: &Request,
    tag: &str,
    paths: &mut BTreeMap<String, Map<String, Value>>,
    skipped: &mut BTreeMap<String, usize>,
) {
    let path = openapi_path(request.url.raw());
    if path.is_empty() {
        return;
    }
    let method = request.method.to_lowercase();

    let mut params: Vec<Value> = path_params(&path)
        .into_iter()
        .map(|p| param(&p, "path", true))
        .collect();
    for key in query_keys(request.url.raw()) {
        params.push(param(&key, "query", false));
    }

    let mut op = Map::new();
    op.insert("summary".into(), json!(name));
    op.insert("operationId".into(), json!(operation_id(&method, &path)));
    op.insert("tags".into(), json!([tag]));
    if !params.is_empty() {
        op.insert("parameters".into(), json!(params));
    }
    if WRITE_METHODS.contains(&method.as_str()) {
        if let Some(body) = request_body(request) {
            op.insert("requestBody".into(), body);
        }
    }
    op.insert(
        "responses".into(),
        json!({
            "200": { "description": "Successful response" },
            "default": { "description": "Error response" }
        }),
    );

    let methods = paths.entry(path.clone()).or_default();
    if methods.contains_key(&method) {
        // One operation per path+method in OpenAPI; never silently overwrite.
        *skipped
            .entry(format!("{} {path}", method.to_uppercase()))
            .or_default() += 1;
        return;
    }
    methods.insert(method, Value::Object(op));
}

/// Strip the scheme/host (or `{{baseUrl}}` token) + query, then turn `{{var}}` and
/// `:param` segments into OpenAPI `{param}`.
fn openapi_path(raw: &str) -> String {
    let no_query = raw.split(['?', '#']).next().unwrap_or(raw);
    let after_host = if let Some(pos) = no_query.find("://") {
        let after = &no_query[pos + 3..];
        after.find('/').map_or("/", |i| &after[i..])
    } else {
        no_query.find('/').map_or("", |i| &no_query[i..])
    };
    let braced = after_host.replace("{{", "{").replace("}}", "}");
    braced
        .split('/')
        .map(|seg| match seg.strip_prefix(':') {
            Some(rest) if !rest.is_empty() => format!("{{{rest}}}"),
            _ => seg.to_string(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn path_params(path: &str) -> Vec<String> {
    path.split('/')
        .filter_map(|seg| {
            let inner = seg.strip_prefix('{')?.strip_suffix('}')?;
            (!inner.is_empty()).then(|| inner.to_string())
        })
        .collect()
}

fn query_keys(raw: &str) -> Vec<String> {
    let Some(query) = raw.split('?').nth(1) else {
        return Vec::new();
    };
    let query = query.split('#').next().unwrap_or(query);
    query
        .split('&')
        .filter_map(|pair| {
            let key = pair.split('=').next().unwrap_or("").trim();
            (!key.is_empty()).then(|| key.to_string())
        })
        .collect()
}

fn param(name: &str, location: &str, required: bool) -> Value {
    json!({
        "name": name,
        "in": location,
        "required": required,
        "schema": { "type": "string" }
    })
}

fn request_body(request: &Request) -> Option<Value> {
    let body = request.body.as_ref()?;
    let example = if body.mode == "graphql" {
        let query = body
            .graphql
            .as_ref()
            .map(|g| g.query.clone())
            .unwrap_or_default();
        json!({ "query": query })
    } else {
        match body.raw.as_ref()? {
            Value::String(s) => {
                serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.clone()))
            }
            other => other.clone(),
        }
    };
    Some(json!({
        "content": { "application/json": { "schema": { "type": "object" }, "example": example } }
    }))
}

fn operation_id(method: &str, path: &str) -> String {
    let slug: String = path
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{method}{slug}").trim_matches('_').to_string()
}
