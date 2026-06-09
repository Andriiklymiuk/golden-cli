//! Edit-mode state for the request pane. A small enum + buffer that the app's
//! key handler drives; on commit it calls golden_core::store::* and saves.

use golden_core::model::Collection;
use golden_core::store;

/// Which request field the user is currently editing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditField {
    Method,
    Url,
    HeadersJson,
    BodyRaw,
    GraphqlQuery,
    GraphqlVariables,
    PreRequestScript,
    TestScript,
}

impl EditField {
    /// Human-readable title for the edit overlay.
    pub fn title(&self) -> &'static str {
        match self {
            EditField::Method => "Edit Method",
            EditField::Url => "Edit URL",
            EditField::HeadersJson => "Edit Headers (JSON)",
            EditField::BodyRaw => "Edit Body (raw)",
            EditField::GraphqlQuery => "Edit GraphQL Query",
            EditField::GraphqlVariables => "Edit GraphQL Variables",
            EditField::PreRequestScript => "Edit Pre-request Script",
            EditField::TestScript => "Edit Test Script",
        }
    }
}

/// Active edit session: the targeted item path + a text buffer.
#[derive(Debug, Clone)]
pub struct EditSession {
    pub field: EditField,
    /// Index path into the collection's item tree (NOT including the collection
    /// index — that lives on the `App` side as `collection_index`).
    pub path: Vec<usize>,
    pub buffer: String,
}

impl EditSession {
    pub fn new(field: EditField, path: Vec<usize>, initial: String) -> Self {
        EditSession {
            field,
            path,
            buffer: initial,
        }
    }

    /// Apply the buffer to the collection. Returns Ok(()) or a user-facing error string.
    pub fn commit(&self, coll: &mut Collection) -> Result<(), String> {
        match self.field {
            EditField::Method => store::set_method(&mut coll.item, &self.path, self.buffer.trim()),
            EditField::Url => store::set_url(&mut coll.item, &self.path, &self.buffer),
            EditField::HeadersJson => {
                store::set_headers_from_json(&mut coll.item, &self.path, &self.buffer)
            }
            EditField::BodyRaw => {
                if self.buffer.is_empty() {
                    store::clear_body(&mut coll.item, &self.path)
                } else {
                    store::set_raw_body(&mut coll.item, &self.path, &self.buffer)
                }
            }
            EditField::GraphqlQuery => {
                // Preserve any existing variables so editing the query in isolation
                // doesn't drop them.
                let existing_vars = existing_graphql_variables_json(coll, &self.path);
                store::set_graphql_body(
                    &mut coll.item,
                    &self.path,
                    &self.buffer,
                    existing_vars.as_deref(),
                )
            }
            EditField::GraphqlVariables => {
                // Variables are only meaningful alongside a query: preserve the
                // existing query and apply the edited variables onto it.
                let existing_query = existing_graphql_query(coll, &self.path);
                let vars = if self.buffer.trim().is_empty() {
                    None
                } else {
                    Some(self.buffer.as_str())
                };
                store::set_graphql_body(&mut coll.item, &self.path, &existing_query, vars)
            }
            EditField::PreRequestScript => store::set_script(
                &mut coll.item,
                &self.path,
                "prerequest",
                &split_lines(&self.buffer),
            ),
            EditField::TestScript => store::set_script(
                &mut coll.item,
                &self.path,
                "test",
                &split_lines(&self.buffer),
            ),
        }
    }
}

/// Read the request's existing GraphQL query at `path` (empty if absent), so a
/// variables-only edit can re-apply it alongside the new variables.
fn existing_graphql_query(coll: &Collection, path: &[usize]) -> String {
    initial_text_for(coll, path, &EditField::GraphqlQuery)
}

