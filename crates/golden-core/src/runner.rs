//! Orchestration: depth-first item traversal x iterations.

use std::collections::HashMap;

use crate::env::VarScopes;
use crate::http::{send, HttpConfig};
use crate::model::{Collection, Item, Request};
use crate::result::*;
use crate::sandbox::{JsEngine, RequestView, ResponseView, RquickJsEngine};

/// Join a script's `exec` lines for an event listener, if present.
fn script_for(item: &Item, listen: &str) -> Option<String> {
    item.event
        .iter()
        .find(|e| e.listen == listen)
        .map(|e| e.script.exec.join("\n"))
}

/// One runnable request plus the ordered script chains that wrap it: `pre` runs
/// collection → folders(outer→inner) → request before the send; `post` runs
/// request → folders(inner→outer) → collection after (Newman bubbling order).
struct RunUnit<'a> {
    item: &'a Item,
    request: &'a Request,
    pre: Vec<String>,
    post: Vec<String>,
}

/// Read a collection-level script. Postman stores the top-level `event` array in
/// the flattened `extra`, so parse it from there (no model/serialization change).
fn collection_script(coll: &Collection, listen: &str) -> Option<String> {
    let events: Vec<crate::model::Event> =
        serde_json::from_value(coll.extra.get("event")?.clone()).ok()?;
    events
        .iter()
        .find(|e| e.listen == listen)
        .map(|e| e.script.exec.join("\n"))
}

/// Depth-first: build a `RunUnit` per leaf request, threading folder prerequest/
/// test scripts down the tree. `folder_pre`/`folder_test` accumulate outer→inner;
/// folder tests are reversed at the leaf so they bubble inner→outer.
fn collect_units<'a>(
    items: &'a [Item],
    folder_pre: &[String],
    folder_test: &[String],
    out: &mut Vec<RunUnit<'a>>,
) {
    for item in items {
        if let Some(children) = &item.item {
            let mut fp = folder_pre.to_vec();
            if let Some(s) = script_for(item, "prerequest") {
                fp.push(s);
            }
            let mut ft = folder_test.to_vec();
            if let Some(s) = script_for(item, "test") {
                ft.push(s);
            }
            collect_units(children, &fp, &ft, out);
        } else if let Some(request) = &item.request {
            let mut pre = folder_pre.to_vec();
            if let Some(s) = script_for(item, "prerequest") {
                pre.push(s);
            }
            let mut post = Vec::new();
            if let Some(s) = script_for(item, "test") {
                post.push(s);
            }
            post.extend(folder_test.iter().rev().cloned());
            out.push(RunUnit {
                item,
                request,
                pre,
                post,
            });
        }
    }
}

/// Run a collection (collection-run precedence: all pm.* scopes fold into one
/// active variable map; later writes win — Postman-runtime style).
pub fn run(coll: &Collection, scopes: &VarScopes, iterations: u32, cfg: &HttpConfig) -> RunResult {
    run_with_bail(coll, scopes, iterations, cfg, false)
}

/// As `run`, but `bail` stops the run after the first failed assertion.
pub fn run_with_bail(
    coll: &Collection,
    scopes: &VarScopes,
    iterations: u32,
    cfg: &HttpConfig,
    bail: bool,
) -> RunResult {
    run_with_options(coll, scopes, iterations, cfg, bail, &[])
}

