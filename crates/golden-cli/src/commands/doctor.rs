//! `golden doctor`: workspace + setup health. Reports problems and how to fix
//! them; `--fix` applies the safe ones (currently: seed `collections/` when none
//! are found). Offline — no network.

use std::collections::{BTreeSet, HashMap};

use golden_core::env::resolve;
use golden_core::model::{Collection, Item};

use crate::discovery::{discover, env_paths};
use crate::exit::FATAL;
use crate::load::load;

pub fn execute(fix: bool, collections_override: &[String]) -> i32 {
    let workspace = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("golden: cannot read current dir: {e}");
            return FATAL;
        }
    };

    let files = discover(&workspace, collections_override, env_paths());

    if files.is_empty() {
        if fix {
            println!("• no collections found — seeding via `golden init`…");
            return crate::commands::init::execute();
        }
        println!("✗ no collections found under collections/ (.golden/, .retriever/).");
        println!("  fix: run `golden init` (or `golden doctor --fix`).");
        return 1;
    }
    println!("✓ {} collection file(s) discovered", files.len());

    let mut problems = 0usize;
    let mut warnings = 0usize;
    for file in &files {
        let label = file.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        match load(file) {
            Err(e) => {
                println!("✗ {label}: {e}");
                problems += 1;
            }
            Ok(loaded) => {
                let scopes = resolve(
                    &loaded.workspace,
                    &loaded.collections_root,
                    &loaded.collection.variable,
                );
                let unresolved = unresolved_vars(&loaded.collection, scopes.as_map());
                if unresolved.is_empty() {
                    println!("✓ {label}: valid; every {{{{var}}}} resolves");
                } else {
                    println!(
                        "⚠ {label}: valid; unresolved vars: {}",
                        unresolved.join(", ")
                    );
                    warnings += 1;
                }
            }
        }
    }

    if problems > 0 {
        println!("\ngolden doctor: {problems} problem(s) found.");
        return 1;
    }
    if warnings > 0 {
        println!(
            "\ngolden doctor: OK ({warnings} warning(s) — unresolved vars may be set at runtime or via --env)."
        );
    } else {
        println!("\ngolden doctor: OK");
    }
    0
}

/// Variable names referenced as `{{name}}` in any request URL/header/body that
/// are neither defined in `vars` nor dynamic (`{{$...}}`). Sorted + deduped.
pub fn unresolved_vars(coll: &Collection, vars: &HashMap<String, String>) -> Vec<String> {
    let mut out = BTreeSet::new();
    walk(&coll.item, vars, &mut out);
    out.into_iter().collect()
}

fn walk(items: &[Item], vars: &HashMap<String, String>, out: &mut BTreeSet<String>) {
    for item in items {
        if let Some(req) = &item.request {
            extract(req.url.raw(), vars, out);
            for h in &req.header {
                extract(&h.key, vars, out);
                extract(&h.value, vars, out);
            }
            if let Some(raw) = req.body.as_ref().and_then(|b| b.raw.as_ref()) {
                let text = match raw {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                extract(&text, vars, out);
            }
        }
        if let Some(children) = &item.item {
            walk(children, vars, out);
        }
    }
}

fn extract(text: &str, vars: &HashMap<String, String>, out: &mut BTreeSet<String>) {
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else { break };
        let name = after[..close].trim();
        if !name.is_empty() && !name.starts_with('$') && !vars.contains_key(name) {
            out.insert(name.to_string());
        }
        rest = &after[close + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coll(json: &str) -> Collection {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn flags_unresolved_keeps_resolved_and_dynamic() {
        let c = coll(
            r#"{"info":{"name":"C"},"item":[
                {"name":"r","request":{"method":"GET","url":"{{base}}/u/{{userId}}?t={{$guid}}",
                 "header":[{"key":"Authorization","value":"Bearer {{token}}"}]}}
            ]}"#,
        );
        let vars = HashMap::from([("base".to_string(), "https://x".to_string())]);
        // base resolved; $guid dynamic (ignored); userId + token unresolved (sorted).
        assert_eq!(
            unresolved_vars(&c, &vars),
            vec!["token".to_string(), "userId".to_string()]
        );
    }

    #[test]
    fn none_unresolved_when_all_defined() {
        let c = coll(
            r#"{"info":{"name":"C"},"item":[
                {"name":"r","request":{"method":"GET","url":"{{base}}/ping"}}
            ]}"#,
        );
        let vars = HashMap::from([("base".to_string(), "x".to_string())]);
        assert!(unresolved_vars(&c, &vars).is_empty());
    }

    #[test]
    fn walks_nested_folders() {
        let c = coll(
            r#"{"info":{"name":"C"},"item":[
                {"name":"f","item":[{"name":"r","request":{"method":"GET","url":"{{deep}}/x"}}]}
            ]}"#,
        );
        assert_eq!(
            unresolved_vars(&c, &HashMap::new()),
            vec!["deep".to_string()]
        );
    }
}
