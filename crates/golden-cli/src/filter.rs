//! --filter glob matching over request/folder names. Backed by globset, with
//! literal-substring fallback semantics provided by leading/trailing `*`.

use globset::{Glob, GlobMatcher};

use golden_core::model::{Collection, Item};

/// A compiled request filter: an optional name glob plus an optional HTTP-method
/// allow-list. A `None` glob matches every name; an empty method list matches every
/// method. The two compose — a request must pass both to be kept.
pub struct Filter {
    matcher: Option<GlobMatcher>,
    methods: Option<Vec<String>>,
}

impl Filter {
    /// Compile a glob, or build a pass-through filter when `pattern` is None.
    pub fn new(pattern: Option<&str>) -> Result<Self, String> {
        let matcher = match pattern {
            None => None,
            Some(p) => Some(Glob::new(p).map_err(|e| e.to_string())?.compile_matcher()),
        };
        Ok(Filter {
            matcher,
            methods: None,
        })
    }

    /// Restrict to the given HTTP methods (case-insensitive). Empty = no restriction.
    pub fn with_methods(mut self, methods: &[String]) -> Self {
        if !methods.is_empty() {
            self.methods = Some(methods.iter().map(|m| m.to_uppercase()).collect());
        }
        self
    }

    /// Does `name` pass the name glob?
    pub fn matches(&self, name: &str) -> bool {
        match &self.matcher {
            None => true,
            Some(m) => m.is_match(name),
        }
    }

    /// Does this request item pass the method allow-list?
    pub fn method_ok(&self, item: &Item) -> bool {
        match &self.methods {
            None => true,
            Some(allowed) => item
                .request
                .as_ref()
                .is_some_and(|r| allowed.contains(&r.method.to_uppercase())),
        }
    }
}

/// Retain only requests that pass the filter (name glob AND method allow-list); keep a
/// folder only if it still has matching descendants. A request also passes the name check
/// if its parent folder name matches (folder-level filter). Mutates the item tree.
pub fn prune_collection(collection: &mut Collection, filter: &Filter) {
    collection.item = prune_items(std::mem::take(&mut collection.item), filter, false);
}

fn prune_items(items: Vec<Item>, filter: &Filter, parent_matched: bool) -> Vec<Item> {
    let mut kept = Vec::new();
    for mut item in items {
        if item.is_folder() {
            let folder_matched = parent_matched || filter.matches(&item.name);
            let children = item.item.take().unwrap_or_default();
            let pruned = prune_items(children, filter, folder_matched);
            if !pruned.is_empty() {
                item.item = Some(pruned);
                kept.push(item);
            }
        } else if item.is_request()
            && (parent_matched || filter.matches(&item.name))
            && filter.method_ok(&item)
        {
            kept.push(item);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_matches_everything() {
        let f = Filter::new(None).unwrap();
        assert!(f.matches("anything"));
        assert!(f.matches("auth/login"));
    }

    #[test]
    fn glob_matches_by_name() {
        let f = Filter::new(Some("auth*")).unwrap();
        assert!(f.matches("auth"));
        assert!(f.matches("authToken"));
        assert!(!f.matches("users"));
    }

    #[test]
    fn glob_with_wildcard_segment() {
        let f = Filter::new(Some("*login*")).unwrap();
        assert!(f.matches("user login flow"));
        assert!(!f.matches("logout"));
    }

    #[test]
    fn invalid_glob_is_error() {
        assert!(Filter::new(Some("[")).is_err());
    }

    #[test]
    fn prune_keeps_matching_requests_and_their_folders() {
        use golden_core::model::Collection;
        let json = r#"{
          "info": {"name":"C"},
          "item": [
            {"name":"auth","item":[
              {"name":"login","request":{"method":"GET","url":"x"}},
              {"name":"logout","request":{"method":"GET","url":"x"}}
            ]},
            {"name":"users","item":[
              {"name":"list","request":{"method":"GET","url":"x"}}
            ]}
          ]
        }"#;
        let mut c: Collection = serde_json::from_str(json).unwrap();
        let f = super::Filter::new(Some("login")).unwrap();
        super::prune_collection(&mut c, &f);
        // only "auth" folder survives, containing only "login"
        assert_eq!(c.item.len(), 1);
        assert_eq!(c.item[0].name, "auth");
        let kids = c.item[0].item.as_ref().unwrap();
        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0].name, "login");
    }

    #[test]
    fn prune_keeps_all_requests_when_folder_name_matches() {
        use golden_core::model::Collection;
        let json = r#"{
          "info": {"name":"C"},
          "item": [
            {"name":"auth","item":[
              {"name":"login","request":{"method":"GET","url":"x"}},
              {"name":"logout","request":{"method":"GET","url":"x"}}
            ]}
          ]
        }"#;
        let mut c: Collection = serde_json::from_str(json).unwrap();
        let f = super::Filter::new(Some("auth")).unwrap();
        super::prune_collection(&mut c, &f);
        let kids = c.item[0].item.as_ref().unwrap();
        assert_eq!(kids.len(), 2);
    }

    #[test]
    fn method_filter_keeps_only_matching_verbs() {
        use golden_core::model::Collection;
        let json = r#"{
          "info": {"name":"C"},
          "item": [
            {"name":"read one","request":{"method":"GET","url":"x"}},
            {"name":"make one","request":{"method":"post","url":"x"}}
          ]
        }"#;
        let mut c: Collection = serde_json::from_str(json).unwrap();
        // case-insensitive; composes with a pass-through name filter
        let f = Filter::new(None)
            .unwrap()
            .with_methods(&["get".to_string()]);
        prune_collection(&mut c, &f);
        assert_eq!(c.item.len(), 1);
        assert_eq!(c.item[0].name, "read one");
    }

    #[test]
    fn empty_methods_is_no_restriction() {
        let f = Filter::new(None).unwrap().with_methods(&[]);
        let item: golden_core::model::Item =
            serde_json::from_str(r#"{"name":"r","request":{"method":"DELETE","url":"x"}}"#)
                .unwrap();
        assert!(f.method_ok(&item));
    }
}