/// As `run_with_bail`, plus a data set for data-driven runs. When `data` is
/// non-empty it drives the iteration count (one row per iteration) and each row
/// overlays the variable set (data wins over env/collection) and feeds
/// `pm.iterationData` for that iteration.
pub fn run_with_options(
    coll: &Collection,
    scopes: &VarScopes,
    iterations: u32,
    cfg: &HttpConfig,
    bail: bool,
    data: &[HashMap<String, String>],
) -> RunResult {
    let engine = RquickJsEngine::new();
    let mut collection_result = CollectionResult {
        name: coll.info.name.clone(),
        iterations: Vec::new(),
        stats: Vec::new(),
    };
    let mut totals = Totals::default();
    // request name -> list of times across iterations (for stats)
    let mut times: Vec<(String, Vec<u128>)> = Vec::new();

    // Collection-level scripts + the per-request pre/post chains are stable across
    // iterations, so resolve them once.
    let coll_pre = collection_script(coll, "prerequest");
    let coll_test = collection_script(coll, "test");
    let mut units: Vec<RunUnit> = Vec::new();
    collect_units(&coll.item, &[], &[], &mut units);

    // Data rows drive the iteration count when present (Postman behaviour).
    let iters = if data.is_empty() {
        iterations.max(1)
    } else {
        data.len() as u32
    };

    for i in 0..iters {
        let mut iter = Iteration {
            index: i + 1,
            requests: Vec::new(),
        };
        let mut live: HashMap<String, String> = scopes.as_map().clone();
        let empty_row = HashMap::new();
        let data_row = data.get(i as usize).unwrap_or(&empty_row);
        for (k, v) in data_row {
            live.insert(k.clone(), v.clone());
        }

        // Cursor (not a plain for-loop) so setNextRequest can jump/stop. The step
        // cap is a runaway guard against a script that loops setNextRequest forever.
        let mut idx = 0usize;
        let mut steps = 0usize;
        let max_steps = units.len().saturating_mul(4) + 8;
        while idx < units.len() {
            steps += 1;
            if steps > max_steps {
                break;
            }
            let unit = &units[idx];
            let mut pre = Vec::new();
            if let Some(s) = &coll_pre {
                pre.push(s.clone());
            }
            pre.extend(unit.pre.iter().cloned());
            let mut post = unit.post.clone();
            if let Some(s) = &coll_test {
                post.push(s.clone());
            }
            let (rr, next) = run_one(
                &engine,
                unit.item,
                unit.request,
                &pre,
                &post,
                i,
                iters,
                data_row,
                &mut live,
                cfg,
            );
            totals.requests += 1;
            if rr.error.is_some() || rr.status.map(|s| s >= 400).unwrap_or(true) {
                totals.failed_requests += 1;
            }
            totals.assertions += rr.assertions.len();
            totals.failed_assertions += rr.assertions.iter().filter(|a| !a.passed).count();
            totals.total_ms += rr.time_ms;
            push_time(&mut times, &unit.item.name, rr.time_ms);
            let bailed = bail && rr.assertions.iter().any(|a| !a.passed);
            iter.requests.push(rr);
            if bailed {
                collection_result.iterations.push(iter);
                return RunResult {
                    collections: vec![collection_result],
                    totals,
                };
            }
            match next {
                Some(None) => break,
                Some(Some(target)) => {
                    idx = units
                        .iter()
                        .position(|u| u.item.name == target)
                        .unwrap_or(idx + 1);
                }
                None => idx += 1,
            }
        }
        collection_result.iterations.push(iter);
    }

    if iters > 1 {
        collection_result.stats = times
            .into_iter()
            .map(|(name, ts)| {
                let sum: u128 = ts.iter().sum();
                let avg = sum as f64 / ts.len() as f64;
                RequestStats {
                    name,
                    avg_ms: avg,
                    min_ms: *ts.iter().min().unwrap_or(&0),
                    max_ms: *ts.iter().max().unwrap_or(&0),
                }
            })
            .collect();
    }

    RunResult {
        collections: vec![collection_result],
        totals,
    }
}

