//! SPIKE (golden-core Task 8) — does real chai.js + pm.test run under rquickjs 0.6?
//!
//! This is throwaway exploratory code, NOT production. It exists to DECIDE the
//! sandbox strategy for Plan B: real chai.js vs a hand-written `pm.expect` subset.
//!
//! Run:  cargo run -p golden-core --example spike_pm
//!
//! What it does:
//!  1. Boots an rquickjs Context.
//!  2. Loads the real chai.js UMD build from /tmp/chai.js (downloaded by the
//!     spike driver; if absent it prints a clear message and skips the chai path).
//!  3. Injects a `pm` global:
//!       - pm.test(name, fn)            -> runs fn, catches throws, records result
//!       - pm.expect                    -> chai.expect (when chai loaded)
//!       - pm.response.code/.json()/.text()
//!       - pm.response.to.have.status() -> Postman chai plugin (registered manually)
//!       - pm.collectionVariables.get/set -> backed by a Rust-side HashMap
//!  4. Runs the audited representative scripts and prints collected results.
//!
//! The point is to validate the *real rquickjs 0.6 binding shape*: how native
//! fns are injected, how JS exceptions are caught, how values are marshalled,
//! and whether chai's getter/Proxy-heavy chains survive QuickJS.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rquickjs::{CatchResultExt, CaughtError, Context, Function, Object, Runtime, Value};

/// One `pm.test(...)` outcome, observed from the Rust side.
#[derive(Debug, Clone)]
struct TestOutcome {
    name: String,
    passed: bool,
    error: Option<String>,
}

/// Shared sandbox state the native closures write into.
#[derive(Default)]
struct SandboxState {
    outcomes: Vec<TestOutcome>,
    collection_vars: HashMap<String, String>,
}

type Shared = Rc<RefCell<SandboxState>>;

// Hardcoded fake response so scripts have something real to assert on.
const FAKE_STATUS: u16 = 200;
const FAKE_BODY: &str = r#"[{"id":1,"name":"x","email":"a@b.c"}]"#;
// A second body shaped like a single user, for the "user id matches" script.
const FAKE_USER_BODY: &str = r#"{"id":1,"name":"x","email":"a@b.c"}"#;

fn main() {
    let rt = Runtime::new().expect("create runtime");
    // QuickJS default JS stack limit is ~256KB; chai's assertion-builder machinery
    // (addChainableMethod / proxify) recurses deep enough to blow it. Bump it.
    // 0 == unlimited; we use a generous fixed cap instead to stay safe.
    rt.set_max_stack_size(4 * 1024 * 1024); // 4 MB
    rt.set_memory_limit(64 * 1024 * 1024); // 64 MB
    let ctx = Context::full(&rt).expect("create context");

    let state: Shared = Rc::new(RefCell::new(SandboxState::default()));
    // Pre-seed userId so the "user id matches" script has something to compare.
    state
        .borrow_mut()
        .collection_vars
        .insert("userId".into(), "1".into());

    // --- Phase 0: probe raw QuickJS capabilities chai depends on ----------
    probe_engine_features(&ctx);

    // --- Phase 1: try to load real chai.js --------------------------------
    let t0 = std::time::Instant::now();
    let chai_loaded = load_chai(&ctx);
    println!("[timing] chai load + plugin register: {:?}", t0.elapsed());

    // --- Phase 2: inject the pm global ------------------------------------
    inject_pm(&ctx, &state, chai_loaded);

    // --- Phase 3: run the audited representative scripts ------------------
    println!("\n=== Running representative scripts ===");

    // Script A + B use the array body. Script C uses the single-user body.
    run_script(&ctx, SCRIPT_STATUS, FAKE_BODY);
    run_script(&ctx, SCRIPT_ARRAY, FAKE_BODY);
    run_script(&ctx, SCRIPT_USER_MATCH, FAKE_USER_BODY);
    // Negative path: prove a failing chai assertion is caught & its message captured.
    run_script(&ctx, SCRIPT_EXPECT_FAIL, FAKE_BODY);

    // --- Phase 4: report --------------------------------------------------
    println!("\n=== Collected pm.test results ===");
    let st = state.borrow();
    for o in &st.outcomes {
        let mark = if o.passed { "PASS" } else { "FAIL" };
        match &o.error {
            Some(e) => println!("  [{mark}] {} :: {}", o.name, e),
            None => println!("  [{mark}] {}", o.name),
        }
    }
    println!(
        "\nTotals: {} tests, {} passed, {} failed.",
        st.outcomes.len(),
        st.outcomes.iter().filter(|o| o.passed).count(),
        st.outcomes.iter().filter(|o| !o.passed).count(),
    );
    println!("collectionVariables write-back: {:?}", st.collection_vars);
}

