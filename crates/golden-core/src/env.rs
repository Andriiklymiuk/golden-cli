//! .env discovery and the extension's exact variable precedence.

use std::collections::HashMap;
use std::path::Path;

use crate::model::Variable;

/// Parse .env content. Splits on the FIRST '='; strips matching surrounding
/// quotes; for unquoted values strips an inline `\s+#` comment; skips blank and
/// '#' lines; keeps empty values. Mirrors the extension's parseEnvFile.
pub fn parse_env(content: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].trim().to_string();
        let mut value = line[eq + 1..].trim().to_string();

        let dq = value.starts_with('"') && value.ends_with('"') && value.len() >= 2;
        let sq = value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2;
        if dq || sq {
            value = value[1..value.len() - 1].to_string();
        } else if let Some(idx) = find_inline_comment(&value) {
            value = value[..idx].trim_end().to_string();
        }
        if !key.is_empty() {
            result.insert(key, value);
        }
    }
    result
}

/// Index of an inline comment start: whitespace followed by '#'. Mirrors /\s+#/.
fn find_inline_comment(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    for i in 1..bytes.len() {
        if bytes[i] == b'#' && bytes[i - 1].is_ascii_whitespace() {
            // return the index of the first whitespace of the run
            let mut start = i - 1;
            while start > 0 && bytes[start - 1].is_ascii_whitespace() {
                start -= 1;
            }
            return Some(start);
        }
    }
    None
}

/// Final resolved variable map for substitution.
#[derive(Debug, Clone, Default)]
pub struct VarScopes {
    vars: HashMap<String, String>,
}

impl VarScopes {
    pub fn get(&self, key: &str) -> Option<&String> {
        self.vars.get(key)
    }
    pub fn as_map(&self) -> &HashMap<String, String> {
        &self.vars
    }
    pub fn set(&mut self, key: String, value: String) {
        self.vars.insert(key, value);
    }
}

/// global = <workspace>/.env, then process env for keys ABSENT in global .env.
fn load_global(workspace: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let global_path = workspace.join(".env");
    if let Ok(content) = std::fs::read_to_string(&global_path) {
        for (k, v) in parse_env(&content) {
            map.insert(k, v);
        }
    }
    for (k, v) in std::env::vars() {
        // Skip empty process-env values to match the extension's truthy `if (value)`
        // guard (environmentService.ts loadGlobalVariables). .env still beats process env.
        if !v.is_empty() {
            map.entry(k).or_insert(v);
        }
    }
    map
}

/// local = copy of global, overlaid with `<dirname(collections_root)>/.env`.
fn load_local(workspace: &Path, collections_root: &Path) -> HashMap<String, String> {
    let mut map = load_global(workspace);
    let parent = collections_root.parent().unwrap_or(workspace);
    let local_path = parent.join(".env");
    if let Ok(content) = std::fs::read_to_string(&local_path) {
        for (k, v) in parse_env(&content) {
            map.insert(k, v); // local overlays global
        }
    }
    map
}

/// Precedence (highest->lowest): local .env > global .env > process env > collection vars.
/// Implemented as: base = collection vars, overlay env map, SKIP empty env values.
pub fn resolve(
    workspace: &Path,
    collections_root: &Path,
    collection_vars: &[Variable],
) -> VarScopes {
    let mut merged: HashMap<String, String> = HashMap::new();
    for v in collection_vars {
        if !v.key.is_empty() {
            merged.insert(v.key.clone(), v.value.clone());
        }
    }
    let env_map = load_local(workspace, collections_root);
    for (k, v) in env_map {
        if !v.is_empty() {
            merged.insert(k, v); // non-empty env wins over collection
        }
    }
    VarScopes { vars: merged }
}

#[cfg(test)]
mod resolve_tests {
    use super::*;
    use crate::model::Variable;
    use std::fs;
    use tempfile::tempdir;

    fn var(k: &str, v: &str) -> Variable {
        Variable {
            key: k.into(),
            value: v.into(),
            extra: Default::default(),
        }
    }

    #[test]
    fn env_overrides_collection_but_empty_does_not() {
        let ws = tempdir().unwrap();
        // global .env at <workspace>/.env
        fs::write(ws.path().join(".env"), "TOKEN=from_env\nEMPTY_VAR=\n").unwrap();
        // default config: collections root = <workspace>/collections -> local .env == global
        let collections_root = ws.path().join("collections");
        fs::create_dir_all(&collections_root).unwrap();

        let coll_vars = vec![
            var("TOKEN", "from_collection"),
            var("EMPTY_VAR", "collection_default"),
            var("ONLY_COLLECTION", "kept"),
        ];

        let scopes = resolve(ws.path(), &collections_root, &coll_vars);
        // .env wins over collection var
        assert_eq!(scopes.get("TOKEN").unwrap(), "from_env");
        // empty .env value does NOT override the collection default
        assert_eq!(scopes.get("EMPTY_VAR").unwrap(), "collection_default");
        // collection-only var survives
        assert_eq!(scopes.get("ONLY_COLLECTION").unwrap(), "kept");
    }

    #[test]
    fn local_env_is_parent_of_collections_root() {
        let ws = tempdir().unwrap();
        // collections root nested: <ws>/api/collections  -> local .env at <ws>/api/.env
        let collections_root = ws.path().join("api").join("collections");
        fs::create_dir_all(&collections_root).unwrap();
        fs::write(ws.path().join(".env"), "HOST=global\n").unwrap();
        fs::write(ws.path().join("api").join(".env"), "HOST=local\n").unwrap();

        let scopes = resolve(ws.path(), &collections_root, &[]);
        // local .env overlays global
        assert_eq!(scopes.get("HOST").unwrap(), "local");
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn parses_key_value_quotes_and_comments() {
        let input = "\
# a comment line
PLAIN=hello
SPACED = world
DQUOTED=\"with spaces\"
SQUOTED='single'
INLINE=val # trailing comment
QUOTED_HASH=\"keep # this\"
EMPTY=
WITH_EQ=a=b=c
";
        let m = parse_env(input);
        assert_eq!(m.get("PLAIN").unwrap(), "hello");
        assert_eq!(m.get("SPACED").unwrap(), "world");
        assert_eq!(m.get("DQUOTED").unwrap(), "with spaces");
        assert_eq!(m.get("SQUOTED").unwrap(), "single");
        assert_eq!(m.get("INLINE").unwrap(), "val");
        assert_eq!(m.get("QUOTED_HASH").unwrap(), "keep # this");
        assert_eq!(m.get("EMPTY").unwrap(), "");
        assert_eq!(m.get("WITH_EQ").unwrap(), "a=b=c");
        assert!(!m.contains_key("# a comment line"));
    }
}
