//! `golden run`: discover collections, resolve env, run via golden_core, emit
//! reports through the requested reporters, and return an exit code.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use golden_core::env::resolve;
use golden_core::http::HttpConfig;
use golden_core::result::{RequestResult, RunResult, Totals};
use golden_core::runner::{run_with_events, run_with_options, RequestEventHandler};

use crate::cli::{ReporterKind, RunArgs};
use crate::discovery::{discover, env_paths, expand_paths};
use crate::exit::{code_for_result, FATAL};
use crate::filter::{prune_collection, Filter};
use crate::load::{load, Loaded};
use crate::reporter::{default_if_empty, json_stream, reporter_for};

/// Execute the run command. Returns the process exit code.
pub fn execute(args: &RunArgs, collections_override: &[String]) -> i32 {
    let workspace = match std::env::current_dir() {
        Ok(d) => d,
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

    let filter = match Filter::new(args.filter.as_deref()) {
        Ok(f) => f.with_methods(&args.method),
        Err(e) => {
            eprintln!("golden: invalid --filter glob: {e}");
            return FATAL;
        }
    };

    let cfg = HttpConfig {
        insecure: args.insecure,
        timeout_ms: args.timeout,
    };

    let data = match &args.data {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => match golden_core::data::parse_data(&text) {
                Ok(rows) => rows,
                Err(e) => {
                    eprintln!("golden: {e}");
                    return FATAL;
                }
            },
            Err(e) => {
                eprintln!("golden: cannot read data file '{path}': {e}");
                return FATAL;
            }
        },
        None => Vec::new(),
    };

    // json-stream owns the whole output stream (NDJSON), so it cannot be
    // combined with other reporters writing to the same destination.
    let kinds = default_if_empty(&args.reporter);
    let streaming = kinds.contains(&ReporterKind::JsonStream);
    if streaming && kinds.len() > 1 {
        eprintln!("golden: --reporter json-stream cannot be combined with other reporters");
        return FATAL;
    }
    if streaming {
        return execute_stream(args, &files, &filter, &cfg, &data);
    }

    let mut merged = RunResult::default();
    for file in &files {
        let loaded = match load(file) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("golden: {e}");
                return FATAL;
            }
        };
        let single = run_one(
            loaded,
            &filter,
            args.iterations,
            &cfg,
            args.env.as_deref(),
            &data,
            None,
        );
        let bail_now =
            args.bail && (single.totals.failed_assertions > 0 || single.totals.failed_requests > 0);
        accumulate_result(&mut merged, single);
        if bail_now {
            break;
        }
    }

    if let Err(e) = emit(args, &merged) {
        eprintln!("golden: failed to write report: {e}");
        return FATAL;
    }

    code_for_result(&merged)
}

/// The `--reporter json-stream` run loop: NDJSON events (collection start,
/// each request as it completes, one terminal `done` with the merged result)
/// written to stdout — or to `--output` — flushed after every line so
/// consumers see live progress. Exit codes match the json reporter.
fn execute_stream(
    args: &RunArgs,
    files: &[PathBuf],
    filter: &Filter,
    cfg: &HttpConfig,
    data: &[HashMap<String, String>],
) -> i32 {
    let mut out: Box<dyn Write> = match &args.output {
        Some(path) => match File::create(path) {
            Ok(f) => Box::new(f),
            Err(e) => {
                eprintln!("golden: cannot create output file '{path}': {e}");
                return FATAL;
            }
        },
        None => Box::new(io::stdout()),
    };

    let mut merged = RunResult::default();
    for file in files {
        let loaded = match load(file) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("golden: {e}");
                return FATAL;
            }
        };
        let coll_name = loaded.collection.info.name.clone();
        if let Err(e) = write_line(&mut out, &json_stream::collection_line(&coll_name, file)) {
            eprintln!("golden: failed to write report: {e}");
            return FATAL;
        }
        let mut emit_err: Option<io::Error> = None;
        let mut on_request = |rr: &RequestResult, iteration: u32, _completed: usize| {
            if emit_err.is_some() {
                return;
            }
            if let Err(e) = write_line(
                &mut out,
                &json_stream::request_line(&coll_name, iteration, rr),
            ) {
                emit_err = Some(e);
            }
        };
        let single = run_one(
            loaded,
            filter,
            args.iterations,
            cfg,
            args.env.as_deref(),
            data,
            Some(&mut on_request),
        );
        if let Some(e) = emit_err {
            eprintln!("golden: failed to write report: {e}");
            return FATAL;
        }
        let bail_now =
            args.bail && (single.totals.failed_assertions > 0 || single.totals.failed_requests > 0);
        accumulate_result(&mut merged, single);
        if bail_now {
            break;
        }
    }

    if let Err(e) = write_line(&mut out, &json_stream::done_line(&merged)) {
        eprintln!("golden: failed to write report: {e}");
        return FATAL;
    }

    code_for_result(&merged)
}