#[allow(clippy::too_many_arguments)]
fn run_one(
    engine: &RquickJsEngine,
    item: &Item,
    request: &Request,
    pre: &[String],
    post: &[String],
    iteration: u32,
    iteration_count: u32,
    data_row: &HashMap<String, String>,
    live: &mut HashMap<String, String>,
    cfg: &HttpConfig,
) -> (RequestResult, Option<Option<String>>) {
    let name = item.name.clone();
    let mut next_request = None;

    let mut pre_assertions = Vec::new();
    for src in pre {
        let mut view = RequestView::from_request(request, live);
        view.name = name.clone();
        view.iteration = iteration;
        view.iteration_count = iteration_count;
        view.data = data_row.clone();
        let outcome = engine.run_script(src, live, &view, None, cfg);
        apply_mutations(live, &outcome.mutations);
        pre_assertions.extend(outcome.assertions);
        if outcome.next_request.is_some() {
            next_request = outcome.next_request;
        }
        if let Some(err) = &outcome.error {
            let rr = RequestResult {
                name,
                method: request.method.clone(),
                url: crate::subst::substitute(request.url.raw(), live),
                status: None,
                time_ms: 0,
                assertions: pre_assertions,
                error: Some(format!("pre-request script error: {err}")),
            };
            return (rr, next_request);
        }
    }

    let url = crate::subst::substitute(request.url.raw(), live);
    let resp = match send(request, live, cfg) {
        Ok(r) => r,
        Err(e) => {
            let rr = RequestResult {
                name,
                method: request.method.clone(),
                url,
                status: None,
                time_ms: 0,
                assertions: Vec::new(),
                error: Some(e),
            };
            return (rr, next_request);
        }
    };

    let mut assertions = Vec::new();
    let mut script_error = None;
    for src in post {
        let mut view = RequestView::from_request(request, live);
        view.name = name.clone();
        view.iteration = iteration;
        view.iteration_count = iteration_count;
        view.data = data_row.clone();
        let response = ResponseView {
            code: resp.status,
            status_text: String::new(),
            headers: resp.headers.clone(),
            body: String::from_utf8_lossy(&resp.body).to_string(),
            time_ms: resp.time_ms,
        };
        let outcome = engine.run_script(src, live, &view, Some(&response), cfg);
        apply_mutations(live, &outcome.mutations);
        assertions.extend(outcome.assertions);
        if outcome.next_request.is_some() {
            next_request = outcome.next_request;
        }
        if script_error.is_none() {
            script_error = outcome.error.map(|e| format!("test script error: {e}"));
        }
    }

    let rr = RequestResult {
        name,
        method: request.method.clone(),
        url,
        status: Some(resp.status),
        time_ms: resp.time_ms,
        assertions,
        error: script_error,
    };
    (rr, next_request)
}

/// Apply scope mutations to the live variable set (chaining). All scopes fold
/// into the same active map for substitution; see precedence note (Task 11).
fn apply_mutations(live: &mut HashMap<String, String>, ops: &[crate::sandbox::ScopeOp]) {
    use crate::sandbox::ScopeOp;
    for op in ops {
        match op {
            ScopeOp::Set { key, value, .. } => {
                live.insert(key.clone(), value.clone());
            }
            ScopeOp::Unset { key, .. } => {
                live.remove(key);
            }
        }
    }
}

