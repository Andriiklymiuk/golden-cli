//! `golden history <list|clear|on|off|replay N>`.

use golden_core::history;
use golden_core::http::{send as core_send, HttpConfig};
use golden_core::model::{Body, Header, Request, Url};

use crate::cli::HistoryAction;
use crate::exit::FATAL;

/// Execute the history subcommand. Returns the process exit code.
pub fn execute(action: &HistoryAction) -> i32 {
    let ws = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("golden: cannot read current dir: {e}");
            return FATAL;
        }
    };

    match action {
        HistoryAction::List { json } => match history::read_all(&ws) {
            Ok(entries) => {
                // --json: one JSON array on stdout (newest last, [] when empty).
                if *json {
                    return match serde_json::to_string_pretty(&entries) {
                        Ok(s) => {
                            println!("{s}");
                            0
                        }
                        Err(e) => {
                            eprintln!("golden: {e}");
                            FATAL
                        }
                    };
                }
                if entries.is_empty() {
                    println!("(no history)");
                }
                for (i, e) in entries.iter().enumerate() {
                    let status = e
                        .status
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "ERR".into());
                    println!(
                        "{:>3}  {}  {:<6} {} -> {} ({}ms)",
                        i + 1,
                        e.timestamp,
                        e.method,
                        e.url,
                        status,
                        e.time_ms
                    );
                }
                0
            }
            Err(e) => {
                eprintln!("golden: {e}");
                FATAL
            }
        },
        HistoryAction::Clear => match history::clear(&ws) {
            Ok(_) => {
                println!("history cleared");
                0
            }
            Err(e) => {
                eprintln!("golden: {e}");
                FATAL
            }
        },
        HistoryAction::Off => match history::set_enabled(&ws, false) {
            Ok(_) => {
                println!("history recording disabled");
                0
            }
            Err(e) => {
                eprintln!("golden: {e}");
                FATAL
            }
        },
        HistoryAction::On => match history::set_enabled(&ws, true) {
            Ok(_) => {
                println!("history recording enabled");
                0
            }
            Err(e) => {
                eprintln!("golden: {e}");
                FATAL
            }
        },
        HistoryAction::Replay { index } => replay(&ws, *index),
    }
}

fn replay(ws: &std::path::Path, index: usize) -> i32 {
    let entries = match history::read_all(ws) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("golden: {e}");
            return FATAL;
        }
    };
    if index == 0 || index > entries.len() {
        eprintln!("golden: index {index} out of range (1..={})", entries.len());
        return FATAL;
    }
    let e = &entries[index - 1];
    let req = Request {
        method: e.method.clone(),
        url: Url::Raw(e.url.clone()),
        header: e
            .request_headers
            .iter()
            .map(|(k, v)| Header {
                key: k.clone(),
                value: v.clone(),
                disabled: false,
                extra: serde_json::Map::new(),
            })
            .collect(),
        body: e.request_body.clone().map(|raw| Body {
            mode: "raw".into(),
            raw: Some(serde_json::Value::String(raw)),
            graphql: None,
            formdata: vec![],
        }),
    };
    match core_send(
        &req,
        &std::collections::HashMap::new(),
        &HttpConfig::default(),
    ) {
        Ok(resp) => {
            println!("{} {}", resp.status, String::from_utf8_lossy(&resp.body));
            0
        }
        Err(err) => {
            eprintln!("golden: {err}");
            FATAL
        }
    }
}
