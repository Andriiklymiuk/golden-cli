//! rquickjs implementation of the JsEngine trait.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rquickjs::{Context, Ctx, Function, Object, Runtime, Value};

use crate::http::HttpConfig;
use crate::sandbox::{JsEngine, RequestView, ResponseView, Scope, ScopeOp, ScriptOutcome};

const JS_PRELUDE: &str = concat!(
    // chai's UMD wrapper probes for `global`/`self`/`window`; QuickJS only has
    // `globalThis`. Alias them first so the UMD factory attaches `globalThis.chai`.
    "var global=globalThis;var self=globalThis;var window=globalThis;\n",
    include_str!("chai.js"),
    "\n",
    include_str!("prelude.js"),
);

/// Shared, mutable state collected from a single script run.
#[derive(Default)]
struct Collector {
    assertions: Vec<crate::result::Assertion>,
    mutations: Vec<ScopeOp>,
    logs: Vec<String>,
    next_request: Option<Option<String>>,
}

pub struct RquickJsEngine;

impl RquickJsEngine {
    pub fn new() -> Self {
        RquickJsEngine
    }
}

impl Default for RquickJsEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl JsEngine for RquickJsEngine {
    fn run_script(
        &self,
        script: &str,
        vars: &HashMap<String, String>,
        request: &RequestView,
        response: Option<&ResponseView>,
        cfg: &HttpConfig,
    ) -> ScriptOutcome {
        let collector = Rc::new(RefCell::new(Collector::default()));
        let rt = match Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                return ScriptOutcome {
                    error: Some(e.to_string()),
                    ..Default::default()
                }
            }
        };
        rt.set_max_stack_size(4 * 1024 * 1024); // 4 MB — needed for chai in Task 7; harmless now
        let ctx = match Context::full(&rt) {
            Ok(c) => c,
            Err(e) => {
                return ScriptOutcome {
                    error: Some(e.to_string()),
                    ..Default::default()
                }
            }
        };

        let mut error: Option<String> = None;

        ctx.with(|ctx| {
            if let Err(e) = install_console(&ctx, collector.clone()) {
                error = Some(format!("console install failed: {e}"));
                return;
            }
            // pm.* + require are installed by later tasks; pass through the inputs now.
            if let Err(e) = install_pm(&ctx, collector.clone(), vars, request, response, cfg) {
                error = Some(format!("pm install failed: {e}"));
                return;
            }
            // Evaluate the script. A throw / syntax error becomes the outcome error.
            if let Err(e) = ctx.eval::<(), _>(script) {
                error = Some(js_error_string(&ctx, e));
            }
        });

        // The collector Rc may still be referenced by JS function closures held
        // in the context. We extract the data by borrowing rather than unwrapping.
        let c = collector.borrow();
        ScriptOutcome {
            assertions: c.assertions.clone(),
            mutations: c.mutations.clone(),
            logs: c.logs.clone(),
            error,
            next_request: c.next_request.clone(),
        }
    }
}

/// Turn an rquickjs error into a readable string, pulling the JS exception message
/// out of the context when the error is an Exception.
fn js_error_string(ctx: &Ctx<'_>, e: rquickjs::Error) -> String {
    if e.is_exception() {
        let exc = ctx.catch();
        if let Some(s) = exc.as_string() {
            if let Ok(msg) = s.to_string() {
                return msg;
            }
        }
        if let Some(obj) = exc.as_object() {
            if let Ok(msg) = obj.get::<_, String>("message") {
                return msg;
            }
        }
    }
    e.to_string()
}

/// console.log/info/warn/error -> push formatted line into the collector.
///
/// Strategy: register a native Rust function `__golden_log` that takes a single
/// pre-formatted string, then wire `console.log` etc. as JS shims that call
/// `Array.from(arguments).map(String).join(" ")` before forwarding to it.
/// This avoids the rquickjs variadic-closure lifetime puzzle entirely.
fn install_console(
    ctx: &Ctx<'_>,
    collector: Rc<RefCell<Collector>>,
) -> Result<(), rquickjs::Error> {
    // Native sink: __golden_log(parts: Rest<String>)
    // rquickjs coerces each JS arg to String automatically (Rest<T> where T: FromJs).
    let col = collector;
    let native_log = Function::new(
        ctx.clone(),
        move |parts: rquickjs::function::Rest<String>| {
            let line = parts.0.join(" ");
            col.borrow_mut().logs.push(line);
        },
    )?;
    ctx.globals().set("__golden_log", native_log)?;

    // JS shim: install console.{log,info,warn,error}.
    // Each method coerces args to String (matching JS console.log behaviour) and
    // forwards the joined line to the Rust sink.
    ctx.eval::<(), _>(
        r#"
        var console = (function() {
            function _fmt(args) {
                return Array.prototype.map.call(args, function(v) {
                    if (v === null) return 'null';
                    if (v === undefined) return 'undefined';
                    if (typeof v === 'object' || typeof v === 'function') {
                        try { return JSON.stringify(v); } catch(e) { return '[object]'; }
                    }
                    return String(v);
                }).join(' ');
            }
            return {
                log:   function() { __golden_log(_fmt(arguments)); },
                info:  function() { __golden_log(_fmt(arguments)); },
                warn:  function() { __golden_log(_fmt(arguments)); },
                error: function() { __golden_log(_fmt(arguments)); }
            };
        })();
        "#,
    )?;
    Ok(())
}

