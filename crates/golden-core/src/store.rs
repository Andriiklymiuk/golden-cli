//! On-disk persistence + tree mutation for Postman v2.1 collections.
//! Replicates the VSCode extension's collectionsProvider semantics so files
//! round-trip byte-identically (2-space indent, lowercase-hyphenated names).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::model::{
    Body, Collection, Event, Graphql, Header, Item, Request, Script, Url, Variable,
};

/// Read + parse a collection file. Errors are human-readable strings.
pub fn load_collection(path: &Path) -> Result<Collection, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("parse {}: {e}", path.display()))
}

/// Serialize a collection exactly like the extension: `JSON.stringify(x, null, 2)`
/// (serde_json pretty default == 2 spaces). No trailing newline (matches the
/// extension, which does not append one).
pub fn to_pretty(coll: &Collection) -> Result<String, String> {
    serde_json::to_string_pretty(coll).map_err(|e| format!("serialize: {e}"))
}

/// Atomically write a collection back to `path` (temp file in the same dir,
/// fsync, rename over the target — see write_atomic).
pub fn save_collection(path: &Path, coll: &Collection) -> Result<(), String> {
    let text = to_pretty(coll)?;
    write_atomic(path, text.as_bytes())
}

/// Write `bytes` to `path` atomically: write to a temp file in the SAME directory
/// (so rename is atomic on the same filesystem), flush + fsync, then rename over
/// the destination. Avoids the extension's partial-write risk.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".golden-tmp-")
        .suffix(".json")
        .tempfile_in(dir)
        .map_err(|e| format!("temp file in {}: {e}", dir.display()))?;
    tmp.write_all(bytes)
        .map_err(|e| format!("write temp: {e}"))?;
    tmp.as_file_mut()
        .sync_all()
        .map_err(|e| format!("fsync temp: {e}"))?;
    tmp.persist(path)
        .map_err(|e| format!("rename onto {}: {}", path.display(), e.error))?;
    Ok(())
}

/// Extension filename rule: name.toLowerCase().replace(/\s+/g, '-') + ".json".
pub fn collection_filename(name: &str) -> String {
    format!("{}.json", slugify(name))
}

/// Lowercase, collapse runs of ASCII whitespace into a single '-'. Mirrors
/// JS `String.toLowerCase().replace(/\s+/g, '-')` (only \s, not punctuation).
pub fn slugify(name: &str) -> String {
    let lower = name.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut in_ws = false;
    for ch in lower.chars() {
        if ch.is_whitespace() {
            if !in_ws {
                out.push('-');
                in_ws = true;
            }
        } else {
            out.push(ch);
            in_ws = false;
        }
    }
    out
}

/// Path for a collection named `name` inside directory `dir`.
pub fn collection_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(collection_filename(name))
}

// ── index-path lookup ──────────────────────────────────────────────────────

/// Walk `items` following an index path (each element = position in the current
/// `item[]`). Returns the addressed Item, or None if any index is out of range.
pub fn item_at_path_mut<'a>(items: &'a mut [Item], path: &[usize]) -> Option<&'a mut Item> {
    let (first, rest) = path.split_first()?;
    let item = items.get_mut(*first)?;
    if rest.is_empty() {
        Some(item)
    } else {
        item.item
            .as_mut()
            .and_then(|children| item_at_path_mut(children, rest))
    }
}

fn request_at_path_mut<'a>(
    items: &'a mut [Item],
    path: &[usize],
) -> Result<&'a mut Request, String> {
    item_at_path_mut(items, path)
        .ok_or_else(|| "item not found at path".to_string())?
        .request
        .as_mut()
        .ok_or_else(|| "item at path is not a request".to_string())
}

// ── method / url / script editing ─────────────────────────────────────────

pub fn set_method(items: &mut [Item], path: &[usize], method: &str) -> Result<(), String> {
    request_at_path_mut(items, path)?.method = method.to_string();
    Ok(())
}

/// Set the URL. If the existing URL is a parsed object, keep the object shape
/// (host/path/query extras preserved) and update only `raw`. Otherwise store a
/// raw string. (The extension only ever edits `raw`.)
pub fn set_url(items: &mut [Item], path: &[usize], url: &str) -> Result<(), String> {
    let req = request_at_path_mut(items, path)?;
    match &mut req.url {
        Url::Raw(s) => *s = url.to_string(),
        Url::Parsed { raw, .. } => *raw = url.to_string(),
    }
    Ok(())
}

/// Set (create or replace) the script for an event `listen` ("prerequest" | "test").
/// Empty `exec` removes the event entirely (matches the extension dropping empty scripts).
pub fn set_script(
    items: &mut [Item],
    path: &[usize],
    listen: &str,
    exec: &[String],
) -> Result<(), String> {
    let item = item_at_path_mut(items, path).ok_or_else(|| "item not found at path".to_string())?;
    if exec.is_empty() {
        item.event.retain(|e| e.listen != listen);
        return Ok(());
    }
    if let Some(ev) = item.event.iter_mut().find(|e| e.listen == listen) {
        ev.script.exec = exec.to_vec();
    } else {
        item.event.push(Event {
            listen: listen.to_string(),
            script: Script {
                script_type: None,
                exec: exec.to_vec(),
                extra: serde_json::Map::new(),
            },
        });
    }
    Ok(())
}

// ── header editing ─────────────────────────────────────────────────────────

/// Replace the entire header table.
pub fn set_headers(items: &mut [Item], path: &[usize], headers: Vec<Header>) -> Result<(), String> {
    request_at_path_mut(items, path)?.header = headers;
    Ok(())
}

/// Bulk-edit headers from a JSON array of {key,value,disabled?} (the extension's
/// header bulk-edit textarea).
pub fn set_headers_from_json(items: &mut [Item], path: &[usize], json: &str) -> Result<(), String> {
    let headers: Vec<Header> =
        serde_json::from_str(json).map_err(|e| format!("parse headers JSON: {e}"))?;
    set_headers(items, path, headers)
}

// ── query-param editing ────────────────────────────────────────────────────

