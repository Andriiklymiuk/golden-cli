//! Background workers: run HTTP sends / collection runs off the UI thread and
//! stream results back over channels. A shared AtomicBool is the cancel token.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

use golden_core::env::VarScopes;
use golden_core::http::{send, HttpConfig, HttpResponse};
use golden_core::model::{Collection, Item, Request};
use golden_core::result::RunResult;
use golden_core::runner::run as core_run;

/// Messages the send worker streams back to the UI loop.
#[derive(Debug)]
pub enum SendMsg {
    Done(Result<HttpResponse, String>),
    Cancelled,
}

/// A live send: its receiving end + a cancel flag + the join handle.
pub struct SendHandle {
    pub rx: Receiver<SendMsg>,
    pub cancel: Arc<AtomicBool>,
    pub join: Option<JoinHandle<()>>,
}

/// Spawn a thread that sends `req` with `vars` + `cfg`, then posts the result.
/// If `cancel` is set before the result is delivered, posts `Cancelled` instead.
pub fn spawn_send(req: Request, vars: HashMap<String, String>, cfg: HttpConfig) -> SendHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_thread = cancel.clone();
    let (tx, rx): (Sender<SendMsg>, Receiver<SendMsg>) = std::sync::mpsc::channel();
    let join = std::thread::spawn(move || {
        let result = send(&req, &vars, &cfg);
        if cancel_thread.load(Ordering::SeqCst) {
            let _ = tx.send(SendMsg::Cancelled);
        } else {
            let _ = tx.send(SendMsg::Done(result));
        }
    });
    SendHandle {
        rx,
        cancel,
        join: Some(join),
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
    Done(RunResult),
}

/// A live collection run: receiver + join handle.
/// (Cancellation is not supported for a single blocking call; parks naturally.)
pub struct RunHandle {
    pub rx: Receiver<RunMsg>,
    pub join: Option<JoinHandle<()>>,
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

/// Spawn a thread that runs each collection `iterations` times and posts the
/// merged `RunResult` back as a single `RunMsg::Done`.
///
/// Because `golden_core::runner::run` is a synchronous / batch call (no
/// per-request streaming), we report `Done` once all collections finish.
/// The overlay shows a "running…" spinner while in-flight and the full result
/// once the message arrives.
pub fn spawn_run(
    colls: Vec<Collection>,
    scopes: VarScopes,
    iterations: u32,
    cfg: HttpConfig,
) -> RunHandle {
    let (tx, rx): (Sender<RunMsg>, Receiver<RunMsg>) = std::sync::mpsc::channel();
    let join = std::thread::spawn(move || {
        let mut merged = RunResult::default();
        for coll in &colls {
            let r = core_run(coll, &scopes, iterations, &cfg);
            merged.collections.extend(r.collections);
            merged.totals.requests += r.totals.requests;
            merged.totals.failed_requests += r.totals.failed_requests;
            merged.totals.assertions += r.totals.assertions;
            merged.totals.failed_assertions += r.totals.failed_assertions;
            merged.totals.total_ms += r.totals.total_ms;
        }
        let _ = tx.send(RunMsg::Done(merged));
    });
    RunHandle {
        rx,
        join: Some(join),
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
    use golden_core::model::{Collection, Url};
    use httpmock::prelude::*;

    fn req(url: &str) -> Request {
        Request {
            method: "GET".into(),
            url: Url::Raw(url.into()),
            header: vec![],
            body: None,
        }
    }

    #[test]
    fn spawn_send_delivers_response() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/ok");
            then.status(200).body("hi");
        });
        let handle = spawn_send(
            req(&format!("{}/ok", server.base_url())),
            HashMap::new(),
            HttpConfig::default(),
        );
        let msg = handle.rx.recv().unwrap();
        match msg {
            SendMsg::Done(Ok(resp)) => assert_eq!(resp.status, 200),
            other => panic!("expected Done(Ok), got {other:?}"),
        }
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
        let handle = spawn_send(
            req(&format!("{}/slow", server.base_url())),
            HashMap::new(),
            HttpConfig::default(),
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
        let handle = spawn_run(vec![coll], VarScopes::default(), 1, HttpConfig::default());
        let RunMsg::Done(result) = handle.rx.recv().unwrap();
        assert_eq!(result.totals.requests, 1);
    }
}
