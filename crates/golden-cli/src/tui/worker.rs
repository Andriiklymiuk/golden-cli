//! Background workers: run HTTP sends / collection runs off the UI thread and
//! stream results back over channels. A shared AtomicBool is the cancel token.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

use golden_core::env::VarScopes;
use golden_core::http::HttpConfig;
use golden_core::model::{Collection, Item};
use golden_core::result::RunResult;
use golden_core::runner::{run_single, run_with_progress, SingleOutcome};

/// Default per-request timeout in the TUI so a hung server cannot wedge `sending…`.
pub const TUI_TIMEOUT_MS: u64 = 30_000;

/// The HttpConfig the TUI uses for sends and runs (timeout-bounded).
pub fn tui_http_config() -> HttpConfig {
    HttpConfig {
        insecure: false,
        timeout_ms: Some(TUI_TIMEOUT_MS),
    }
}

/// Messages the send worker streams back to the UI loop.
#[derive(Debug)]
pub enum SendMsg {
    Done(Box<SingleOutcome>),
    Cancelled,
}

/// A live send: its receiving end + a cancel flag + the join handle.
pub struct SendHandle {
    pub rx: Receiver<SendMsg>,
    pub cancel: Arc<AtomicBool>,
    pub join: Option<JoinHandle<()>>,
}

/// Spawn a thread that runs the leaf request at `target_path` within `coll`
/// through the script pipeline (so its pre/test scripts run), then posts the
/// outcome. If `cancel` is set before the result is delivered, posts
/// `Cancelled` instead.
pub fn spawn_send(
    coll: Collection,
    target_path: Vec<usize>,
    scopes: VarScopes,
    cfg: HttpConfig,
    workspace: std::path::PathBuf,
) -> SendHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_thread = cancel.clone();
    let (tx, rx): (Sender<SendMsg>, Receiver<SendMsg>) = std::sync::mpsc::channel();
    let join = std::thread::spawn(move || {
        let outcome = run_single(&coll, &target_path, &scopes, &cfg);
        if cancel_thread.load(Ordering::SeqCst) {
            let _ = tx.send(SendMsg::Cancelled);
        } else {
            // Record the send into request history (no-op when disabled). Build the
            // entry from the resolved request so it stays replayable; unmasked so
            // replay can resend the real headers.
            if let Some(req) = request_at(&coll, &target_path) {
                let entry = golden_core::history::HistoryEntry::from_sent(
                    req,
                    scopes.as_map(),
                    outcome.response.as_ref().map(|r| r.status),
                    outcome.result.time_ms,
                );
                let _ = golden_core::history::append(&workspace, entry, false);
            }
            let _ = tx.send(SendMsg::Done(Box::new(outcome)));
        }
    });
    SendHandle {
        rx,
        cancel,
        join: Some(join),
    }
}

/// Resolve the leaf `Request` at `target_path` (indices into `coll.item`).
/// Mirrors the index walk the runner's `collect_units` produces, so the request
/// recorded into history is exactly the one that was sent.
fn request_at<'a>(
    coll: &'a Collection,
    target_path: &[usize],
) -> Option<&'a golden_core::model::Request> {
    let (first, rest) = target_path.split_first()?;
    let mut item = coll.item.get(*first)?;
    for &idx in rest {
        item = item.item.as_deref()?.get(idx)?;
    }
    item.request.as_ref()
}