/// Parse `key=value` pairs from the query string of a raw URL (percent-decoded).
pub fn parse_query_params(url: &str) -> Vec<(String, String)> {
    let Some(q) = url.split_once('?').map(|(_, q)| q) else {
        return Vec::new();
    };
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

/// Rebuild a raw URL from its base (everything before '?') plus encoded params.
/// Empty params drop the '?' entirely (matches the extension's URL↔table sync).
pub fn build_url_with_params(url: &str, params: &[(String, String)]) -> String {
    let base = url.split_once('?').map(|(b, _)| b).unwrap_or(url);
    if params.is_empty() {
        return base.to_string();
    }
    let query = params
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{query}")
}

/// Minimal percent-encoding for query components: encode everything that is not
/// an RFC-3986 unreserved char (ALPHA / DIGIT / -._~).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ── body + variable editing ────────────────────────────────────────────────

/// Set a raw body (mode="raw"), replacing any existing body.
/// The text is stored as `serde_json::Value::String` to match the Postman on-disk format.
pub fn set_raw_body(items: &mut [Item], path: &[usize], raw: &str) -> Result<(), String> {
    request_at_path_mut(items, path)?.body = Some(Body {
        mode: "raw".to_string(),
        raw: Some(serde_json::Value::String(raw.to_string())),
        graphql: None,
        formdata: vec![],
    });
    Ok(())
}

/// Set a graphql body (mode="graphql"). `variables_json`, when Some, must parse as
/// JSON; None leaves variables absent so it serializes away (round-trip safe).
pub fn set_graphql_body(
    items: &mut [Item],
    path: &[usize],
    query: &str,
    variables_json: Option<&str>,
) -> Result<(), String> {
    let variables = match variables_json {
        Some(j) if !j.trim().is_empty() => {
            Some(serde_json::from_str(j).map_err(|e| format!("parse graphql variables: {e}"))?)
        }
        _ => None,
    };
    request_at_path_mut(items, path)?.body = Some(Body {
        mode: "graphql".to_string(),
        raw: None,
        graphql: Some(Graphql {
            query: query.to_string(),
            variables,
        }),
        formdata: vec![],
    });
    Ok(())
}

/// Remove the body entirely.
pub fn clear_body(items: &mut [Item], path: &[usize]) -> Result<(), String> {
    request_at_path_mut(items, path)?.body = None;
    Ok(())
}

/// Add or update a collection variable by key (preserves the existing `extra` map,
/// e.g. Postman's `"type":"string"`, when editing).
pub fn set_variable(coll: &mut Collection, key: &str, value: &str) {
    if let Some(v) = coll.variable.iter_mut().find(|v| v.key == key) {
        v.value = value.to_string();
    } else {
        coll.variable.push(Variable {
            key: key.to_string(),
            value: value.to_string(),
            extra: serde_json::Map::new(),
        });
    }
}

/// Delete a collection variable by key. Returns true if one was removed.
pub fn delete_variable(coll: &mut Collection, key: &str) -> bool {
    let before = coll.variable.len();
    coll.variable.retain(|v| v.key != key);
    coll.variable.len() != before
}

// ── create operations ──────────────────────────────────────────────────────

const POSTMAN_SCHEMA: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

/// Create an empty v2.1 collection file in `dir`. Mirrors the extension's
/// createCollection: fresh _postman_id + _exporter_id, schema,
/// lowercase-hyphen filename. Errors if the file already exists.
pub fn create_collection(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let path = collection_path(dir, name);
    if path.exists() {
        return Err(format!(
            "Collection \"{name}\" already exists in this folder"
        ));
    }
    let mut info_extra = serde_json::Map::new();
    info_extra.insert(
        "_postman_id".into(),
        serde_json::Value::String(uuid::Uuid::new_v4().to_string()),
    );
    info_extra.insert(
        "schema".into(),
        serde_json::Value::String(POSTMAN_SCHEMA.into()),
    );
    info_extra.insert(
        "_exporter_id".into(),
        serde_json::Value::String(uuid::Uuid::new_v4().to_string()),
    );
    let coll = Collection {
        info: crate::model::Info {
            name: name.to_string(),
            extra: info_extra,
        },
        item: Vec::new(),
        variable: Vec::new(),
        extra: serde_json::Map::new(),
    };
    save_collection(&path, &coll)?;
    Ok(path)
}

/// Build a fresh request Item matching the extension's addRequest default.
fn new_request_item(name: &str, method: &str) -> Item {
    let mut url_extra = serde_json::Map::new();
    url_extra.insert("protocol".into(), serde_json::Value::String("https".into()));
    url_extra.insert("host".into(), serde_json::json!(["example", "com"]));
    Item {
        name: name.to_string(),
        description: None,
        item: None,
        request: Some(Request {
            method: method.to_string(),
            url: Url::Parsed {
                raw: "https://example.com".to_string(),
                extra: url_extra,
            },
            header: Vec::new(),
            body: None,
        }),
        event: Vec::new(),
        extra: serde_json::Map::new(),
    }
}

/// Add a request to the container addressed by `path`. Empty path == top level.
pub fn add_request(
    items: &mut Vec<Item>,
    path: &[usize],
    name: &str,
    method: &str,
) -> Result<(), String> {
    let item = new_request_item(name, method);
    push_into_container(items, path, item)
}

/// Add a logical folder to the container addressed by `path`. Empty path == top level.
pub fn add_folder(items: &mut Vec<Item>, path: &[usize], name: &str) -> Result<(), String> {
    let folder = Item {
        name: name.to_string(),
        description: None,
        item: Some(Vec::new()),
        request: None,
        event: Vec::new(),
        extra: serde_json::Map::new(),
    };
    push_into_container(items, path, folder)
}

/// Push `new_item` into the `item[]` array of the node at `path`. Empty path
/// pushes to the top-level `items`. Errors if the path addresses a non-folder.
fn push_into_container(
    items: &mut Vec<Item>,
    path: &[usize],
    new_item: Item,
) -> Result<(), String> {
    if path.is_empty() {
        items.push(new_item);
        return Ok(());
    }
    let parent =
        item_at_path_mut(items, path).ok_or_else(|| "parent not found at path".to_string())?;
    let children = parent.item.get_or_insert_with(Vec::new);
    children.push(new_item);
    Ok(())
}

/// Create a physical folder (filesystem directory). Errors if it exists.
pub fn create_physical_folder(parent: &Path, name: &str) -> Result<PathBuf, String> {
    let dir = parent.join(name);
    if dir.exists() {
        return Err(format!("Folder \"{name}\" already exists"));
    }
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    Ok(dir)
}

// ── delete + rename operations ─────────────────────────────────────────────

/// Delete the first item with `name` (depth-first, recursive). Returns true if removed.
pub fn delete_item_by_name(items: &mut Vec<Item>, name: &str) -> bool {
    if let Some(pos) = items.iter().position(|i| i.name == name) {
        items.remove(pos);
        return true;
    }
    for item in items.iter_mut() {
        if let Some(children) = item.item.as_mut() {
            if delete_item_by_name(children, name) {
                return true;
            }
        }
    }
    false
}

/// Rename the first item named `old` to `new` (depth-first, recursive). Returns true if renamed.
pub fn rename_item_by_name(items: &mut [Item], old: &str, new: &str) -> bool {
    for item in items.iter_mut() {
        if item.name == old {
            item.name = new.to_string();
            return true;
        }
        if let Some(children) = item.item.as_mut() {
            if rename_item_by_name(children, old, new) {
                return true;
            }
        }
    }
    false
}

/// Rename a collection: set info.name, write to the new lowercase-hyphen path,
/// remove the old file if the slug changed. Returns the resulting path.
pub fn rename_collection(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    let mut coll = load_collection(path)?;
    coll.info.name = new_name.to_string();
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let new_path = collection_path(dir, new_name);
    save_collection(&new_path, &coll)?;
    if new_path != path {
        fs::remove_file(path).map_err(|e| format!("remove old {}: {e}", path.display()))?;
    }
    Ok(new_path)
}

/// Delete a collection file.
pub fn delete_collection_file(path: &Path) -> Result<(), String> {
    fs::remove_file(path).map_err(|e| format!("remove {}: {e}", path.display()))
}

/// Rename a physical folder (filesystem dir). Errors if the target exists.
pub fn rename_physical_folder(dir: &Path, new_name: &str) -> Result<PathBuf, String> {
    let parent = dir.parent().unwrap_or_else(|| Path::new("."));
    let new_path = parent.join(new_name);
    if new_path.exists() {
        return Err(format!("Folder \"{new_name}\" already exists"));
    }
    fs::rename(dir, &new_path).map_err(|e| format!("rename folder: {e}"))?;
    Ok(new_path)
}

/// Delete a physical folder and all contents (recursive).
pub fn delete_physical_folder(dir: &Path) -> Result<(), String> {
    fs::remove_dir_all(dir).map_err(|e| format!("remove dir {}: {e}", dir.display()))
}

// ── cross-collection move (atomic two-file write) ─────────────────────────

/// Find a folder by name (recursive) in a tree and return a mutable ref to its
/// children Vec. Used as the cross-move target container.
fn folder_children_mut<'a>(items: &'a mut [Item], folder_name: &str) -> Option<&'a mut Vec<Item>> {
    // First pass: direct match at this level (use position + index to satisfy borrow checker)
    if let Some(pos) = items.iter().position(|i| i.name == folder_name) {
        return items[pos].item.as_mut();
    }
    // Second pass: recurse into children
    for item in items.iter_mut() {
        if let Some(children) = item.item.as_mut() {
            if let Some(found) = folder_children_mut(children, folder_name) {
                return Some(found);
            }
        }
    }
    None
}

