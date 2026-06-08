//! The pm.* JavaScript sandbox. One JS context per request, behind a JsEngine
//! trait so the engine (rquickjs today, boa later) is swappable.

mod pm;
mod shims;

pub use pm::RquickJsEngine;

use std::collections::HashMap;

use crate::model::Request;
use crate::result::Assertion;

/// Which scope a `pm.<scope>.set/unset` call targets. The runner decides how each
/// maps onto its real maps (see Phase 3 for precedence).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Environment,
    Globals,
    Collection,
    /// `pm.variables.set` — request-local, but the extension/newman both fold it
    /// into the active variable set for chaining; we treat it as Collection.
    Local,
}

/// A single scope mutation emitted by a script (`pm.environment.set("k","v")`,
/// `pm.collectionVariables.unset("k")`, ...). Applied by the runner after the script.
#[derive(Debug, Clone)]
pub enum ScopeOp {
    Set {
        scope: Scope,
        key: String,
        value: String,
    },
    Unset {
        scope: Scope,
        key: String,
    },
}

/// Everything a script needs as input + everything it produces as output.
#[derive(Debug, Default)]
pub struct ScriptOutcome {
    /// Assertions from `pm.test(...)` (test scripts) — empty for pre-request scripts.
    pub assertions: Vec<Assertion>,
    /// Scope mutations to apply (write-back / chaining).
    pub mutations: Vec<ScopeOp>,
    /// console.log/info/warn/error lines, in order (for CLI verbose / debugging).
    pub logs: Vec<String>,
    /// A hard script error (threw outside a pm.test, syntax error). The request
    /// still records whatever assertions ran before the throw.
    pub error: Option<String>,
    /// `pm.execution.setNextRequest`/`postman.setNextRequest`: `None` = not called;
    /// `Some(None)` = stop the run; `Some(Some(name))` = jump to `name`.
    pub next_request: Option<Option<String>>,
}

/// Read-only response data handed to a *test* script. None for pre-request.
#[derive(Debug, Clone)]
pub struct ResponseView {
    pub code: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub time_ms: u128,
}

/// Snapshot of the request as a *script* sees it (post-substitution URL/headers).
/// Also carries run context for `pm.info`/`pm.iterationData` (set by the runner).
#[derive(Debug, Clone, Default)]
pub struct RequestView {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub name: String,
    pub iteration: u32,
    pub iteration_count: u32,
    pub data: HashMap<String, String>,
}

impl RequestView {
    pub fn from_request(req: &Request, vars: &HashMap<String, String>) -> Self {
        use crate::subst::substitute;
        RequestView {
            method: req.method.clone(),
            url: substitute(req.url.raw(), vars),
            headers: req
                .header
                .iter()
                .filter(|h| !h.disabled)
                .map(|h| (substitute(&h.key, vars), substitute(&h.value, vars)))
                .collect(),
            body: req.body.as_ref().and_then(|b| b.raw.as_ref()).map(|r| {
                let text = match r {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                substitute(&text, vars)
            }),
            name: String::new(),
            iteration: 0,
            iteration_count: 1,
            data: HashMap::new(),
        }
    }
}

/// A swappable JS engine. The runner only ever talks to this trait.
pub trait JsEngine {
    /// Run a script.
    /// - `script`: the joined `script.exec` lines.
    /// - `vars`: current resolved variable map (read side for `pm.*.get`).
    /// - `request`: the request the script is attached to.
    /// - `response`: Some for test scripts, None for pre-request scripts.
    /// - `cfg`: HTTP config, so `pm.sendRequest` can reuse the executor.
    fn run_script(
        &self,
        script: &str,
        vars: &HashMap<String, String>,
        request: &RequestView,
        response: Option<&ResponseView>,
        cfg: &crate::http::HttpConfig,
    ) -> ScriptOutcome;
}