/// Find the first leaf request named `name` anywhere in the collection tree.
/// Used to recover header/body templates for a run's `RequestResult` (which only
/// carries the resolved url/method/status/time).
fn find_request_by_name<'a>(
    coll: &'a Collection,
    name: &str,
) -> Option<&'a golden_core::model::Request> {
    fn walk<'a>(items: &'a [Item], name: &str) -> Option<&'a golden_core::model::Request> {
        for item in items {
            if item.item.is_none() && item.name == name {
                if let Some(req) = item.request.as_ref() {
                    return Some(req);
                }
            }
            if let Some(children) = item.item.as_deref() {
                if let Some(found) = walk(children, name) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(&coll.item, name)
}

/// Append every request a run executed into request history. The authoritative
/// url/method/status/time come from the `RequestResult`; header/body templates are
/// recovered from the matching request and resolved against `scopes`. Skips
/// requests that errored before sending (no status and no time).
fn record_run_history(workspace: &Path, coll: &Collection, result: &RunResult, scopes: &VarScopes) {
    for cr in &result.collections {
        for iter in &cr.iterations {
            for rr in &iter.requests {
                let Some(req) = find_request_by_name(coll, &rr.name) else {
                    continue;
                };
                let mut entry = golden_core::history::HistoryEntry::from_sent(
                    req,
                    scopes.as_map(),
                    rr.status,
                    rr.time_ms,
                );
                // Trust the run's resolved url/method over the template re-resolution
                // (the live var map may have changed mid-run via pre-request scripts).
                entry.method = rr.method.clone();
                entry.url = rr.url.clone();
                let _ = golden_core::history::append(workspace, entry, false);
            }
        }
    }
}

impl SendHandle {
    /// Signal cancellation; the worker reports Cancelled instead of Done.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Run worker
// ---------------------------------------------------------------------------

/// Messages the run worker sends back to the UI loop.
#[derive(Debug)]
pub enum RunMsg {
    /// Cumulative completed-request count so far (lets the gauge advance live).
    Progress(usize),
    Done(RunResult),
}

/// A live collection run: receiver + cooperative cancel flag + join handle.
pub struct RunHandle {
    pub rx: Receiver<RunMsg>,
    pub cancel: Arc<AtomicBool>,
    pub join: Option<JoinHandle<()>>,
}

impl RunHandle {
    /// Signal cancellation; the run stops between requests.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// Count leaf request items in a collection (for the progress total).
pub fn count_requests(coll: &Collection) -> usize {
    fn walk(items: &[Item]) -> usize {
        items
            .iter()
            .map(|i| {
                if let Some(children) = &i.item {
                    walk(children)
                } else if i.request.is_some() {
                    1
                } else {
                    0
                }
            })
            .sum()
    }
    walk(&coll.item)
}

/// Spawn a thread that runs each collection `iterations` times, streaming the
/// cumulative completed-request count back as `RunMsg::Progress` after every
/// request (so the gauge advances live) and posting the merged `RunResult` as a
/// final `RunMsg::Done`.
///
/// `run_with_progress` reports a count that resets at the start of each
/// collection, so we carry a `base` offset of the requests finished by prior
/// collections to make the streamed count globally cumulative.
pub fn spawn_run(
    colls: Vec<Collection>,
    scopes: VarScopes,
    iterations: u32,
    cfg: HttpConfig,
    workspace: std::path::PathBuf,
) -> RunHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_thread = cancel.clone();
    let (tx, rx): (Sender<RunMsg>, Receiver<RunMsg>) = std::sync::mpsc::channel();
    let join = std::thread::spawn(move || {
        let mut merged = RunResult::default();
        let mut base = 0usize;
        for coll in &colls {
            let tx_progress = tx.clone();
            let mut on_progress = |done: usize| {
                let _ = tx_progress.send(RunMsg::Progress(base + done));
            };
            let r = run_with_progress(
                coll,
                &scopes,
                iterations,
                &cfg,
                Some(cancel_thread.clone()),
                Some(&mut on_progress),
            );
            // Record each executed request into history (no-op when disabled). The
            // RequestResult carries the resolved url/method/status/time; headers and
            // body are recovered from the matching request in the collection so the
            // entry stays replayable. Unmasked so replay resends the real headers.
            record_run_history(&workspace, coll, &r, &scopes);
            // Advance the offset by every request this collection ran (across
            // iterations) so the next collection's counts stack on top.
            base += r.totals.requests as usize;
            merged.collections.extend(r.collections);
            merged.totals.requests += r.totals.requests;
            merged.totals.failed_requests += r.totals.failed_requests;
            merged.totals.assertions += r.totals.assertions;
            merged.totals.failed_assertions += r.totals.failed_assertions;
            merged.totals.total_ms += r.totals.total_ms;
            // Honour cancellation between collections: a cancelled run-all must not
            // build an engine / collect_units for the remaining collections.
            if cancel_thread.load(Ordering::SeqCst) {
                break;
            }
        }
        let _ = tx.send(RunMsg::Done(merged));
    });
    RunHandle {
        rx,
        cancel,
        join: Some(join),
    }
}

// ---------------------------------------------------------------------------
// Download / import helpers (used by the `w` and `i` prompts)
// ---------------------------------------------------------------------------

/// Write a response to `target`. When `req` is `Some`, perform a *fresh* streamed
/// download via `golden_core::http::download_to_file` (reusing the same path as
/// `golden send --output`). When `req` is `None`, write the already-fetched
/// `cached` bytes directly. Returns the number of bytes written.
///
/// This keeps the TUI's save-to-disk gesture aligned with the headless download
/// path instead of re-implementing it.
pub fn download_response(
    req: Option<&golden_core::model::Request>,
    cached: Option<&[u8]>,
    scopes: &VarScopes,
    cfg: &HttpConfig,
    target: &Path,
) -> Result<u64, String> {
    match req {
        Some(request) => {
            let info =
                golden_core::http::download_to_file(request, scopes.as_map(), cfg, target, None)?;
            Ok(info.bytes_written)
        }
        None => {
            let bytes = cached.ok_or("no response to save")?;
            std::fs::write(target, bytes).map_err(|e| e.to_string())?;
            Ok(bytes.len() as u64)
        }
    }
}

/// Resolve the effective `--from` source kind the import prompt should use, given
/// the user-typed flag (`auto|postman|raw|folder|openapi|curl`) and the source.
/// Mirrors the auto-detection in `commands::import::resolve_inputs` so the prompt
/// and the headless command behave identically. Returns Err for an unknown flag.
pub fn parse_import_from(from: &str) -> Result<&'static str, String> {
    match from {
        "auto" => Ok("auto"),
        "postman" => Ok("postman"),
        "raw" => Ok("raw"),
        "folder" => Ok("folder"),
        "openapi" => Ok("openapi"),
        "curl" => Ok("curl"),
        other => Err(format!(
            "unknown --from '{other}' (use auto|postman|raw|folder|openapi|curl)"
        )),
    }
}

// ---------------------------------------------------------------------------
// Reload helper
// ---------------------------------------------------------------------------

/// Apply a fresh batch of loaded collections to the app, preserving selection.
/// Called from the run loop when the watcher signals a file change.
pub fn apply_reload(
    app: &mut crate::tui::app::App,
    collections: Vec<crate::tui::loader::LoadedCollection>,
    errors: &[String],
) {
    app.collections = collections;
    app.rebuild_rows();
    app.status = if errors.is_empty() {
        "reloaded".into()
    } else {
        format!("reloaded ({} parse error(s))", errors.len())
    };
}

// ---------------------------------------------------------------------------
// File watcher (hot-reload)
// ---------------------------------------------------------------------------

/// A live file watcher: keeps the watcher alive + a receiver of change pings.
pub struct WatchHandle {
    pub rx: Receiver<()>,
    /// Watcher is kept alive by the struct; dropped with it.
    _watcher: notify::RecommendedWatcher,
}

/// Watch `dir` recursively; send `()` on the channel for any change event.
/// Returns an error if the watcher cannot be created or the path cannot be
/// watched (e.g. the directory does not exist).
pub fn spawn_watch(dir: &Path) -> notify::Result<WatchHandle> {
    use notify::{RecursiveMode, Watcher};

    let (tx, rx): (Sender<()>, Receiver<()>) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = tx.send(());
        }
    })?;
    watcher.watch(dir, RecursiveMode::Recursive)?;
    Ok(WatchHandle {
        rx,
        _watcher: watcher,
    })
}