/// Remove the first item named `name` (recursive, depth-first) and return it.
fn take_item_by_name(items: &mut Vec<Item>, name: &str) -> Option<Item> {
    if let Some(pos) = items.iter().position(|i| i.name == name) {
        return Some(items.remove(pos));
    }
    for item in items.iter_mut() {
        if let Some(children) = item.item.as_mut() {
            if let Some(found) = take_item_by_name(children, name) {
                return Some(found);
            }
        }
    }
    None
}

/// Move a request or folder named `item_name` from `src_path` to `dst_path`.
/// `dst_folder` = Some(folder_name) drops it into that logical folder, None into
/// the target collection root. Validates BOTH mutations in memory first, then
/// writes both files atomically (temp+rename each). If the in-memory validation
/// fails, neither file is written (clean rollback). If the second write fails
/// after the first succeeded, the source change is reverted to the on-disk backup.
pub fn move_item_across_collections(
    src_path: &Path,
    item_name: &str,
    dst_path: &Path,
    dst_folder: Option<&str>,
) -> Result<(), String> {
    let mut src = load_collection(src_path)?;
    let mut dst = load_collection(dst_path)?;
    let src_backup = to_pretty(&src)?; // for rollback of a half-applied write

    // 1) remove from source (in memory)
    let removed = take_item_by_name(&mut src.item, item_name)
        .ok_or_else(|| format!("item \"{item_name}\" not found in source"))?;

    // 2) add to target (in memory) — validate target folder exists BEFORE any write
    match dst_folder {
        None => dst.item.push(removed),
        Some(folder) => {
            let children = folder_children_mut(&mut dst.item, folder)
                .ok_or_else(|| format!("Target folder \"{folder}\" not found"))?;
            children.push(removed);
        }
    }

    // 3) write both atomically; roll the source file back if the target write fails
    save_collection(src_path, &src)?;
    if let Err(e) = save_collection(dst_path, &dst) {
        let _ = write_atomic(src_path, src_backup.as_bytes()); // best-effort rollback
        return Err(format!("target write failed (source restored): {e}"));
    }
    Ok(())
}

// ── root-level reorder via NNN_ numeric file prefixes ─────────────────────

/// Strip a leading `NNN_` (exactly three ASCII digits + underscore) from a filename.
pub fn strip_numeric_prefix(filename: &str) -> String {
    let bytes = filename.as_bytes();
    if bytes.len() > 4
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[3] == b'_'
    {
        filename[4..].to_string()
    } else {
        filename.to_string()
    }
}

/// `NNN_<base>` where N is the zero-padded index and base has any existing prefix stripped.
pub fn prefixed_filename(index: usize, filename: &str) -> String {
    format!("{:03}_{}", index, strip_numeric_prefix(filename))
}