fn install_pm(
    ctx: &Ctx<'_>,
    collector: Rc<RefCell<Collector>>,
    vars: &HashMap<String, String>,
    request: &RequestView,
    response: Option<&ResponseView>,
    cfg: &HttpConfig,
) -> Result<(), rquickjs::Error> {
    // Base `pm` object — surface members added by the prelude.
    let pm = Object::new(ctx.clone())?;
    ctx.globals().set("pm", pm)?;

    // __pm_record(name, passed, error|null) -> push Assertion into the collector.
    {
        let col = collector.clone();
        let record = Function::new(
            ctx.clone(),
            move |name: String, passed: bool, error: Value<'_>| {
                let err_msg = if error.is_null() || error.is_undefined() {
                    None
                } else if let Some(s) = error.as_string() {
                    s.to_string().ok()
                } else {
                    None
                };
                col.borrow_mut().assertions.push(crate::result::Assertion {
                    name,
                    passed,
                    error: err_msg,
                });
            },
        )?;
        ctx.globals().set("__pm_record", record)?;
    }

    // ---- request snapshot (always present) ----
    {
        let rq = Object::new(ctx.clone())?;
        rq.set("method", request.method.clone())?;
        rq.set("url", request.url.clone())?;
        let hdrs = Object::new(ctx.clone())?;
        for (k, v) in &request.headers {
            hdrs.set(k.as_str(), v.clone())?;
        }
        rq.set("__headers", hdrs)?;
        match &request.body {
            Some(b) => rq.set("body", b.clone())?,
            None => rq.set("body", rquickjs::Undefined)?,
        }
        ctx.globals().set("__pm_request", rq)?;
    }

    // ---- response snapshot (test scripts only) ----
    if let Some(r) = response {
        let rs = Object::new(ctx.clone())?;
        rs.set("code", r.code)?;
        rs.set("status", r.status_text.clone())?;
        rs.set("text", r.body.clone())?;
        rs.set("responseTime", r.time_ms as f64)?;
        let hdrs = Object::new(ctx.clone())?;
        for (k, v) in &r.headers {
            hdrs.set(k.to_ascii_lowercase().as_str(), v.clone())?;
        }
        rs.set("__headers", hdrs)?;
        ctx.globals().set("__pm_response", rs)?;
    }

    // ---- variable read map: __pm_vars[key] -> value ----
    {
        let vmap = Object::new(ctx.clone())?;
        for (k, v) in vars {
            vmap.set(k.as_str(), v.clone())?;
        }
        ctx.globals().set("__pm_vars", vmap)?;
    }

    // ---- pm.info + pm.iterationData inputs ----
    {
        let info = Object::new(ctx.clone())?;
        info.set("requestName", request.name.clone())?;
        info.set("requestId", request.name.clone())?;
        info.set("iteration", request.iteration as f64)?;
        info.set("iterationCount", request.iteration_count as f64)?;
        ctx.globals().set("__pm_info", info)?;

        let data = Object::new(ctx.clone())?;
        for (k, v) in &request.data {
            data.set(k.as_str(), v.clone())?;
        }
        ctx.globals().set("__pm_data", data)?;
    }

    // ---- native setNextRequest recorder: __pm_set_next(name|null) ----
    {
        let col = collector.clone();
        let set_next = Function::new(ctx.clone(), move |name: Value<'_>| {
            let next = if name.is_null() || name.is_undefined() {
                None
            } else {
                name.as_string().and_then(|s| s.to_string().ok())
            };
            col.borrow_mut().next_request = Some(next);
        })?;
        ctx.globals().set("__pm_set_next", set_next)?;
    }

    // ---- native mutation recorder: __pm_mutate(op, scope, key, value|null) ----
    {
        let col = collector.clone();
        let mutate = Function::new(
            ctx.clone(),
            move |op: String, scope: String, key: String, value: Value<'_>| {
                let sc = match scope.as_str() {
                    "environment" => Scope::Environment,
                    "globals" => Scope::Globals,
                    "variables" => Scope::Local,
                    _ => Scope::Collection,
                };
                let val_str = if value.is_null() || value.is_undefined() {
                    None
                } else if let Some(s) = value.as_string() {
                    s.to_string().ok()
                } else {
                    None
                };
                let m = if op == "unset" {
                    ScopeOp::Unset { scope: sc, key }
                } else {
                    ScopeOp::Set {
                        scope: sc,
                        key,
                        value: val_str.unwrap_or_default(),
                    }
                };
                col.borrow_mut().mutations.push(m);
            },
        )?;
        ctx.globals().set("__pm_mutate", mutate)?;
    }

    // ---- pm.sendRequest native bridge ----
    // Returns a JSON string that the JS wrapper parses; avoids Object<'js> lifetime trouble.
    {
        let cfg = cfg.clone();
        let send_fn = Function::new(ctx.clone(), move |spec: Value<'_>| -> String {
            use crate::model::{Header, Request, Url};
            // Accept a string URL or an object {url, method, body}.
            let (method, url, headers, body) = if let Some(s) = spec.as_string() {
                (
                    "GET".to_string(),
                    s.to_string().unwrap_or_default(),
                    Vec::new(),
                    None,
                )
            } else if let Some(o) = spec.as_object() {
                let url: String = o.get("url").unwrap_or_default();
                let method: String = o.get("method").unwrap_or_else(|_| "GET".to_string());
                let body: Option<String> = o.get::<_, String>("body").ok();
                (method, url, Vec::<(String, String)>::new(), body)
            } else {
                ("GET".to_string(), String::new(), Vec::new(), None)
            };
            let req = Request {
                method,
                url: Url::Raw(url),
                header: headers
                    .into_iter()
                    .map(|(k, v): (String, String)| Header {
                        key: k,
                        value: v,
                        disabled: false,
                        extra: serde_json::Map::new(),
                    })
                    .collect(),
                body: body.map(|raw| crate::model::Body {
                    mode: "raw".into(),
                    raw: Some(serde_json::Value::String(raw)),
                    graphql: None,
                    formdata: vec![],
                }),
            };
            match crate::http::send(&req, &std::collections::HashMap::new(), &cfg) {
                Ok(resp) => {
                    let text = String::from_utf8_lossy(&resp.body).to_string();
                    // Escape the text safely for inclusion in JSON
                    let text_escaped =
                        serde_json::to_string(&text).unwrap_or_else(|_| "\"\"".to_string());
                    format!(
                        r#"{{"code":{},"status":{},"responseTime":{},"text":{},"__error":null}}"#,
                        resp.status, resp.status, resp.time_ms, text_escaped,
                    )
                }
                Err(e) => {
                    let e_escaped =
                        serde_json::to_string(&e).unwrap_or_else(|_| "\"error\"".to_string());
                    format!(r#"{{"__error":{}}}"#, e_escaped)
                }
            }
        })?;
        ctx.globals().set("__pm_send", send_fn)?;
    }

    // ---- require(module) registry ----
    {
        let shim_srcs: &[(&str, &str)] = &[
            ("lodash", crate::sandbox::shims::LODASH),
            ("crypto-js", crate::sandbox::shims::CRYPTO_JS),
            ("uuid", crate::sandbox::shims::UUID),
        ];
        // Evaluate each shim source into a stable global, then require() returns it.
        for (name, src) in shim_srcs {
            let key = format!("__shim_{}", name.replace('-', "_"));
            let val: Value = ctx.eval(*src)?;
            ctx.globals().set(key.as_str(), val)?;
        }

        // Native validator: __require_validate(name) -> throws for unknown modules,
        // returns the global key string for known ones. The JS wrapper does the
        // actual global lookup so we avoid the Value<'js> lifetime issue in closures.
        let validate = Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, name: String| -> rquickjs::Result<String> {
                let key: &'static str = match name.as_str() {
                    "lodash" => "__shim_lodash",
                    "crypto-js" => "__shim_crypto_js",
                    "uuid" => "__shim_uuid",
                    other => {
                        return Err(ctx.throw(
                            rquickjs::String::from_str(
                                ctx.clone(),
                                &format!("Cannot find module '{other}'"),
                            )?
                            .into(),
                        ));
                    }
                };
                Ok(key.to_string())
            },
        )?;
        ctx.globals().set("__require_validate", validate)?;

        // JS wrapper: require(name) -> globalThis[__require_validate(name)]
        ctx.eval::<(), _>(
            "var require = function(name) { return globalThis[__require_validate(name)]; };",
        )?;
    }

    // Evaluate the prelude (defines pm.test, pm.expect, pm.request, pm.response, scopes, sendRequest).
    ctx.eval::<(), _>(JS_PRELUDE)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpConfig;
    use crate::sandbox::{JsEngine, RequestView, ResponseView, Scope, ScopeOp};
    use std::collections::HashMap;

    fn req() -> RequestView {
        RequestView {
            method: "GET".into(),
            url: "http://x/y".into(),
            headers: vec![],
            body: None,
            ..Default::default()
        }
    }

    fn run(script: &str, resp: Option<&ResponseView>) -> ScriptOutcome {
        RquickJsEngine::new().run_script(
            script,
            &HashMap::new(),
            &req(),
            resp,
            &HttpConfig::default(),
        )
    }

    #[test]
    fn runs_script_and_captures_console_log() {
        let eng = RquickJsEngine::new();
        let out = eng.run_script(
            "console.log('hello', 42);",
            &HashMap::new(),
            &req(),
            None,
            &HttpConfig::default(),
        );
        assert!(out.error.is_none(), "unexpected error: {:?}", out.error);
        assert_eq!(out.logs, vec!["hello 42".to_string()]);
    }

    #[test]
    fn syntax_error_is_captured_not_panicked() {
        let eng = RquickJsEngine::new();
        let out = eng.run_script(
            "this is not js;;;{",
            &HashMap::new(),
            &req(),
            None,
            &HttpConfig::default(),
        );
        assert!(out.error.is_some());
    }

    // --- Task 4 helpers + tests ---

    fn resp() -> ResponseView {
        ResponseView {
            code: 201,
            status_text: "Created".into(),
            headers: vec![("content-type".into(), "application/json".into())],
            body: r#"{"id":7,"name":"x"}"#.into(),
            time_ms: 12,
        }
    }

    #[test]
    fn pm_response_surface() {
        let script = r#"
            pm.test("code", function () { pm.expect(pm.response.code).to.equal(201); });
            pm.test("status helper", function () { pm.response.to.have.status(201); });
            pm.test("json", function () { pm.expect(pm.response.json()).to.have.property('id', 7); });
            pm.test("text", function () { pm.expect(pm.response.text()).to.include('name'); });
            pm.test("time", function () { pm.expect(pm.response.responseTime).to.equal(12); });
            pm.test("header", function () {
                pm.expect(pm.response.headers.get('content-type')).to.include('json');
            });
        "#;
        let out = run(script, Some(&resp()));
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.assertions.len(), 6);
        assert!(
            out.assertions.iter().all(|a| a.passed),
            "{:?}",
            out.assertions
        );
    }

    #[test]
    fn pm_request_surface() {
        let r = RequestView {
            method: "POST".into(),
            url: "http://api/login".into(),
            headers: vec![("X-Token".into(), "abc".into())],
            body: Some("{}".into()),
            ..Default::default()
        };
        let out = RquickJsEngine::new().run_script(
            r#"pm.test("m", function () { pm.expect(pm.request.method).to.equal('POST'); });
               pm.test("u", function () { pm.expect(pm.request.url).to.include('login'); });"#,
            &HashMap::new(),
            &r,
            None,
            &HttpConfig::default(),
        );
        assert_eq!(out.assertions.len(), 2);
        assert!(
            out.assertions.iter().all(|a| a.passed),
            "{:?}",
            out.assertions
        );
    }

    // --- Task 3 tests ---

    #[test]
    fn pm_test_records_pass_and_fail() {
        let eng = RquickJsEngine::new();
        let script = r#"
            pm.test("passes", function () { /* no throw */ });
            pm.test("fails", function () { throw new Error("boom"); });
        "#;
        let out = eng.run_script(
            script,
            &HashMap::new(),
            &req(),
            None,
            &HttpConfig::default(),
        );
        assert!(out.error.is_none(), "script error: {:?}", out.error);
        assert_eq!(out.assertions.len(), 2);
        assert_eq!(out.assertions[0].name, "passes");
        assert!(out.assertions[0].passed);
        assert_eq!(out.assertions[1].name, "fails");
        assert!(!out.assertions[1].passed);
        assert_eq!(out.assertions[1].error.as_deref(), Some("boom"));
    }

    #[test]
    fn expect_subset_covers_real_corpus() {
        let script = r#"
            pm.test("equal", function () { pm.expect(1).to.equal(1); });
            pm.test("an array", function () { pm.expect([1,2]).to.be.an('array'); });
            pm.test("above", function () { pm.expect(5).to.be.above(0); });
            pm.test("property", function () { pm.expect({id:1}).to.have.property('id'); });
            pm.test("property value", function () { pm.expect({id:1}).to.have.property('id', 1); });
            pm.test("include", function () { pm.expect([1,2,3]).to.include(2); });
            pm.test("not", function () { pm.expect(1).to.not.equal(2); });
            pm.test("FAILS", function () { pm.expect(1).to.equal(2); });
        "#;
        let out = run(script, None);
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.assertions.len(), 8);
        for a in &out.assertions[..7] {
            assert!(a.passed, "{} should pass", a.name);
        }
        assert!(!out.assertions[7].passed);
    }

    // --- Task 5 tests ---

    #[test]
    fn pm_scope_get_returns_resolved_vars() {
        let vars = HashMap::from([("userId".to_string(), "99".to_string())]);
        let out = RquickJsEngine::new().run_script(
            r#"pm.test("get", function () { pm.expect(pm.collectionVariables.get('userId')).to.equal('99'); });
               pm.test("has", function () { pm.expect(pm.environment.has('userId')).to.equal(true); });
               pm.test("missing", function () { pm.expect(pm.variables.get('nope')).to.be.undefined; });"#,
            &vars, &req(), None, &HttpConfig::default());
        assert!(out.error.is_none(), "{:?}", out.error);
        assert!(
            out.assertions.iter().all(|a| a.passed),
            "{:?}",
            out.assertions
        );
    }

    #[test]
    fn pm_scope_set_unset_emits_mutations() {
        let out = run(
            r#"pm.collectionVariables.set('postId', '7');
               pm.environment.set('token', 'abc');
               pm.globals.set('g', '1');
               pm.variables.set('v', '2');
               pm.collectionVariables.unset('old');"#,
            None,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.mutations.len(), 5);
        let set_post = out.mutations.iter().any(|m| matches!(m,
            ScopeOp::Set { scope: Scope::Collection, key, value } if key == "postId" && value == "7"));
        assert!(set_post);
        let unset = out.mutations.iter().any(|m| {
            matches!(m,
            ScopeOp::Unset { scope: Scope::Collection, key } if key == "old")
        });
        assert!(unset);
        let env_set = out.mutations.iter().any(|m| {
            matches!(m,
            ScopeOp::Set { scope: Scope::Environment, key, .. } if key == "token")
        });
        assert!(env_set);
    }

    // --- Task 7 tests ---

    #[test]
    fn pm_cookies_stub_present() {
        let out = run(
            r#"pm.test("no cookie", function () { pm.expect(pm.cookies.has('sid')).to.equal(false); });
               pm.test("get undef", function () { pm.expect(pm.cookies.get('sid')).to.be.undefined; });"#,
            None,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert!(
            out.assertions.iter().all(|a| a.passed),
            "{:?}",
            out.assertions
        );
    }

    // --- Task 8 tests ---

    #[test]
    fn require_shims_work() {
        let script = r#"
            var _ = require('lodash');
            var CryptoJS = require('crypto-js');
            var uuid = require('uuid');
            pm.test("lodash get", function () {
                pm.expect(_.get({a:{b:2}}, 'a.b')).to.equal(2);
            });
            pm.test("lodash isEmpty", function () {
                pm.expect(_.isEmpty([])).to.equal(true);
            });
            pm.test("crypto md5 hex length", function () {
                pm.expect(CryptoJS.MD5('abc').toString().length).to.equal(32);
            });
            pm.test("crypto md5 abc digest", function () {
                pm.expect(CryptoJS.MD5('abc').toString()).to.equal('900150983cd24fb0d6963f7d28e17f72');
            });
            pm.test("uuid v4 shape", function () {
                pm.expect(uuid.v4().length).to.equal(36);
            });
        "#;
        let out = run(script, None);
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.assertions.len(), 5);
        assert!(
            out.assertions.iter().all(|a| a.passed),
            "{:?}",
            out.assertions
        );
    }

    #[test]
    fn require_unknown_throws() {
        let out = run(r#"require('not-a-real-module');"#, None);
        assert!(out.error.is_some());
        assert!(out.error.as_ref().unwrap().contains("not-a-real-module"));
    }

    #[test]
    fn atob_btoa_available() {
        let out = run(
            r#"
            pm.test("btoa", function () { pm.expect(btoa('hello')).to.equal('aGVsbG8='); });
            pm.test("atob", function () { pm.expect(atob('aGVsbG8=')).to.equal('hello'); });
            pm.test("roundtrip", function () { pm.expect(atob(btoa('Golden!'))).to.equal('Golden!'); });
        "#,
            None,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.assertions.len(), 3);
        assert!(
            out.assertions.iter().all(|a| a.passed),
            "{:?}",
            out.assertions
        );
    }

    #[test]
    fn pm_info_and_iteration_data_exposed() {
        let mut data = HashMap::new();
        data.insert("userId".to_string(), "u-42".to_string());
        let view = RequestView {
            method: "GET".into(),
            url: "http://api/x".into(),
            name: "Login".into(),
            iteration: 2,
            iteration_count: 5,
            data,
            ..Default::default()
        };
        let out = RquickJsEngine::new().run_script(
            r#"
            pm.test("requestName", function () { pm.expect(pm.info.requestName).to.equal('Login'); });
            pm.test("iteration", function () { pm.expect(pm.info.iteration).to.equal(2); });
            pm.test("iterationCount", function () { pm.expect(pm.info.iterationCount).to.equal(5); });
            pm.test("iterationData", function () { pm.expect(pm.iterationData.get('userId')).to.equal('u-42'); });
            "#,
            &HashMap::new(),
            &view,
            None,
            &HttpConfig::default(),
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.assertions.len(), 4);
        assert!(
            out.assertions.iter().all(|a| a.passed),
            "{:?}",
            out.assertions
        );
    }

    #[test]
    fn set_next_request_signal_captured() {
        assert_eq!(
            run(r#"pm.execution.setNextRequest('Step 2');"#, None).next_request,
            Some(Some("Step 2".to_string()))
        );
        assert_eq!(
            run(r#"postman.setNextRequest(null);"#, None).next_request,
            Some(None)
        );
        assert_eq!(run(r#"1 + 1;"#, None).next_request, None);
    }

    #[test]
    fn cryptojs_sha256_hmac_base64_and_md5() {
        let script = r#"
            var C = require('crypto-js');
            pm.test("sha256 hex", function () {
                pm.expect(C.SHA256('abc').toString())
                    .to.equal('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
            });
            pm.test("sha256 base64", function () {
                pm.expect(C.SHA256('abc').toString(C.enc.Base64))
                    .to.equal('ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=');
            });
            pm.test("hmac-sha256 hex", function () {
                pm.expect(C.HmacSHA256('The quick brown fox jumps over the lazy dog', 'key').toString())
                    .to.equal('f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8');
            });
            pm.test("md5 still works", function () {
                pm.expect(C.MD5('abc').toString()).to.equal('900150983cd24fb0d6963f7d28e17f72');
            });
            pm.test("enc.Base64.stringify", function () {
                pm.expect(C.enc.Base64.stringify(C.SHA256('abc')))
                    .to.equal('ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=');
            });
        "#;
        let out = run(script, None);
        assert!(out.error.is_none(), "script error: {:?}", out.error);
        assert_eq!(out.assertions.len(), 5);
        assert!(
            out.assertions.iter().all(|a| a.passed),
            "failed: {:?}",
            out.assertions
                .iter()
                .filter(|a| !a.passed)
                .collect::<Vec<_>>()
        );
    }

    // --- Task 6 tests ---

    #[test]
    fn pm_send_request_resolves_callback_synchronously() {
        use httpmock::prelude::*;
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/sub");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"v":5}"#);
        });
        let url = format!("{}/sub", server.base_url());
        let script = format!(
            r#"pm.sendRequest("{url}", function (err, res) {{
                 pm.test("sub ok", function () {{ pm.expect(res.code).to.equal(200); }});
                 pm.test("sub json", function () {{ pm.expect(res.json().v).to.equal(5); }});
               }});"#,
            url = url
        );
        let out = run(&script, None);
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.assertions.len(), 2);
        assert!(
            out.assertions.iter().all(|a| a.passed),
            "{:?}",
            out.assertions
        );
    }
}