// ----------------------------------------------------------------------------
// Phase 0: feature probes
// ----------------------------------------------------------------------------
fn probe_engine_features(ctx: &Context) {
    println!("=== Engine feature probes ===");
    ctx.with(|ctx| {
        // getters
        let getter: bool = ctx
            .eval(
                r#"(() => { const o = { get x() { return 42; } }; return o.x === 42; })()"#,
            )
            .unwrap_or(false);
        println!("  getters supported:        {getter}");

        // Proxy
        let proxy: bool = ctx
            .eval(
                r#"(() => { try { const p = new Proxy({}, { get: () => 7 }); return p.anything === 7; } catch(e) { return false; } })()"#,
            )
            .unwrap_or(false);
        println!("  Proxy supported:          {proxy}");

        // Object.defineProperty with getter (chai uses this heavily)
        let defprop: bool = ctx
            .eval(
                r#"(() => { const o = {}; Object.defineProperty(o, 'to', { get() { return this; } }); return o.to === o; })()"#,
            )
            .unwrap_or(false);
        println!("  defineProperty getter:    {defprop}");

        // Symbol / Symbol.toPrimitive (chai flag machinery)
        let sym: bool = ctx
            .eval(r#"(() => typeof Symbol === 'function' && typeof Symbol.iterator === 'symbol')()"#)
            .unwrap_or(false);
        println!("  Symbol supported:         {sym}");
    });
}

// ----------------------------------------------------------------------------
// Phase 1: load chai
// ----------------------------------------------------------------------------
fn load_chai(ctx: &Context) -> bool {
    let chai_src = match std::fs::read_to_string("/tmp/chai.js") {
        Ok(s) => s,
        Err(_) => {
            println!(
                "\n[chai] /tmp/chai.js not found — skipping real-chai path. \
                 (download with: curl -sL https://cdnjs.cloudflare.com/ajax/libs/chai/4.4.1/chai.js -o /tmp/chai.js)"
            );
            return false;
        }
    };
    println!("\n[chai] loaded /tmp/chai.js ({} bytes)", chai_src.len());

    ctx.with(|ctx| {
        // chai's UMD wrapper probes for `global`/`self`/`window`. QuickJS has
        // none of these by default — only `globalThis`. Alias them so the UMD
        // factory attaches `chai` to a place we can reach.
        let prep = r#"
            var global = globalThis;
            var self = globalThis;
            var window = globalThis;
        "#;
        if let Err(e) = ctx.eval::<(), _>(prep).catch(&ctx) {
            println!("[chai] failed to set up global aliases: {e}");
            return false;
        }

        // Evaluate the UMD bundle. It assigns `globalThis.chai`.
        match ctx.eval::<(), _>(chai_src.as_bytes()).catch(&ctx) {
            Ok(()) => {}
            Err(e) => {
                println!("[chai] chai.js eval FAILED: {e}");
                return false;
            }
        }

        // Confirm chai.expect exists, then register the Postman `.status` plugin.
        let has_expect: bool = ctx
            .eval(r#"typeof globalThis.chai !== 'undefined' && typeof globalThis.chai.expect === 'function'"#)
            .unwrap_or(false);
        println!("[chai] chai.expect present: {has_expect}");
        if !has_expect {
            return false;
        }

        // Register Postman's `.to.have.status(n)` — NOT stock chai. Postman adds
        // it via chai.use(plugin). We replicate the essence: an assertion method
        // `status` that compares the (Postman-shaped) response's `.code`.
        // In real Postman the asserted object is a response wrapper exposing
        // `.code`; here pm.response.to is `chai.expect(pm.response)` so the
        // subject (`this._obj`) is the response object with a `.code` field.
        let plugin = r#"
            globalThis.chai.use(function (chai, utils) {
                chai.Assertion.addMethod('status', function (expected) {
                    var obj = this._obj;
                    var actual = (obj && typeof obj === 'object' && 'code' in obj) ? obj.code : obj;
                    this.assert(
                        actual === expected,
                        'expected response to have status #{exp} but got #{act}',
                        'expected response not to have status #{exp}',
                        expected,
                        actual
                    );
                });
            });
        "#;
        match ctx.eval::<(), _>(plugin).catch(&ctx) {
            Ok(()) => {
                println!("[chai] registered Postman-style .status() assertion");
                true
            }
            Err(e) => {
                println!("[chai] failed to register .status() plugin: {e}");
                false
            }
        }
    })
}

// ----------------------------------------------------------------------------
// Phase 2: inject pm
// ----------------------------------------------------------------------------
fn inject_pm(ctx: &Context, state: &Shared, chai_loaded: bool) {
    ctx.with(|ctx| {
        let globals = ctx.globals();
        let pm = Object::new(ctx.clone()).unwrap();

        // ---- pm.test(name, fn) -------------------------------------------
        // Records pass/fail by catching any throw from fn().
        let st_test = state.clone();
        let test_fn = Function::new(
            ctx.clone(),
            move |ctx_inner: rquickjs::Ctx, name: String, cb: Function| {
                // Call the JS callback; chai assertion failures throw.
                let res: rquickjs::Result<()> = cb.call(());
                let outcome = match res.catch(&ctx_inner) {
                    Ok(()) => TestOutcome {
                        name: name.clone(),
                        passed: true,
                        error: None,
                    },
                    Err(e) => TestOutcome {
                        name: name.clone(),
                        passed: false,
                        error: Some(extract_error_message(&e)),
                    },
                };
                st_test.borrow_mut().outcomes.push(outcome);
            },
        )
        .unwrap();
        pm.set("test", test_fn).unwrap();

        // ---- pm.collectionVariables.get/set ------------------------------
        let cv = Object::new(ctx.clone()).unwrap();
        let st_get = state.clone();
        let get_fn = Function::new(ctx.clone(), move |key: String| -> Option<String> {
            st_get.borrow().collection_vars.get(&key).cloned()
        })
        .unwrap();
        let st_set = state.clone();
        let set_fn = Function::new(ctx.clone(), move |key: String, val: Value| {
            // Coerce the value to a string (Postman stores vars as strings).
            let s = value_to_string(&val);
            st_set.borrow_mut().collection_vars.insert(key, s);
        })
        .unwrap();
        cv.set("get", get_fn).unwrap();
        cv.set("set", set_fn).unwrap();
        pm.set("collectionVariables", cv).unwrap();

        // ---- pm.expect ---------------------------------------------------
        if chai_loaded {
            // pm.expect = chai.expect
            let chai: Object = globals.get("chai").unwrap();
            let expect: Function = chai.get("expect").unwrap();
            pm.set("expect", expect).unwrap();
        }

        globals.set("pm", pm).unwrap();

        // ---- pm.response (built in JS so .json()/.text()/.code/.to are natural) ----
        // We construct it in JS and wire .to to chai.expect(self) so that
        // `pm.response.to.have.status(n)` resolves through the registered plugin.
        let resp_setup = format!(
            r#"
            (function() {{
                var rawBody = {body:?};
                pm.response = {{
                    code: {status},
                    status: 'OK',
                    json: function() {{ return JSON.parse(rawBody); }},
                    text: function() {{ return rawBody; }},
                }};
                {to_wire}
            }})();
            "#,
            body = FAKE_BODY,
            status = FAKE_STATUS,
            to_wire = if chai_loaded {
                // `.to` must be a *fresh* chai assertion each access (chai chains
                // are stateful), so define it as a getter.
                r#"Object.defineProperty(pm.response, 'to', {
                    get: function() { return pm.expect(pm.response); }
                });"#
            } else {
                ""
            }
        );
        if let Err(e) = ctx.eval::<(), _>(resp_setup).catch(&ctx) {
            println!("[pm] failed to set up pm.response: {e}");
        }
    });
}