/// Reorder root-level `.json` collection files by renaming ALL siblings with
/// numeric prefixes reflecting the new order. `target` is moved to `new_index`;
/// other files keep their relative order. Numeric prefixes apply ONLY at root.
/// Two-phase rename (temp then final) to avoid same-name collisions.
pub fn reorder_root_collection(
    dir: &Path,
    target_filename: &str,
    new_index: usize,
) -> Result<(), String> {
    // current order: sorted by filename (matches the extension's getItemsInFolder sort)
    let mut files: Vec<String> = fs::read_dir(dir)
        .map_err(|e| format!("read dir {}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect();
    files.sort();

    let cur = files
        .iter()
        .position(|f| f == target_filename)
        .ok_or_else(|| format!("{target_filename} not found in {}", dir.display()))?;
    if new_index >= files.len() {
        return Err(format!(
            "new index {new_index} out of range (len {})",
            files.len()
        ));
    }

    // compute the new ordering
    let target = files.remove(cur);
    files.insert(new_index.min(files.len()), target);

    // two-phase rename to dodge same-name collisions: first to a unique temp name,
    // then to the final NNN_<base> name.
    let mut temp_paths: Vec<(PathBuf, String)> = Vec::new();
    for (i, name) in files.iter().enumerate() {
        let from = dir.join(name);
        let tmp = dir.join(format!(".golden-reorder-{i}-{name}"));
        fs::rename(&from, &tmp).map_err(|e| format!("temp-rename {name}: {e}"))?;
        temp_paths.push((tmp, prefixed_filename(i, name)));
    }
    for (tmp, final_name) in temp_paths {
        let to = dir.join(&final_name);
        fs::rename(&tmp, &to).map_err(|e| format!("final-rename to {final_name}: {e}"))?;
    }
    Ok(())
}

// ── reorder within a collection ───────────────────────────────────────────

/// Move an item within the container addressed by `container_path` (empty == top
/// level) from `from` to `to`. Pure array reposition — no file renames. This is
/// the within-collection reorder (extension reorders by JSON array position).
pub fn move_item_in_container(
    items: &mut Vec<Item>,
    container_path: &[usize],
    from: usize,
    to: usize,
) -> Result<(), String> {
    if container_path.is_empty() {
        return reposition(items, from, to);
    }
    let container: &mut Vec<Item> = item_at_path_mut(items, container_path)
        .ok_or_else(|| "container not found at path".to_string())?
        .item
        .as_mut()
        .ok_or_else(|| "path is not a folder".to_string())?;
    reposition(container, from, to)
}

fn reposition(items: &mut Vec<Item>, from: usize, to: usize) -> Result<(), String> {
    let len = items.len();
    if from >= len || to >= len {
        return Err(format!("reorder index out of range (len {len})"));
    }
    if from == to {
        return Ok(());
    }
    let item = items.remove(from);
    items.insert(to, item);
    Ok(())
}

// ── duplicate operations ───────────────────────────────────────────────────

/// Compute a unique "(Copy)" / "(Copy N)" name among `siblings`. First try
/// "name (Copy)"; if taken, "name (Copy 1)", "(Copy 2)", ... — matching the
/// extension's counter (which starts the suffix number at 1).
fn unique_copy_name(base: &str, siblings: &[Item]) -> String {
    let first = format!("{base} (Copy)");
    if !siblings.iter().any(|i| i.name == first) {
        return first;
    }
    let mut counter = 1;
    loop {
        let candidate = format!("{base} (Copy {counter})");
        if !siblings.iter().any(|i| i.name == candidate) {
            return candidate;
        }
        counter += 1;
    }
}

/// Deep-copy the first item named `name` (request OR folder), append the copy
/// to its parent container with a unique "(Copy)" name. Recursive search.
pub fn duplicate_item_by_name(items: &mut Vec<Item>, name: &str) -> Result<(), String> {
    if let Some(pos) = items.iter().position(|i| i.name == name) {
        let mut clone = items[pos].clone();
        clone.name = unique_copy_name(&items[pos].name, items);
        items.push(clone);
        return Ok(());
    }
    for item in items.iter_mut() {
        if let Some(children) = item.item.as_mut() {
            if duplicate_item_by_name(children, name).is_ok() {
                return Ok(());
            }
        }
    }
    Err(format!("item \"{name}\" not found"))
}

/// Duplicate a collection FILE: copy all content, set a unique "(Copy)" name +
/// fresh _postman_id/_exporter_id, write to the unique lowercase-hyphen path.
pub fn duplicate_collection_file(path: &Path) -> Result<PathBuf, String> {
    let original = load_collection(path)?;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let base = original.info.name.clone();

    // pick a name whose slug filename does not yet exist
    let mut new_name = format!("{base} (Copy)");
    let mut counter = 1;
    let mut new_path = collection_path(dir, &new_name);
    while new_path.exists() {
        new_name = format!("{base} (Copy {counter})");
        counter += 1;
        new_path = collection_path(dir, &new_name);
    }

    let mut copy = original.clone();
    copy.info.name = new_name;
    copy.info.extra.insert(
        "_postman_id".into(),
        serde_json::Value::String(uuid::Uuid::new_v4().to_string()),
    );
    copy.info.extra.insert(
        "_exporter_id".into(),
        serde_json::Value::String(uuid::Uuid::new_v4().to_string()),
    );
    save_collection(&new_path, &copy)?;
    Ok(new_path)
}

#[cfg(test)]
mod create_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn create_collection_writes_v21_skeleton_with_fresh_ids() {
        let dir = tempdir().unwrap();
        let path = create_collection(dir.path(), "My API").unwrap();
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "my-api.json");
        let c = load_collection(&path).unwrap();
        assert_eq!(c.info.name, "My API");
        assert!(c.item.is_empty());
        // postman id + exporter id + schema present in extra
        assert!(c.info.extra.get("_postman_id").is_some());
        assert!(c.info.extra.get("_exporter_id").is_some());
        assert_eq!(
            c.info.extra.get("schema").unwrap(),
            "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
        );
    }

    #[test]
    fn create_collection_rejects_existing_name() {
        let dir = tempdir().unwrap();
        create_collection(dir.path(), "Dup").unwrap();
        let err = create_collection(dir.path(), "Dup").unwrap_err();
        assert!(err.contains("already exists"), "got: {err}");
    }

    #[test]
    fn add_request_to_collection_root() {
        let dir = tempdir().unwrap();
        let path = create_collection(dir.path(), "C").unwrap();
        let mut c = load_collection(&path).unwrap();
        add_request(&mut c.item, &[], "ping", "GET").unwrap();
        assert_eq!(c.item[0].name, "ping");
        let req = c.item[0].request.as_ref().unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.url.raw(), "https://example.com");
    }

    #[test]
    fn add_folder_then_nested_request() {
        let mut c: Collection = serde_json::from_str(r#"{"info":{"name":"C"},"item":[]}"#).unwrap();
        add_folder(&mut c.item, &[], "auth").unwrap();
        assert!(c.item[0].is_folder());
        add_request(&mut c.item, &[0], "login", "POST").unwrap();
        let nested = &c.item[0].item.as_ref().unwrap()[0];
        assert_eq!(nested.name, "login");
        assert_eq!(nested.request.as_ref().unwrap().method, "POST");
    }

    #[test]
    fn create_physical_folder_makes_dir_and_rejects_dup() {
        let dir = tempdir().unwrap();
        let made = create_physical_folder(dir.path(), "group").unwrap();
        assert!(made.is_dir());
        let err = create_physical_folder(dir.path(), "group").unwrap_err();
        assert!(err.contains("already exists"));
    }

    #[test]
    fn add_request_to_collection_root_targets_top_level_when_path_empty() {
        // empty path == push to the collection's top-level item[] (collection target)
        let mut c: Collection = serde_json::from_str(r#"{"info":{"name":"C"},"item":[]}"#).unwrap();
        add_request(&mut c.item, &[], "a", "GET").unwrap();
        add_request(&mut c.item, &[], "b", "GET").unwrap();
        assert_eq!(
            c.item.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }
}

#[cfg(test)]
mod duplicate_tests {
    use super::*;
    use tempfile::tempdir;

    fn nested() -> crate::model::Collection {
        serde_json::from_str(
            r#"{"info":{"name":"C"},"item":[
            {"name":"grp","item":[
                {"name":"login","request":{"method":"GET","url":"https://x/l"}}
            ]},
            {"name":"ping","request":{"method":"GET","url":"https://x/p"}}
        ]}"#,
        )
        .unwrap()
    }

    #[test]
    fn duplicate_request_appends_copy_with_unique_name() {
        let mut c = nested();
        duplicate_item_by_name(&mut c.item, "login").unwrap();
        let grp = c.item[0].item.as_ref().unwrap();
        assert_eq!(grp.len(), 2);
        assert_eq!(grp[1].name, "login (Copy)");
        // a second duplicate becomes (Copy 1)
        duplicate_item_by_name(&mut c.item, "login").unwrap();
        let grp = c.item[0].item.as_ref().unwrap();
        assert_eq!(grp[2].name, "login (Copy 1)");
    }

    #[test]
    fn duplicate_folder_deep_copies_children() {
        let mut c = nested();
        duplicate_item_by_name(&mut c.item, "grp").unwrap();
        // sibling of grp at root
        let copy = c.item.iter().find(|i| i.name == "grp (Copy)").unwrap();
        assert_eq!(copy.item.as_ref().unwrap()[0].name, "login");
    }

    #[test]
    fn duplicate_missing_item_errors() {
        let mut c = nested();
        assert!(duplicate_item_by_name(&mut c.item, "nope").is_err());
    }

    #[test]
    fn duplicate_collection_file_makes_copy_with_new_ids() {
        let dir = tempdir().unwrap();
        let p = create_collection(dir.path(), "Orig").unwrap();
        let id1 = load_collection(&p)
            .unwrap()
            .info
            .extra
            .get("_postman_id")
            .cloned();
        let copy_path = duplicate_collection_file(&p).unwrap();
        assert_eq!(
            copy_path.file_name().unwrap().to_str().unwrap(),
            "orig-(copy).json"
        );
        let copy = load_collection(&copy_path).unwrap();
        assert_eq!(copy.info.name, "Orig (Copy)");
        assert_ne!(
            copy.info.extra.get("_postman_id").cloned(),
            id1,
            "fresh _postman_id"
        );
        // second duplicate -> (Copy 1)
        let copy2 = duplicate_collection_file(&p).unwrap();
        assert_eq!(
            copy2.file_name().unwrap().to_str().unwrap(),
            "orig-(copy-1).json"
        );
        assert_eq!(load_collection(&copy2).unwrap().info.name, "Orig (Copy 1)");
    }
}