fn push_time(times: &mut Vec<(String, Vec<u128>)>, name: &str, ms: u128) {
    if let Some(entry) = times.iter_mut().find(|(n, _)| n == name) {
        entry.1.push(ms);
    } else {
        times.push((name.to_string(), vec![ms]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::VarScopes;
    use crate::http::HttpConfig;
    use crate::model::Collection;
    use httpmock::prelude::*;

    #[test]
    fn runs_scripts_and_populates_assertions() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/users");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"[{"id":1}]"#);
        });
        let json = format!(
            r#"{{
              "info": {{ "name": "C" }},
              "item": [{{
                "name": "list",
                "request": {{ "method": "GET", "url": "{base}/users" }},
                "event": [{{
                  "listen": "test",
                  "script": {{ "exec": [
                    "pm.test('status 200', function () {{ pm.response.to.have.status(200); }});",
                    "pm.test('is array', function () {{ pm.expect(pm.response.json()).to.be.an('array'); }});",
                    "pm.test('FAILS', function () {{ pm.expect(1).to.equal(2); }});"
                  ] }}
                }}]
              }}]
            }}"#,
            base = server.base_url()
        );
        let coll: Collection = serde_json::from_str(&json).unwrap();
        let result = run(&coll, &VarScopes::default(), 1, &HttpConfig::default());

        let rr = &result.collections[0].iterations[0].requests[0];
        assert_eq!(rr.assertions.len(), 3);
        assert_eq!(rr.assertions[0].name, "status 200");
        assert!(rr.assertions[0].passed);
        assert!(!rr.assertions[2].passed);
        assert_eq!(result.totals.assertions, 3);
        assert_eq!(result.totals.failed_assertions, 1);
    }

    #[test]
    fn runs_collection_and_folder_scripts_around_request() {
        // collection-prereq sets a var, folder-prereq sets another, folder-test
        // asserts BOTH are visible (proving pre order ran), collection-test runs
        // too. The request itself has no scripts — before ancestor support these
        // assertions never ran (collect_items skipped folder/collection items).
        let server = MockServer::start();
        server.mock(|when, then| {
            when.path("/echo");
            then.status(200).body("ok");
        });
        let json = format!(
            r#"{{
              "info": {{ "name": "C" }},
              "event": [{{ "listen": "prerequest", "script": {{ "exec": [
                "pm.environment.set('fromColl','C');" ] }} }},
              {{ "listen": "test", "script": {{ "exec": [
                "pm.test('collection test ran', function () {{ pm.expect(true).to.be.true; }});" ] }} }}],
              "item": [{{
                "name": "Folder",
                "event": [{{ "listen": "prerequest", "script": {{ "exec": [
                  "pm.environment.set('fromFolder','F');" ] }} }},
                {{ "listen": "test", "script": {{ "exec": [
                  "pm.test('folder saw coll+folder vars', function () {{ pm.expect(pm.environment.get('fromColl')).to.equal('C'); pm.expect(pm.environment.get('fromFolder')).to.equal('F'); }});" ] }} }}],
                "item": [{{
                  "name": "req",
                  "request": {{ "method": "GET", "url": "{base}/echo" }}
                }}]
              }}]
            }}"#,
            base = server.base_url()
        );
        let coll: Collection = serde_json::from_str(&json).unwrap();
        let result = run(&coll, &VarScopes::default(), 1, &HttpConfig::default());
        let rr = &result.collections[0].iterations[0].requests[0];
        let names: Vec<&str> = rr.assertions.iter().map(|a| a.name.as_str()).collect();
        assert!(
            names.contains(&"folder saw coll+folder vars"),
            "folder test did not run: {names:?}"
        );
        assert!(
            names.contains(&"collection test ran"),
            "collection test did not run: {names:?}"
        );
        assert!(
            rr.assertions.iter().all(|a| a.passed),
            "an ancestor assertion failed"
        );
    }

    #[test]
    fn set_next_request_jumps_skipping_intermediate() {
        let server = MockServer::start();
        for p in ["/a", "/b", "/c"] {
            server.mock(|when, then| {
                when.path(p);
                then.status(200);
            });
        }
        let json = format!(
            r#"{{"info":{{"name":"C"}},"item":[
              {{"name":"A","request":{{"method":"GET","url":"{b}/a"}},
               "event":[{{"listen":"test","script":{{"exec":["pm.execution.setNextRequest('C');"]}}}}]}},
              {{"name":"B","request":{{"method":"GET","url":"{b}/b"}}}},
              {{"name":"C","request":{{"method":"GET","url":"{b}/c"}}}}
            ]}}"#,
            b = server.base_url()
        );
        let coll: Collection = serde_json::from_str(&json).unwrap();
        let result = run(&coll, &VarScopes::default(), 1, &HttpConfig::default());
        let names: Vec<&str> = result.collections[0].iterations[0]
            .requests
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(names, vec!["A", "C"], "setNextRequest('C') should skip B");
    }

    #[test]
    fn data_drives_iterations_and_overlays_vars() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.path("/e");
            then.status(200);
        });
        let json = format!(
            r#"{{"info":{{"name":"C"}},"variable":[{{"key":"x","value":"base"}}],"item":[
              {{"name":"R","request":{{"method":"GET","url":"{b}/e"}},
               "event":[{{"listen":"test","script":{{"exec":[
                 "pm.test('data feeds iterationData + overlays vars', function () {{ pm.expect(pm.variables.get('x')).to.equal(pm.iterationData.get('x')); pm.expect(pm.iterationData.get('x')).to.not.equal('base'); }});"
               ]}}}}]}}
            ]}}"#,
            b = server.base_url()
        );
        let coll: Collection = serde_json::from_str(&json).unwrap();
        let data = vec![
            std::collections::HashMap::from([("x".to_string(), "row0".to_string())]),
            std::collections::HashMap::from([("x".to_string(), "row1".to_string())]),
        ];
        let result = run_with_options(
            &coll,
            &VarScopes::default(),
            1,
            &HttpConfig::default(),
            false,
            &data,
        );
        assert_eq!(
            result.collections[0].iterations.len(),
            2,
            "data rows drive iteration count"
        );
        assert_eq!(result.totals.requests, 2);
        assert_eq!(
            result.totals.failed_assertions, 0,
            "data should overlay vars + feed iterationData"
        );
    }

    #[test]
    fn set_next_request_null_stops_run() {
        let server = MockServer::start();
        for p in ["/a", "/b"] {
            server.mock(|when, then| {
                when.path(p);
                then.status(200);
            });
        }
        let json = format!(
            r#"{{"info":{{"name":"C"}},"item":[
              {{"name":"A","request":{{"method":"GET","url":"{b}/a"}},
               "event":[{{"listen":"test","script":{{"exec":["postman.setNextRequest(null);"]}}}}]}},
              {{"name":"B","request":{{"method":"GET","url":"{b}/b"}}}}
            ]}}"#,
            b = server.base_url()
        );
        let coll: Collection = serde_json::from_str(&json).unwrap();
        let result = run(&coll, &VarScopes::default(), 1, &HttpConfig::default());
        assert_eq!(
            result.totals.requests, 1,
            "setNextRequest(null) should stop the run after A"
        );
    }

    #[test]
    fn bail_stops_after_first_failed_assertion() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.path("/a");
            then.status(200);
        });
        let b = server.mock(|when, then| {
            when.path("/b");
            then.status(200);
        });
        let json = format!(
            r#"{{"info":{{"name":"C"}},"item":[
              {{"name":"a","request":{{"method":"GET","url":"{base}/a"}},
               "event":[{{"listen":"test","script":{{"exec":[
                 "pm.test('FAIL', function () {{ pm.expect(1).to.equal(2); }});"]}}}}]}},
              {{"name":"b","request":{{"method":"GET","url":"{base}/b"}}}}
            ]}}"#,
            base = server.base_url()
        );
        let coll: Collection = serde_json::from_str(&json).unwrap();
        let result = run_with_bail(
            &coll,
            &VarScopes::default(),
            1,
            &HttpConfig::default(),
            true,
        );
        // request "a" ran (and failed an assertion); "b" was NOT sent due to bail
        b.assert_hits(0);
        assert_eq!(result.totals.requests, 1);
        assert_eq!(result.totals.failed_assertions, 1);
    }

    #[test]
    fn no_bail_runs_all_requests() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.path("/a");
            then.status(200);
        });
        let b = server.mock(|when, then| {
            when.path("/b");
            then.status(200);
        });
        let json = format!(
            r#"{{"info":{{"name":"C"}},"item":[
              {{"name":"a","request":{{"method":"GET","url":"{base}/a"}},
               "event":[{{"listen":"test","script":{{"exec":[
                 "pm.test('FAIL', function () {{ pm.expect(1).to.equal(2); }});"]}}}}]}},
              {{"name":"b","request":{{"method":"GET","url":"{base}/b"}}}}
            ]}}"#,
            base = server.base_url()
        );
        let coll: Collection = serde_json::from_str(&json).unwrap();
        let result = run(&coll, &VarScopes::default(), 1, &HttpConfig::default());
        b.assert_hits(1);
        assert_eq!(result.totals.requests, 2);
    }

    #[test]
    fn collection_run_later_writes_win_over_initial_scope() {
        let server = MockServer::start();
        let hit = server.mock(|when, then| {
            when.path("/v/override");
            then.status(200);
        });
        let json = format!(
            r#"{{"info":{{"name":"C"}},"item":[
              {{"name":"setter","request":{{"method":"GET","url":"{base}/v/override"}},
               "event":[{{"listen":"prerequest","script":{{"exec":[
                 "pm.collectionVariables.set('slot', 'override');"]}}}}]}}
            ]}}"#,
            base = server.base_url()
        );
        let coll: Collection = serde_json::from_str(&json).unwrap();
        let mut scopes = VarScopes::default();
        scopes.set("slot".into(), "initial".into()); // initial value...
                                                     // URL uses {{slot}}; pre-request override must win (collection-run precedence)
        let coll = patch_url(coll, &format!("{}/v/{{{{slot}}}}", server.base_url()));
        let _ = run(&coll, &scopes, 1, &HttpConfig::default());
        hit.assert(); // hit /v/override, proving the pre-request write won
    }

    // small helper to set the first request's URL
    fn patch_url(mut coll: Collection, url: &str) -> Collection {
        if let Some(req) = coll.item[0].request.as_mut() {
            req.url = crate::model::Url::Raw(url.to_string());
        }
        coll
    }

    #[test]
    fn chains_variable_across_requests() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/first");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"id":42}"#);
        });
        let hit = server.mock(|when, then| {
            when.method(GET).path("/second/42");
            then.status(200);
        });
        let json = format!(
            r#"{{
              "info": {{ "name": "Chain" }},
              "item": [
                {{
                  "name": "first",
                  "request": {{ "method": "GET", "url": "{base}/first" }},
                  "event": [{{ "listen": "test", "script": {{ "exec": [
                    "pm.collectionVariables.set('uid', pm.response.json().id.toString());"
                  ] }} }}]
                }},
                {{
                  "name": "second",
                  "request": {{ "method": "GET", "url": "{base}/second/{{{{uid}}}}" }},
                  "event": [{{ "listen": "test", "script": {{ "exec": [
                    "pm.test('chained', function () {{ pm.expect(pm.request.url).to.include('/second/42'); }});"
                  ] }} }}]
                }}
              ]
            }}"#,
            base = server.base_url()
        );
        let coll: Collection = serde_json::from_str(&json).unwrap();
        let result = run(&coll, &VarScopes::default(), 1, &HttpConfig::default());
        hit.assert(); // /second/42 was actually hit -> {{uid}} resolved from request 1
        let second = &result.collections[0].iterations[0].requests[1];
        assert!(
            second.assertions.iter().all(|a| a.passed),
            "{:?}",
            second.assertions
        );
    }

    #[test]
    fn each_iteration_restarts_from_base_scopes() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.path("/x");
            then.status(200).body(r#"{"n":1}"#);
        });
        let json = format!(
            r#"{{"info":{{"name":"C"}},"item":[{{
                "name":"x","request":{{"method":"GET","url":"{base}/x"}},
                "event":[{{"listen":"test","script":{{"exec":[
                  "pm.test('base intact', function () {{ pm.expect(pm.collectionVariables.get('seed')).to.equal('s'); }});",
                  "pm.collectionVariables.set('seed', 'mutated');"
                ]}}}}]
            }}]}}"#,
            base = server.base_url()
        );
        let coll: Collection = serde_json::from_str(&json).unwrap();
        let mut scopes = VarScopes::default();
        scopes.set("seed".into(), "s".into());
        let result = run(&coll, &scopes, 2, &HttpConfig::default());
        // both iterations must see seed='s' at start (write-back does not leak across iterations)
        for it in &result.collections[0].iterations {
            assert!(
                it.requests[0].assertions[0].passed,
                "iter {} leaked",
                it.index
            );
        }
    }

    #[test]
    fn runs_collection_depth_first_and_counts() {
        let server = MockServer::start();
        let a = server.mock(|when, then| {
            when.path("/a");
            then.status(200);
        });
        let b = server.mock(|when, then| {
            when.path("/b");
            then.status(500);
        });

        let json = format!(
            r#"{{
              "info": {{ "name": "C" }},
              "item": [
                {{ "name": "first", "request": {{ "method": "GET", "url": "{base}/a" }} }},
                {{ "name": "grp", "item": [
                  {{ "name": "second", "request": {{ "method": "GET", "url": "{base}/b" }} }}
                ]}}
              ]
            }}"#,
            base = server.base_url()
        );
        let coll: Collection = serde_json::from_str(&json).unwrap();
        let scopes = VarScopes::default();

        let result = run(&coll, &scopes, 1, &HttpConfig::default());
        a.assert();
        b.assert();
        assert_eq!(result.totals.requests, 2);
        assert_eq!(result.totals.failed_requests, 1); // /b returned 500
        let reqs = &result.collections[0].iterations[0].requests;
        assert_eq!(reqs[0].name, "first");
        assert_eq!(reqs[0].status, Some(200));
        assert_eq!(reqs[1].name, "second");
        assert_eq!(reqs[1].status, Some(500));
    }

    #[test]
    fn computes_stats_when_iterations_gt_1() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.path("/x");
            then.status(200);
        });
        let json = format!(
            r#"{{"info":{{"name":"C"}},"item":[{{"name":"x","request":{{"method":"GET","url":"{}/x"}}}}]}}"#,
            server.base_url()
        );
        let coll: Collection = serde_json::from_str(&json).unwrap();
        let result = run(&coll, &VarScopes::default(), 3, &HttpConfig::default());
        assert_eq!(result.collections[0].iterations.len(), 3);
        assert_eq!(result.collections[0].stats.len(), 1);
        assert_eq!(result.collections[0].stats[0].name, "x");
    }
}