/// Pull a human-readable message out of a caught JS error. chai throws an
/// `AssertionError` whose `.message` is the useful text ("expected ... to ...").
/// QuickJS's default Display of a thrown object is just "Object(0x..)", so we
/// reach into the value/exception explicitly. This is exactly the marshalling
/// the production sandbox must do to put a useful string in `Assertion.error`.
fn extract_error_message(e: &CaughtError) -> String {
    match e {
        CaughtError::Exception(exc) => exc
            .message()
            .or_else(|| exc.as_object().get::<_, String>("message").ok())
            .unwrap_or_else(|| "exception (no message)".to_string()),
        CaughtError::Value(val) => {
            // Thrown object that isn't an Error instance — read `.message`.
            if let Some(obj) = val.as_object() {
                if let Ok(msg) = obj.get::<_, String>("message") {
                    return msg;
                }
            }
            if let Some(s) = val.as_string() {
                return s.to_string().unwrap_or_default();
            }
            "thrown value (no message)".to_string()
        }
        CaughtError::Error(err) => err.to_string(),
    }
}

/// Best-effort coercion of a JS value to the string Postman would store.
fn value_to_string(val: &Value) -> String {
    if let Some(s) = val.as_string() {
        s.to_string().unwrap_or_default()
    } else if let Some(n) = val.as_number() {
        // Drop trailing .0 for integers (matches JS String(1) === "1").
        if n.fract() == 0.0 {
            format!("{}", n as i64)
        } else {
            format!("{n}")
        }
    } else if let Some(b) = val.as_bool() {
        b.to_string()
    } else {
        "[object]".to_string()
    }
}