#[cfg(test)]
mod delete_rename_tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn nested() -> crate::model::Collection {
        serde_json::from_str(
            r#"{"info":{"name":"C"},"item":[
            {"name":"grp","item":[
                {"name":"login","request":{"method":"GET","url":"https://x/l"}}
            ]},
            {"name":"ping","request":{"method":"GET","url":"https://x/p"}}
        ]}"#,
        )
        .unwrap()
    }

    #[test]
    fn delete_item_by_name_recursive() {
        let mut c = nested();
        assert!(delete_item_by_name(&mut c.item, "login"));
        assert!(c.item[0].item.as_ref().unwrap().is_empty());
        assert!(!delete_item_by_name(&mut c.item, "missing"));
    }

    #[test]
    fn rename_item_by_name_recursive() {
        let mut c = nested();
        assert!(rename_item_by_name(&mut c.item, "login", "signin"));
        assert_eq!(c.item[0].item.as_ref().unwrap()[0].name, "signin");
        assert!(!rename_item_by_name(&mut c.item, "nope", "x"));
    }

    #[test]
    fn rename_collection_updates_name_and_renames_file() {
        let dir = tempdir().unwrap();
        let old = create_collection(dir.path(), "Old Name").unwrap();
        let new_path = rename_collection(&old, "New Name").unwrap();
        assert_eq!(
            new_path.file_name().unwrap().to_str().unwrap(),
            "new-name.json"
        );
        assert!(!old.exists(), "old file removed");
        assert_eq!(load_collection(&new_path).unwrap().info.name, "New Name");
    }

    #[test]
    fn rename_collection_same_slug_keeps_file() {
        let dir = tempdir().unwrap();
        let p = create_collection(dir.path(), "Same").unwrap();
        let np = rename_collection(&p, "Same").unwrap();
        assert_eq!(p, np);
        assert!(p.exists());
    }

    #[test]
    fn delete_collection_file_removes_it() {
        let dir = tempdir().unwrap();
        let p = create_collection(dir.path(), "Gone").unwrap();
        delete_collection_file(&p).unwrap();
        assert!(!p.exists());
    }

    #[test]
    fn delete_and_rename_physical_folder() {
        let dir = tempdir().unwrap();
        let f = create_physical_folder(dir.path(), "grp").unwrap();
        let renamed = rename_physical_folder(&f, "group").unwrap();
        assert!(renamed.is_dir());
        assert!(!f.exists());
        delete_physical_folder(&renamed).unwrap();
        assert!(!renamed.exists());
    }

    #[test]
    fn rename_physical_folder_rejects_existing_target() {
        let dir = tempdir().unwrap();
        let a = create_physical_folder(dir.path(), "a").unwrap();
        create_physical_folder(dir.path(), "b").unwrap();
        let err = rename_physical_folder(&a, "b").unwrap_err();
        assert!(err.contains("already exists"));
    }

    // suppress unused import
    #[allow(unused_imports)]
    use fs as _fs;
}

