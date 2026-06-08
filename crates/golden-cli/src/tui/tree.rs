//! Flatten loaded collections into a single navigable list of rows the tree
//! pane renders, plus collapse/expand and navigation logic.

use golden_core::model::Item;

use super::loader::LoadedCollection;

/// What a visible row points at, addressed by index path from the roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    /// A whole collection file (top-level, always a folder-like header).
    Collection,
    /// A folder item (has children).
    Folder,
    /// A request item (sendable).
    Request,
}

/// One visible row in the flattened tree.
#[derive(Debug, Clone)]
pub struct TreeRow {
    /// 0 for collection headers, +1 per nesting level.
    pub depth: usize,
    pub name: String,
    /// HTTP method for requests (e.g. "GET"); None for folders/collections.
    pub method: Option<String>,
    pub kind: NodeKind,
    /// Index path: `[collection_index, child_index, …]` to address the node.
    pub path: Vec<usize>,
    /// True if this node has children (collection or folder).
    pub has_children: bool,
}

/// Build the visible rows from collections, honouring the `collapsed` set
/// (paths whose children are hidden).
pub fn flatten(collections: &[LoadedCollection], collapsed: &[Vec<usize>]) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    for (ci, lc) in collections.iter().enumerate() {
        let path = vec![ci];
        let has_children = !lc.collection.item.is_empty();
        rows.push(TreeRow {
            depth: 0,
            name: lc.collection.info.name.clone(),
            method: None,
            kind: NodeKind::Collection,
            path: path.clone(),
            has_children,
        });
        if has_children && !is_collapsed(collapsed, &path) {
            walk(&lc.collection.item, &path, 1, collapsed, &mut rows);
        }
    }
    rows
}

fn walk(
    items: &[Item],
    parent: &[usize],
    depth: usize,
    collapsed: &[Vec<usize>],
    rows: &mut Vec<TreeRow>,
) {
    for (i, item) in items.iter().enumerate() {
        let mut path = parent.to_vec();
        path.push(i);
        if item.is_folder() {
            let has_children = item.item.as_ref().map(|c| !c.is_empty()).unwrap_or(false);
            rows.push(TreeRow {
                depth,
                name: item.name.clone(),
                method: None,
                kind: NodeKind::Folder,
                path: path.clone(),
                has_children,
            });
            if has_children && !is_collapsed(collapsed, &path) {
                walk(
                    item.item.as_ref().unwrap(),
                    &path,
                    depth + 1,
                    collapsed,
                    rows,
                );
            }
        } else if item.is_request() {
            let method = item.request.as_ref().map(|r| r.method.clone());
            rows.push(TreeRow {
                depth,
                name: item.name.clone(),
                method,
                kind: NodeKind::Request,
                path,
                has_children: false,
            });
        }
    }
}

fn is_collapsed(collapsed: &[Vec<usize>], path: &[usize]) -> bool {
    collapsed.iter().any(|c| c.as_slice() == path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use golden_core::model::Collection;

    fn lc(json: &str) -> LoadedCollection {
        LoadedCollection {
            path: "/tmp/x.json".into(),
            collection: serde_json::from_str::<Collection>(json).unwrap(),
        }
    }

    const NESTED: &str = r#"{
      "info": { "name": "Sample" },
      "item": [
        { "name": "auth", "item": [
          { "name": "login", "request": { "method": "POST", "url": "{{base}}/login" } }
        ]},
        { "name": "ping", "request": { "method": "GET", "url": "{{base}}/ping" } }
      ]
    }"#;

    #[test]
    fn flattens_depth_first_with_methods_and_paths() {
        let rows = flatten(&[lc(NESTED)], &[]);
        // collection header, auth folder, login request, ping request
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].kind, NodeKind::Collection);
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].name, "auth");
        assert_eq!(rows[1].kind, NodeKind::Folder);
        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[2].name, "login");
        assert_eq!(rows[2].method.as_deref(), Some("POST"));
        assert_eq!(rows[2].path, vec![0, 0, 0]);
        assert_eq!(rows[3].name, "ping");
        assert_eq!(rows[3].method.as_deref(), Some("GET"));
        assert_eq!(rows[3].path, vec![0, 1]);
    }

    #[test]
    fn collapsing_a_folder_hides_its_children() {
        let rows = flatten(&[lc(NESTED)], &[vec![0, 0]]);
        // auth collapsed -> login hidden; collection + auth + ping remain
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.name != "login"));
    }
}