#[cfg(test)]
mod watch_tests {
    use super::*;
    use crate::tui::app::App;
    use crate::tui::loader::LoadedCollection;
    use golden_core::env::VarScopes;
    use golden_core::model::Collection;
    use std::fs;
    use std::time::Duration;
    use tempfile::tempdir;

    fn make_app(json: &str) -> App {
        let lc = LoadedCollection {
            path: "/tmp/x.json".into(),
            collection: serde_json::from_str::<Collection>(json).unwrap(),
        };
        App::new("/tmp".into(), vec![lc], VarScopes::default())
    }

    const J: &str = r#"{
      "info": { "name": "Sample" },
      "item": [
        { "name": "ping", "request": { "method": "GET", "url": "http://x/ping" } }
      ]
    }"#;

    #[test]
    fn apply_reload_replaces_collections_and_rebuilds_rows() {
        let mut app = make_app(J);
        let initial_rows = app.rows.len();

        // New collection with an extra request.
        let new_json = r#"{
          "info": { "name": "Reloaded" },
          "item": [
            { "name": "alpha", "request": { "method": "GET", "url": "http://x/a" } },
            { "name": "beta",  "request": { "method": "GET", "url": "http://x/b" } }
          ]
        }"#;
        let new_lc = LoadedCollection {
            path: "/tmp/y.json".into(),
            collection: serde_json::from_str::<Collection>(new_json).unwrap(),
        };
        apply_reload(&mut app, vec![new_lc], &[]);
        assert_eq!(app.status, "reloaded");
        assert!(
            app.rows.len() > initial_rows,
            "more rows after reload with more requests"
        );
        assert!(
            app.rows.iter().any(|r| r.name == "alpha"),
            "new request visible after reload"
        );
    }

    #[test]
    fn apply_reload_surfaces_parse_errors_in_status() {
        let mut app = make_app(J);
        apply_reload(
            &mut app,
            vec![],
            &["bad.json: unexpected token".to_string()],
        );
        assert!(
            app.status.contains("reloaded") && app.status.contains("1 parse error"),
            "status should mention parse error count: {}",
            app.status
        );
    }

    #[test]
    fn watcher_pings_on_file_write() {
        let dir = tempdir().unwrap();
        let handle = spawn_watch(dir.path()).unwrap();
        fs::write(dir.path().join("a.json"), "{}").unwrap();
        // Allow the OS to deliver the event (generous timeout for slow CI filesystems).
        let msg = handle.rx.recv_timeout(Duration::from_secs(10));
        assert!(msg.is_ok(), "expected a change ping within 10s");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golden_core::env::VarScopes;
    use golden_core::model::Collection;
    use httpmock::prelude::*;

    fn one_item_coll(url: &str) -> Collection {
        let json = format!(
            r#"{{"info":{{"name":"C"}},"item":[{{"name":"a","request":{{"method":"GET","url":"{url}"}}}}]}}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn tui_http_config_is_timeout_bounded() {
        // The central fix for the "sending… forever" bug: the TUI must send with a
        // bounded timeout (not HttpConfig::default()'s timeout_ms = None).
        assert_eq!(TUI_TIMEOUT_MS, 30_000);
        assert_eq!(tui_http_config().timeout_ms, Some(TUI_TIMEOUT_MS));
        // Guard against an accidental None/0 (the regression that reintroduces the bug).
        assert!(tui_http_config().timeout_ms.is_some_and(|ms| ms > 0));
    }

    #[test]
    fn spawn_send_delivers_response() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/ok");
            then.status(200).body("hi");
        });
        let ws = tempfile::tempdir().unwrap();
        let handle = spawn_send(
            one_item_coll(&format!("{}/ok", server.base_url())),
            vec![0],
            VarScopes::default(),
            HttpConfig::default(),
            ws.path().to_path_buf(),
        );
        match handle.rx.recv().unwrap() {
            SendMsg::Done(out) => assert_eq!(out.response.unwrap().status, 200),
            other => panic!("expected Done, got {other:?}"),
        }
        // Finding #3: the send worker must record the request into history so the
        // History overlay + Enter-to-replay have data. Drain the worker first.
        handle.join.unwrap().join().unwrap();
        let entries = golden_core::history::read_all(ws.path()).unwrap();
        assert_eq!(entries.len(), 1, "send should record exactly one entry");
        assert_eq!(entries[0].method, "GET");
        assert!(entries[0].url.ends_with("/ok"));
        assert_eq!(entries[0].status, Some(200));
    }

    #[test]
    fn spawn_send_records_resolved_replayable_entry() {
        // The recorded entry must carry the *resolved* URL/headers (unmasked) so the
        // replay path (which resends stored headers with an empty var map) works.
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/secure")
                .header("authorization", "Bearer tok123");
            then.status(200).body("ok");
        });
        // url uses {{base}} and the header uses {{tok}}; both must be resolved
        // before recording so the stored entry is replayable with no var map.
        let coll_json = r#"{"info":{"name":"C"},"item":[{"name":"a","request":{"method":"GET","url":"{{base}}/secure","header":[{"key":"Authorization","value":"Bearer {{tok}}"}]}}]}"#;
        let coll: Collection = serde_json::from_str(coll_json).unwrap();
        let mut scopes = VarScopes::default();
        scopes.set("base".into(), server.base_url());
        scopes.set("tok".into(), "tok123".into());
        let ws = tempfile::tempdir().unwrap();
        let handle = spawn_send(
            coll,
            vec![0],
            scopes,
            HttpConfig::default(),
            ws.path().to_path_buf(),
        );
        let _ = handle.rx.recv().unwrap();
        handle.join.unwrap().join().unwrap();
        let entries = golden_core::history::read_all(ws.path()).unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.url, format!("{}/secure", server.base_url()));
        // Authorization recorded unmasked so replay can resend it verbatim.
        let auth = e
            .request_headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            .expect("authorization header recorded");
        assert_eq!(auth.1, "Bearer tok123");
    }

    #[test]
    fn cancel_before_completion_reports_cancelled() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/slow");
            then.status(200)
                .delay(std::time::Duration::from_millis(300))
                .body("late");
        });
        let ws = tempfile::tempdir().unwrap();
        let handle = spawn_send(
            one_item_coll(&format!("{}/slow", server.base_url())),
            vec![0],
            VarScopes::default(),
            HttpConfig::default(),
            ws.path().to_path_buf(),
        );
        handle.cancel();
        let msg = handle.rx.recv().unwrap();
        assert!(matches!(msg, SendMsg::Cancelled));
    }

    #[test]
    fn spawn_run_executes_collection_and_returns_result() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.path("/a");
            then.status(200);
        });
        let json = format!(
            r#"{{"info":{{"name":"C"}},"item":[{{"name":"a","request":{{"method":"GET","url":"{}/a"}}}}]}}"#,
            server.base_url()
        );
        let coll: Collection = serde_json::from_str(&json).unwrap();
        assert_eq!(count_requests(&coll), 1);
        let ws = tempfile::tempdir().unwrap();
        let handle = spawn_run(
            vec![coll],
            VarScopes::default(),
            1,
            HttpConfig::default(),
            ws.path().to_path_buf(),
        );
        // The worker now streams Progress before Done; the final message is Done.
        let result = loop {
            match handle.rx.recv().unwrap() {
                RunMsg::Progress(_) => continue,
                RunMsg::Done(result) => break result,
            }
        };
        assert_eq!(result.totals.requests, 1);
    }

    #[test]
    fn spawn_run_streams_progress_then_done() {
        // A two-request collection should emit Progress messages (cumulative 1, 2)
        // ahead of the final Done — that streaming is what advances the gauge live.
        let server = MockServer::start();
        server.mock(|when, then| {
            when.path("/a");
            then.status(200);
        });
        server.mock(|when, then| {
            when.path("/b");
            then.status(200);
        });
        let json = format!(
            r#"{{"info":{{"name":"C"}},"item":[
                {{"name":"a","request":{{"method":"GET","url":"{base}/a"}}}},
                {{"name":"b","request":{{"method":"GET","url":"{base}/b"}}}}
            ]}}"#,
            base = server.base_url()
        );
        let coll: Collection = serde_json::from_str(&json).unwrap();
        let ws = tempfile::tempdir().unwrap();
        let handle = spawn_run(
            vec![coll],
            VarScopes::default(),
            1,
            HttpConfig::default(),
            ws.path().to_path_buf(),
        );

        let mut progress = Vec::new();
        let result = loop {
            match handle.rx.recv().unwrap() {
                RunMsg::Progress(done) => progress.push(done),
                RunMsg::Done(result) => break result,
            }
        };
        assert_eq!(
            progress,
            vec![1, 2],
            "expected cumulative progress 1 then 2, got {progress:?}"
        );
        assert_eq!(result.totals.requests, 2);
    }

    #[test]
    fn run_all_cancel_skips_remaining_collections() {
        // A cancelled run-all must stop after the in-flight collection: subsequent
        // collections should never be run (no engine build / collect_units), so they
        // never appear in the merged result. The first collection's request is slow
        // enough that the immediate cancel lands before its loop reaches the next
        // step, and the run-all loop then breaks before touching collection B.
        let server = MockServer::start();
        server.mock(|when, then| {
            when.path("/a");
            then.status(200)
                .delay(std::time::Duration::from_millis(300))
                .body("ok");
        });
        let hit_b = server.mock(|when, then| {
            when.path("/b");
            then.status(200);
        });
        let coll_a = one_item_coll(&format!("{}/a", server.base_url()));
        let mut coll_b = one_item_coll(&format!("{}/b", server.base_url()));
        coll_b.info.name = "B".into();
        let ws = tempfile::tempdir().unwrap();
        let handle = spawn_run(
            vec![coll_a, coll_b],
            VarScopes::default(),
            1,
            HttpConfig::default(),
            ws.path().to_path_buf(),
        );
        handle.cancel();
        let result = loop {
            match handle.rx.recv().unwrap() {
                RunMsg::Progress(_) => continue,
                RunMsg::Done(result) => break result,
            }
        };
        // Collection B must have been skipped entirely: never run, never recorded.
        hit_b.assert_hits(0);
        assert!(
            !result.collections.iter().any(|c| c.name == "B"),
            "cancelled run-all should not run collection B at all, got {:?}",
            result
                .collections
                .iter()
                .map(|c| &c.name)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn download_response_writes_cached_bytes_when_no_request() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let target = dir.path().join("saved.json");
        let n = download_response(
            None,
            Some(b"hello world"),
            &VarScopes::default(),
            &HttpConfig::default(),
            &target,
        )
        .unwrap();
        assert_eq!(n, 11);
        assert_eq!(std::fs::read(&target).unwrap(), b"hello world");
    }

    #[test]
    fn download_response_streams_fresh_download_when_request_given() {
        use tempfile::tempdir;
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/file");
            then.status(200).body("fresh-bytes");
        });
        let req: golden_core::model::Request = serde_json::from_value(serde_json::json!({
            "method": "GET",
            "url": format!("{}/file", server.base_url()),
        }))
        .unwrap();
        let dir = tempdir().unwrap();
        let target = dir.path().join("dl.bin");
        // cached bytes are ignored because a request is supplied (fresh stream wins).
        let n = download_response(
            Some(&req),
            Some(b"stale"),
            &VarScopes::default(),
            &HttpConfig::default(),
            &target,
        )
        .unwrap();
        assert_eq!(n, "fresh-bytes".len() as u64);
        assert_eq!(std::fs::read(&target).unwrap(), b"fresh-bytes");
    }

    #[test]
    fn download_response_errors_when_nothing_to_write() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let target = dir.path().join("none.bin");
        let err = download_response(
            None,
            None,
            &VarScopes::default(),
            &HttpConfig::default(),
            &target,
        )
        .unwrap_err();
        assert!(err.contains("no response"), "got: {err}");
        assert!(!target.exists(), "nothing should be written");
    }

    #[test]
    fn parse_import_from_accepts_known_kinds_and_rejects_others() {
        for k in ["auto", "postman", "raw", "folder", "openapi", "curl"] {
            assert_eq!(parse_import_from(k).unwrap(), k);
        }
        assert!(parse_import_from("nope").is_err());
    }

    #[test]
    fn spawn_run_progress_is_globally_cumulative_across_collections() {
        // Two separate one-request collections: the second collection's progress
        // must stack on top of the first (cumulative 1 then 2), not reset to 1.
        let server = MockServer::start();
        server.mock(|when, then| {
            when.path("/a");
            then.status(200);
        });
        server.mock(|when, then| {
            when.path("/b");
            then.status(200);
        });
        let c1: Collection = serde_json::from_str(&format!(
            r#"{{"info":{{"name":"A"}},"item":[{{"name":"a","request":{{"method":"GET","url":"{}/a"}}}}]}}"#,
            server.base_url()
        ))
        .unwrap();
        let c2: Collection = serde_json::from_str(&format!(
            r#"{{"info":{{"name":"B"}},"item":[{{"name":"b","request":{{"method":"GET","url":"{}/b"}}}}]}}"#,
            server.base_url()
        ))
        .unwrap();
        let ws = tempfile::tempdir().unwrap();
        let handle = spawn_run(
            vec![c1, c2],
            VarScopes::default(),
            1,
            HttpConfig::default(),
            ws.path().to_path_buf(),
        );

        let mut progress = Vec::new();
        while let RunMsg::Progress(done) = handle.rx.recv().unwrap() {
            progress.push(done);
        }
        assert_eq!(
            progress,
            vec![1, 2],
            "second collection's progress must be cumulative, got {progress:?}"
        );
    }
}