#[cfg(test)]
mod load_save_tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn load_then_save_is_byte_identical_to_extension_format() {
        // The repo sample was written by the extension (JSON.stringify(x, null, 2)).
        let src = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../collections/sample-collection.json"
        );
        let original = fs::read_to_string(src).unwrap();

        let dir = tempdir().unwrap();
        let path = dir.path().join("sample.json");
        fs::write(&path, &original).unwrap();

        let coll = load_collection(&path).unwrap();
        save_collection(&path, &coll).unwrap();

        let written = fs::read_to_string(&path).unwrap();
        // Value-equal (no fields gained/lost) ...
        let a: serde_json::Value = serde_json::from_str(&original).unwrap();
        let b: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(a, b, "no fields gained or lost");
        // ... and the output uses 2-space indentation (serde_json::to_string_pretty default).
        // We check that any indented line uses spaces in multiples of 2, not tabs or 4-space.
        for line in written.lines() {
            let leading = line.len() - line.trim_start().len();
            assert_eq!(
                leading % 2,
                0,
                "line must be indented by a multiple of 2 spaces: {:?}",
                line
            );
            assert!(
                !line.starts_with('\t'),
                "line must not use tabs: {:?}",
                line
            );
        }
        // Confirm the second-level indent is 2 spaces (first nested key).
        assert!(
            written.contains("  \"info\""),
            "second level should be 2-space indented"
        );
    }

    #[test]
    fn edit_then_save_then_reload_persists_change() {
        let src = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../collections/sample-collection.json"
        );
        let original = std::fs::read_to_string(src).unwrap();
        let dir = tempdir().unwrap();
        let path = dir.path().join("sample.json");
        std::fs::write(&path, &original).unwrap();

        let mut coll = load_collection(&path).unwrap();
        // edit the first request in the first folder
        super::set_method(&mut coll.item, &[0, 0], "PATCH").unwrap();
        save_collection(&path, &coll).unwrap();

        let reloaded = load_collection(&path).unwrap();
        let it = super::item_at_path_mut(&mut reloaded.clone().item, &[0, 0])
            .unwrap()
            .clone();
        assert_eq!(it.request.unwrap().method, "PATCH");
        // everything else still parses
        assert!(!reloaded.item.is_empty());
    }
}

#[cfg(test)]
mod slug_tests {
    use super::*;
    #[test]
    fn slugify_matches_extension_rule() {
        assert_eq!(slugify("My Collection"), "my-collection");
        assert_eq!(slugify("A   B"), "a-b"); // \s+ collapses to one '-'
        assert_eq!(slugify("Tabs\tHere"), "tabs-here");
        assert_eq!(slugify("Keep_underscore"), "keep_underscore"); // not \s
        assert_eq!(collection_filename("My API"), "my-api.json");
    }
}

#[cfg(test)]
mod edit_request_tests {
    use super::*;
    use crate::model::Collection;

    fn coll() -> Collection {
        let json = r#"{
          "info": { "name": "C" },
          "item": [
            { "name": "grp", "item": [
              { "name": "login", "request": {
                  "method": "GET", "url": "https://x/login", "header": []
              }, "event": [] }
            ]}
          ]
        }"#;
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn item_at_path_returns_nested_request() {
        let mut c = coll();
        let it = item_at_path_mut(&mut c.item, &[0, 0]).unwrap();
        assert_eq!(it.name, "login");
        assert!(it.is_request());
        assert!(item_at_path_mut(&mut c.item, &[0, 5]).is_none());
    }

    #[test]
    fn set_method_and_url_mutate_in_place() {
        let mut c = coll();
        set_method(&mut c.item, &[0, 0], "POST").unwrap();
        set_url(&mut c.item, &[0, 0], "https://x/v2/login").unwrap();
        let req = item_at_path_mut(&mut c.item, &[0, 0])
            .unwrap()
            .request
            .as_ref()
            .unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.url.raw(), "https://x/v2/login");
    }

    #[test]
    fn set_url_on_parsed_url_keeps_object_shape_but_updates_raw() {
        let json = r#"{"info":{"name":"C"},"item":[{"name":"r","request":{
            "method":"GET","url":{"raw":"https://x/a","host":["x"],"path":["a"]}}}]}"#;
        let mut c: Collection = serde_json::from_str(json).unwrap();
        set_url(&mut c.item, &[0], "https://x/b").unwrap();
        let url = &item_at_path_mut(&mut c.item, &[0])
            .unwrap()
            .request
            .as_ref()
            .unwrap()
            .url;
        assert_eq!(url.raw(), "https://x/b");
        // still serialized as an object (host/path preserved)
        let v = serde_json::to_value(url).unwrap();
        assert!(v.is_object(), "Parsed url stays an object");
        assert_eq!(v["host"], serde_json::json!(["x"]));
    }

    #[test]
    fn set_script_creates_then_replaces_test_event() {
        let mut c = coll();
        set_script(
            &mut c.item,
            &[0, 0],
            "test",
            &["pm.test('ok', ()=>{});".into()],
        )
        .unwrap();
        let it = item_at_path_mut(&mut c.item, &[0, 0]).unwrap();
        let ev = it.event.iter().find(|e| e.listen == "test").unwrap();
        assert_eq!(ev.script.exec, vec!["pm.test('ok', ()=>{});".to_string()]);
        // replace, not append a second test event
        set_script(&mut c.item, &[0, 0], "test", &["changed".into()]).unwrap();
        let it = item_at_path_mut(&mut c.item, &[0, 0]).unwrap();
        assert_eq!(it.event.iter().filter(|e| e.listen == "test").count(), 1);
        assert_eq!(it.event[0].script.exec, vec!["changed".to_string()]);
    }
}