// ----------------------------------------------------------------------------
// Phase 3: run a single script (re-pointing pm.response.json() body each time)
// ----------------------------------------------------------------------------
fn run_script(ctx: &Context, script: &str, body: &str) {
    ctx.with(|ctx| {
        // Repoint the fake response body for this script.
        let repoint = format!(
            r#"(function() {{ var b = {body:?}; pm.response.json = function() {{ return JSON.parse(b); }}; pm.response.text = function() {{ return b; }}; }})();"#,
            body = body
        );
        let _ = ctx.eval::<(), _>(repoint).catch(&ctx);

        if let Err(e) = ctx.eval::<(), _>(script).catch(&ctx) {
            // A throw *outside* pm.test (e.g. a syntax/setup error) — surface it.
            println!("[run] script-level error: {e}");
        }
    });
}

// ----------------------------------------------------------------------------
// Audited representative scripts (verbatim from the task brief)
// ----------------------------------------------------------------------------
const SCRIPT_STATUS: &str = r#"
pm.test('Status code is 200', function() {
    pm.response.to.have.status(200);
});
"#;

const SCRIPT_ARRAY: &str = r#"
pm.test('Response is an array', function() {
    const response = pm.response.json();
    pm.expect(response).to.be.an('array');
    pm.expect(response.length).to.be.above(0);
    const user = response[0];
    pm.expect(user).to.have.property('id');
    pm.collectionVariables.set('userId', user.id);
});
"#;

const SCRIPT_USER_MATCH: &str = r#"
pm.test('user id matches', function() {
    const response = pm.response.json();
    pm.expect(response.id.toString()).to.equal(pm.collectionVariables.get('userId'));
    pm.expect(response).to.have.property('email');
});
"#;

// Deliberately failing — confirms chai's AssertionError message is captured.
const SCRIPT_EXPECT_FAIL: &str = r#"
pm.test('NEGATIVE: status should be 404 (expected to fail)', function() {
    pm.response.to.have.status(404);
});
"#;