/// Read the request's existing GraphQL variables at `path` as a JSON string,
/// or None if there are none — so a query-only edit can preserve them.
fn existing_graphql_variables_json(coll: &Collection, path: &[usize]) -> Option<String> {
    let s = initial_text_for(coll, path, &EditField::GraphqlVariables);
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Postman scripts are stored as `exec: string[]` (one entry per line).
pub fn split_lines(s: &str) -> Vec<String> {
    if s.is_empty() {
        return Vec::new();
    }
    s.split('\n').map(|l| l.to_string()).collect()
}

/// Extract the current text value of a field from a collection item so the
/// buffer is pre-filled when entering edit mode.
pub fn initial_text_for(coll: &Collection, path: &[usize], field: &EditField) -> String {
    let item = {
        // Walk read-only (no store fn needed).
        let (first, rest) = match path.split_first() {
            Some(v) => v,
            None => return String::new(),
        };
        let items: &[golden_core::model::Item] = &coll.item;
        let item = match items.get(*first) {
            Some(i) => i,
            None => return String::new(),
        };
        let mut cur = item;
        for &idx in rest {
            let children = match cur.item.as_deref() {
                Some(c) => c,
                None => return String::new(),
            };
            cur = match children.get(idx) {
                Some(i) => i,
                None => return String::new(),
            };
        }
        cur
    };

    let req = match item.request.as_ref() {
        Some(r) => r,
        None => return String::new(),
    };

    match field {
        EditField::Method => req.method.clone(),
        EditField::Url => req.url.raw().to_string(),
        EditField::HeadersJson => serde_json::to_string_pretty(&req.header).unwrap_or_default(),
        EditField::BodyRaw => req
            .body
            .as_ref()
            .and_then(|b| b.raw.as_ref())
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        EditField::GraphqlQuery => req
            .body
            .as_ref()
            .and_then(|b| b.graphql.as_ref())
            .map(|g| g.query.clone())
            .unwrap_or_default(),
        EditField::GraphqlVariables => req
            .body
            .as_ref()
            .and_then(|b| b.graphql.as_ref())
            .and_then(|g| g.variables.as_ref())
            .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
            .unwrap_or_default(),
        EditField::PreRequestScript => item
            .event
            .iter()
            .find(|e| e.listen == "prerequest")
            .map(|e| e.script.exec.join("\n"))
            .unwrap_or_default(),
        EditField::TestScript => item
            .event
            .iter()
            .find(|e| e.listen == "test")
            .map(|e| e.script.exec.join("\n"))
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coll() -> Collection {
        let json = r#"{"info":{"name":"C"},"item":[{"name":"r","request":{
            "method":"GET","url":"https://x/a","header":[]}}]}"#;
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn commit_method_edit_updates_collection() {
        let mut c = coll();
        let s = EditSession::new(EditField::Method, vec![0], "DELETE".into());
        s.commit(&mut c).unwrap();
        assert_eq!(c.item[0].request.as_ref().unwrap().method, "DELETE");
    }

    #[test]
    fn commit_url_edit_updates_collection() {
        let mut c = coll();
        let s = EditSession::new(EditField::Url, vec![0], "https://y/b".into());
        s.commit(&mut c).unwrap();
        assert_eq!(c.item[0].request.as_ref().unwrap().url.raw(), "https://y/b");
    }

    #[test]
    fn commit_test_script_splits_lines_into_exec() {
        let mut c = coll();
        let s = EditSession::new(EditField::TestScript, vec![0], "line1\nline2".into());
        s.commit(&mut c).unwrap();
        let ev = &c.item[0].event[0];
        assert_eq!(ev.listen, "test");
        assert_eq!(
            ev.script.exec,
            vec!["line1".to_string(), "line2".to_string()]
        );
    }

    #[test]
    fn commit_prerequest_script_splits_lines_into_exec() {
        let mut c = coll();
        let s = EditSession::new(EditField::PreRequestScript, vec![0], "a\nb\nc".into());
        s.commit(&mut c).unwrap();
        let ev = &c.item[0].event[0];
        assert_eq!(ev.listen, "prerequest");
        assert_eq!(ev.script.exec, vec!["a", "b", "c"]);
    }

    #[test]
    fn commit_bad_headers_json_returns_error() {
        let mut c = coll();
        let s = EditSession::new(EditField::HeadersJson, vec![0], "nope".into());
        assert!(s.commit(&mut c).is_err());
    }

    #[test]
    fn commit_raw_body_sets_body_mode() {
        let mut c = coll();
        let s = EditSession::new(EditField::BodyRaw, vec![0], r#"{"key":"val"}"#.into());
        s.commit(&mut c).unwrap();
        let body = c.item[0].request.as_ref().unwrap().body.as_ref().unwrap();
        assert_eq!(body.mode, "raw");
    }

    #[test]
    fn commit_empty_raw_body_clears_body() {
        let mut c = coll();
        // first set a body
        EditSession::new(EditField::BodyRaw, vec![0], "some data".into())
            .commit(&mut c)
            .unwrap();
        assert!(c.item[0].request.as_ref().unwrap().body.is_some());
        // then clear it with empty buffer
        EditSession::new(EditField::BodyRaw, vec![0], "".into())
            .commit(&mut c)
            .unwrap();
        assert!(c.item[0].request.as_ref().unwrap().body.is_none());
    }

    #[test]
    fn commit_graphql_query_sets_mode_graphql() {
        let mut c = coll();
        let s = EditSession::new(EditField::GraphqlQuery, vec![0], "{ me { id } }".into());
        s.commit(&mut c).unwrap();
        let body = c.item[0].request.as_ref().unwrap().body.as_ref().unwrap();
        assert_eq!(body.mode, "graphql");
        assert_eq!(body.graphql.as_ref().unwrap().query, "{ me { id } }");
    }

    #[test]
    fn commit_graphql_variables_persists_onto_existing_query() {
        let mut c = coll();
        // First set a query so variables have a query to attach to.
        EditSession::new(EditField::GraphqlQuery, vec![0], "{ me { id } }".into())
            .commit(&mut c)
            .unwrap();
        // Now edit only the variables.
        EditSession::new(EditField::GraphqlVariables, vec![0], r#"{"x":1}"#.into())
            .commit(&mut c)
            .unwrap();
        let body = c.item[0].request.as_ref().unwrap().body.as_ref().unwrap();
        assert_eq!(body.mode, "graphql");
        let g = body.graphql.as_ref().unwrap();
        // Query must be preserved, variables applied.
        assert_eq!(g.query, "{ me { id } }");
        assert_eq!(g.variables.as_ref().unwrap()["x"], 1);
    }

    #[test]
    fn commit_graphql_query_preserves_existing_variables() {
        let mut c = coll();
        // Seed a graphql body with variables.
        store::set_graphql_body(&mut c.item, &[0], "{ a }", Some(r#"{"k":"v"}"#)).unwrap();
        // Edit only the query.
        EditSession::new(EditField::GraphqlQuery, vec![0], "{ b }".into())
            .commit(&mut c)
            .unwrap();
        let g = c.item[0]
            .request
            .as_ref()
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .graphql
            .as_ref()
            .unwrap();
        assert_eq!(g.query, "{ b }");
        // Variables survive a query-only edit.
        assert_eq!(g.variables.as_ref().unwrap()["k"], "v");
    }

    #[test]
    fn commit_graphql_variables_empty_clears_variables_keeps_query() {
        let mut c = coll();
        store::set_graphql_body(&mut c.item, &[0], "{ a }", Some(r#"{"k":"v"}"#)).unwrap();
        // Clear variables with an empty buffer.
        EditSession::new(EditField::GraphqlVariables, vec![0], "".into())
            .commit(&mut c)
            .unwrap();
        let g = c.item[0]
            .request
            .as_ref()
            .unwrap()
            .body
            .as_ref()
            .unwrap()
            .graphql
            .as_ref()
            .unwrap();
        assert_eq!(g.query, "{ a }");
        assert!(g.variables.is_none());
    }

    #[test]
    fn commit_graphql_variables_bad_json_returns_error() {
        let mut c = coll();
        store::set_graphql_body(&mut c.item, &[0], "{ a }", None).unwrap();
        let s = EditSession::new(EditField::GraphqlVariables, vec![0], "not json".into());
        assert!(s.commit(&mut c).is_err());
    }

    #[test]
    fn initial_text_for_method_returns_current_method() {
        let c = coll();
        assert_eq!(initial_text_for(&c, &[0], &EditField::Method), "GET");
    }

    #[test]
    fn initial_text_for_url_returns_raw_url() {
        let c = coll();
        assert_eq!(initial_text_for(&c, &[0], &EditField::Url), "https://x/a");
    }

    #[test]
    fn initial_text_for_empty_body_is_empty_string() {
        let c = coll();
        assert_eq!(initial_text_for(&c, &[0], &EditField::BodyRaw), "");
    }

    #[test]
    fn split_lines_empty_returns_empty_vec() {
        assert_eq!(split_lines(""), Vec::<String>::new());
    }

    #[test]
    fn split_lines_single_line_returns_one_element() {
        assert_eq!(split_lines("hello"), vec!["hello".to_string()]);
    }

    #[test]
    fn split_lines_multiline_returns_all_lines() {
        assert_eq!(
            split_lines("a\nb\nc"),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn commit_and_save_persists_to_disk() {
        use std::fs;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let path = dir.path().join("test.json");

        let json = r#"{"info":{"name":"C"},"item":[{"name":"r","request":{
            "method":"GET","url":"https://x/a","header":[]}}]}"#;
        fs::write(&path, json).unwrap();

        let mut c: Collection = serde_json::from_str(json).unwrap();
        let s = EditSession::new(EditField::Method, vec![0], "PATCH".into());
        s.commit(&mut c).unwrap();
        store::save_collection(&path, &c).unwrap();

        let reloaded = store::load_collection(&path).unwrap();
        assert_eq!(reloaded.item[0].request.as_ref().unwrap().method, "PATCH");
    }
}