#[cfg(test)]
mod edit_headers_tests {
    use super::*;
    use crate::model::{Collection, Header};

    fn coll() -> Collection {
        let json = r#"{"info":{"name":"C"},"item":[{"name":"r","request":{
            "method":"GET","url":"https://x/a?q=1&page=2","header":[
              {"key":"Accept","value":"*/*"}]}}]}"#;
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn set_headers_replaces_whole_table() {
        let mut c = coll();
        set_headers(
            &mut c.item,
            &[0],
            vec![
                Header {
                    key: "Authorization".into(),
                    value: "Bearer {{tok}}".into(),
                    disabled: false,
                    extra: serde_json::Map::new(),
                },
                Header {
                    key: "X-Off".into(),
                    value: "y".into(),
                    disabled: true,
                    extra: serde_json::Map::new(),
                },
            ],
        )
        .unwrap();
        let req = c.item[0].request.as_ref().unwrap();
        assert_eq!(req.header.len(), 2);
        assert_eq!(req.header[0].key, "Authorization");
        assert!(req.header[1].disabled);
    }

    #[test]
    fn set_headers_from_json_parses_array_of_objects() {
        let mut c = coll();
        let json = r#"[{"key":"A","value":"1"},{"key":"B","value":"2","disabled":true}]"#;
        set_headers_from_json(&mut c.item, &[0], json).unwrap();
        let req = c.item[0].request.as_ref().unwrap();
        assert_eq!(req.header.len(), 2);
        assert_eq!(req.header[1].key, "B");
        assert!(req.header[1].disabled);
    }

    #[test]
    fn set_headers_from_invalid_json_errors() {
        let mut c = coll();
        let err = set_headers_from_json(&mut c.item, &[0], "not json").unwrap_err();
        assert!(err.contains("parse"), "got: {err}");
    }

    #[test]
    fn query_param_helpers_read_and_write_raw_url() {
        let params = parse_query_params("https://x/a?q=1&page=2");
        assert_eq!(
            params,
            vec![
                ("q".to_string(), "1".to_string()),
                ("page".to_string(), "2".to_string())
            ]
        );
        let url = build_url_with_params(
            "https://x/a?old=z",
            &[
                ("q".to_string(), "hello world".to_string()),
                ("page".to_string(), "2".to_string()),
            ],
        );
        assert_eq!(url, "https://x/a?q=hello%20world&page=2");
    }

    #[test]
    fn build_url_with_no_params_strips_question_mark() {
        assert_eq!(
            build_url_with_params("https://x/a?old=z", &[]),
            "https://x/a"
        );
    }
}

#[cfg(test)]
mod cross_move_tests {
    use super::*;
    use tempfile::tempdir;

    fn write(dir: &std::path::Path, slug: &str, name: &str, body: &str) -> PathBuf {
        let p = dir.join(format!("{slug}.json"));
        let json = format!(r#"{{"info":{{"name":"{name}"}},"item":[{body}]}}"#);
        fs::write(&p, json).unwrap();
        p
    }

    #[test]
    fn move_request_to_target_collection_root() {
        let dir = tempdir().unwrap();
        let src = write(
            dir.path(),
            "src",
            "Src",
            r#"{"name":"login","request":{"method":"GET","url":"u"}}"#,
        );
        let dst = write(dir.path(), "dst", "Dst", "");

        move_item_across_collections(&src, "login", &dst, None).unwrap();

        let s = load_collection(&src).unwrap();
        let d = load_collection(&dst).unwrap();
        assert!(
            s.item.iter().all(|i| i.name != "login"),
            "removed from source"
        );
        assert_eq!(
            d.item.iter().filter(|i| i.name == "login").count(),
            1,
            "added to target"
        );
    }

    #[test]
    fn move_request_into_target_folder() {
        let dir = tempdir().unwrap();
        let src = write(
            dir.path(),
            "src",
            "Src",
            r#"{"name":"login","request":{"method":"GET","url":"u"}}"#,
        );
        let dst = write(dir.path(), "dst", "Dst", r#"{"name":"auth","item":[]}"#);

        move_item_across_collections(&src, "login", &dst, Some("auth")).unwrap();

        let d = load_collection(&dst).unwrap();
        let folder = d.item.iter().find(|i| i.name == "auth").unwrap();
        assert_eq!(folder.item.as_ref().unwrap()[0].name, "login");
    }

    #[test]
    fn move_missing_source_item_errors_and_does_not_touch_target() {
        let dir = tempdir().unwrap();
        let src = write(dir.path(), "src", "Src", "");
        let dst = write(dir.path(), "dst", "Dst", "");
        let dst_before = fs::read_to_string(&dst).unwrap();
        let err = move_item_across_collections(&src, "nope", &dst, None).unwrap_err();
        assert!(err.contains("not found"));
        assert_eq!(
            fs::read_to_string(&dst).unwrap(),
            dst_before,
            "target untouched on failure"
        );
    }

    #[test]
    fn move_to_missing_target_folder_errors_and_keeps_source_intact() {
        let dir = tempdir().unwrap();
        let src = write(
            dir.path(),
            "src",
            "Src",
            r#"{"name":"login","request":{"method":"GET","url":"u"}}"#,
        );
        let dst = write(dir.path(), "dst", "Dst", "");
        let src_before = fs::read_to_string(&src).unwrap();
        let err = move_item_across_collections(&src, "login", &dst, Some("ghost")).unwrap_err();
        assert!(err.contains("Target folder"));
        // rollback: source still has the item (we did not write either file)
        assert_eq!(
            fs::read_to_string(&src).unwrap(),
            src_before,
            "source rolled back"
        );
    }
}

#[cfg(test)]
mod root_reorder_tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn touch(dir: &std::path::Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, "{}").unwrap();
        p
    }