/// Write one NDJSON line and flush, so consumers see it immediately.
fn write_line(out: &mut dyn Write, line: &str) -> io::Result<()> {
    writeln!(out, "{line}")?;
    out.flush()
}

fn run_one(
    mut loaded: Loaded,
    filter: &Filter,
    iterations: u32,
    cfg: &HttpConfig,
    env_override: Option<&str>,
    data: &[HashMap<String, String>],
    on_request: Option<RequestEventHandler<'_>>,
) -> RunResult {
    prune_collection(&mut loaded.collection, filter);

    // Resolve env relative to the loaded file's workspace/collections-root.
    // `--env` (a path/name) is honored by setting GOLDEN-style override: if it
    // points at a readable file, its parsed vars overlay the resolved scope.
    let mut scopes = resolve(
        &loaded.workspace,
        &loaded.collections_root,
        &loaded.collection.variable,
    );
    if let Some(env_sel) = env_override {
        apply_env_override(&loaded.workspace, env_sel, &mut scopes);
    }

    match on_request {
        Some(cb) => run_with_events(
            &loaded.collection,
            &scopes,
            iterations,
            cfg,
            None,
            data,
            Some(cb),
        ),
        None => run_with_options(&loaded.collection, &scopes, iterations, cfg, false, data),
    }
}

/// Overlay a selected .env (path or name) onto the resolved scopes.
/// A path that exists is read directly; a bare name resolves to
/// `<workspace>/.env.<name>`.
fn apply_env_override(workspace: &Path, sel: &str, scopes: &mut golden_core::env::VarScopes) {
    let candidate = {
        let direct = Path::new(sel);
        if direct.is_file() {
            direct.to_path_buf()
        } else {
            workspace.join(format!(".env.{sel}"))
        }
    };
    if let Ok(content) = std::fs::read_to_string(&candidate) {
        for (k, v) in golden_core::env::parse_env(&content) {
            if !v.is_empty() {
                scopes.set(k, v);
            }
        }
    }
}

fn accumulate_result(into: &mut RunResult, mut from: RunResult) {
    into.collections.append(&mut from.collections);
    accumulate_totals(&mut into.totals, &from.totals);
}

fn accumulate_totals(into: &mut Totals, from: &Totals) {
    into.requests += from.requests;
    into.failed_requests += from.failed_requests;
    into.assertions += from.assertions;
    into.failed_assertions += from.failed_assertions;
    into.total_ms += from.total_ms;
}

fn emit(args: &RunArgs, result: &RunResult) -> io::Result<()> {
    let kinds = default_if_empty(&args.reporter);
    match &args.output {
        Some(path) => {
            // File output: no color, all reporters concatenated.
            let mut file = File::create(path)?;
            for kind in kinds {
                reporter_for(kind).report(result, &mut file, false)?;
            }
        }
        None => {
            let color = supports_color();
            let stdout = io::stdout();
            let mut lock = stdout.lock();
            for kind in kinds {
                reporter_for(kind).report(result, &mut lock, color)?;
            }
        }
    }
    Ok(())
}

/// True when stdout is a tty and NO_COLOR is unset.
fn supports_color() -> bool {
    use std::io::IsTerminal;
    std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal()
}
