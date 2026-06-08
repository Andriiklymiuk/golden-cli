//! --filter glob matching over request/folder names. Backed by globset, with
//! literal-substring fallback semantics provided by leading/trailing `*`.

use globset::{Glob, GlobMatcher};

/// A compiled name filter. `None` matches everything.
pub struct Filter {
    matcher: Option<GlobMatcher>,
}

impl Filter {
    /// Compile a glob, or build a pass-through filter when `pattern` is None.
    pub fn new(pattern: Option<&str>) -> Result<Self, String> {
        match pattern {
            None => Ok(Filter { matcher: None }),
            Some(p) => {
                let glob = Glob::new(p).map_err(|e| e.to_string())?;
                Ok(Filter {
                    matcher: Some(glob.compile_matcher()),
                })
            }
        }
    }

    /// Does `name` pass the filter?
    pub fn matches(&self, name: &str) -> bool {
        match &self.matcher {
            None => true,
            Some(m) => m.is_match(name),
        }
    }
}

use golden_core::model::{Collection, Item};

/// Retain only requests whose name matches the filter; keep a folder only if it
/// still has matching descendants. A request also stays if its parent folder
/// name matches (folder-level filter). Mutates the collection's item tree.
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
        } else if item.is_request() && (parent_matched || filter.matches(&item.name)) {
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
}