    fn entries(dir: &std::path::Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn prefixed_name_pads_index_to_three_digits() {
        assert_eq!(prefixed_filename(0, "users.json"), "000_users.json");
        assert_eq!(prefixed_filename(5, "000_users.json"), "005_users.json"); // strips old prefix
        assert_eq!(prefixed_filename(12, "orders.json"), "012_orders.json");
    }

    #[test]
    fn strip_numeric_prefix_removes_nnn_underscore() {
        assert_eq!(strip_numeric_prefix("003_users.json"), "users.json");
        assert_eq!(strip_numeric_prefix("users.json"), "users.json");
        assert_eq!(strip_numeric_prefix("12_x.json"), "12_x.json"); // not 3 digits -> unchanged
    }

    #[test]
    fn reorder_root_collections_renames_all_siblings_with_prefixes() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "alpha.json");
        touch(dir.path(), "beta.json");
        touch(dir.path(), "gamma.json");
        // current order is sorted-by-filename: alpha, beta, gamma. Move gamma(2) -> 0.
        reorder_root_collection(dir.path(), "gamma.json", 0).unwrap();
        // result order by prefix: gamma, alpha, beta
        assert_eq!(
            entries(dir.path()),
            vec![
                "000_gamma.json".to_string(),
                "001_alpha.json".to_string(),
                "002_beta.json".to_string(),
            ]
        );
    }
}

#[cfg(test)]
mod reorder_items_tests {
    use super::*;

    fn coll() -> crate::model::Collection {
        serde_json::from_str(
            r#"{"info":{"name":"C"},"item":[
            {"name":"a","request":{"method":"GET","url":"https://x/a"}},
            {"name":"b","request":{"method":"GET","url":"https://x/b"}},
            {"name":"c","request":{"method":"GET","url":"https://x/c"}}
        ]}"#,
        )
        .unwrap()
    }

    fn names(c: &crate::model::Collection) -> Vec<&str> {
        c.item.iter().map(|i| i.name.as_str()).collect()
    }

    #[test]
    fn move_item_within_container_up_and_down() {
        let mut c = coll();
        // move index 2 ("c") to index 0
        move_item_in_container(&mut c.item, &[], 2, 0).unwrap();
        assert_eq!(names(&c), vec!["c", "a", "b"]);
        // move index 0 ("c") to index 2 (end)
        move_item_in_container(&mut c.item, &[], 0, 2).unwrap();
        assert_eq!(names(&c), vec!["a", "b", "c"]);
    }

    #[test]
    fn move_item_within_nested_folder() {
        let mut c: crate::model::Collection = serde_json::from_str(
            r#"{"info":{"name":"C"},"item":[
            {"name":"grp","item":[
                {"name":"x","request":{"method":"GET","url":"u"}},
                {"name":"y","request":{"method":"GET","url":"u"}}
            ]}
        ]}"#,
        )
        .unwrap();
        move_item_in_container(&mut c.item, &[0], 1, 0).unwrap();
        let grp = c.item[0].item.as_ref().unwrap();
        assert_eq!(
            grp.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            vec!["y", "x"]
        );
    }

    #[test]
    fn move_item_out_of_range_errors() {
        let mut c = coll();
        assert!(move_item_in_container(&mut c.item, &[], 9, 0).is_err());
        assert!(move_item_in_container(&mut c.item, &[], 0, 9).is_err());
    }
}

#[cfg(test)]
mod edit_body_var_tests {
    use super::*;
    use crate::model::Collection;

    fn coll() -> Collection {
        let json = r#"{"info":{"name":"C"},
          "variable":[{"key":"base","value":"https://x","type":"string"}],
          "item":[{"name":"r","request":{"method":"POST","url":"https://x/a","header":[]}}]}"#;
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn set_raw_body_creates_body_with_mode_raw() {
        let mut c = coll();
        set_raw_body(&mut c.item, &[0], "{\"a\":1}").unwrap();
        let body = c.item[0].request.as_ref().unwrap().body.as_ref().unwrap();
        assert_eq!(body.mode, "raw");
        // raw is stored as Value::String (matches Postman on-disk format)
        assert_eq!(
            body.raw.as_ref().and_then(|v| v.as_str()),
            Some("{\"a\":1}")
        );
        assert!(body.graphql.is_none());
    }

    #[test]
    fn clear_body_removes_it() {
        let mut c = coll();
        set_raw_body(&mut c.item, &[0], "x").unwrap();
        clear_body(&mut c.item, &[0]).unwrap();
        assert!(c.item[0].request.as_ref().unwrap().body.is_none());
    }

    #[test]
    fn set_graphql_body_stores_query_and_variables() {
        let mut c = coll();
        set_graphql_body(&mut c.item, &[0], "{ me { id } }", Some("{\"x\":1}")).unwrap();
        let body = c.item[0].request.as_ref().unwrap().body.as_ref().unwrap();
        assert_eq!(body.mode, "graphql");
        let g = body.graphql.as_ref().unwrap();
        assert_eq!(g.query, "{ me { id } }");
        assert_eq!(g.variables, Some(serde_json::json!({"x": 1})));
        assert!(body.raw.is_none());
    }

    #[test]
    fn set_graphql_body_with_no_variables_keeps_them_absent() {
        let mut c = coll();
        set_graphql_body(&mut c.item, &[0], "{ a }", None).unwrap();
        let g = c.item[0]
            .request
            .as_ref()
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .graphql
            .as_ref()
            .unwrap();
        assert!(
            g.variables.is_none(),
            "absent variables must serialize away"
        );
    }

    #[test]
    fn collection_variable_crud() {
        let mut c = coll();
        set_variable(&mut c, "token", "abc"); // add
        assert_eq!(
            c.variable.iter().find(|v| v.key == "token").unwrap().value,
            "abc"
        );
        set_variable(&mut c, "base", "https://y"); // edit existing, keep extra "type"
        let base = c.variable.iter().find(|v| v.key == "base").unwrap();
        assert_eq!(base.value, "https://y");
        assert_eq!(base.extra.get("type"), Some(&serde_json::json!("string")));
        assert!(delete_variable(&mut c, "token")); // delete
        assert!(c.variable.iter().all(|v| v.key != "token"));
        assert!(!delete_variable(&mut c, "missing"));
    }
}
