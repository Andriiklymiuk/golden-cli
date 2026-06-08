//! The single source of truth for the TUI: loaded data, selection, focus,
//! modes, and in-flight response state.

use std::path::PathBuf;

use golden_core::env::VarScopes;

use super::edit::{EditField, EditSession};
use super::loader::LoadedCollection;
use super::run_state::RunState;
use super::tree::{flatten, NodeKind, TreeRow};

/// Which pane has focus (tab cycles through them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Tree,
    Request,
    Response,
}

/// Modal overlay currently active, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Help,
    EnvSwitch,
    Search,
    Run,
    /// A single-field inline editor is open (stored in `App::edit`).
    Edit,
    /// A single-line tree-CRUD prompt is open (stored in `App::prompt`).
    Prompt,
    /// Waiting for y/n confirmation before a destructive op (stored in `App::confirm`).
    Confirm,
    /// Collection picker for cross-collection item move (j/k to navigate, Enter to confirm).
    MoveTarget,
}

/// What tree-CRUD operation a prompt is driving.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptOp {
    /// Add a new request — carries (collection_index, parent_item_path, default_method).
    AddRequest { ci: usize, parent: Vec<usize> },
    /// Add a new folder — carries (collection_index, parent_item_path).
    AddFolder { ci: usize, parent: Vec<usize> },
    /// Rename an existing item by its current name — carries (ci, old_name).
    Rename { ci: usize, old_name: String },
    /// Create a brand-new top-level collection in the given directory.
    CreateCollection { dir: std::path::PathBuf },
}

/// Active name-prompt session (drives `Mode::Prompt`).
#[derive(Debug, Clone)]
pub struct PromptSession {
    pub op: PromptOp,
    /// Title shown in the overlay.
    pub title: String,
    /// Editable text buffer.
    pub buffer: String,
}

impl PromptSession {
    pub fn new(op: PromptOp, title: impl Into<String>) -> Self {
        PromptSession {
            op,
            title: title.into(),
            buffer: String::new(),
        }
    }
}

/// Pending destructive-op confirmation.
#[derive(Debug, Clone)]
pub struct ConfirmOp {
    /// Human-readable prompt shown in the status bar (e.g. "delete 'ping'? (y/n)").
    pub message: String,
    /// The action to take on 'y'.
    pub action: ConfirmAction,
}

/// Which action to execute on confirmation.
#[derive(Debug, Clone)]
pub enum ConfirmAction {
    /// Delete a request/folder by name from a collection file.
    DeleteItem { ci: usize, name: String },
    /// Delete a whole collection file.
    DeleteCollection { ci: usize },
    /// Duplicate a request/folder by name within a collection file.
    DuplicateItem { ci: usize, name: String },
}

/// Which sub-section of the request pane is "focused" for editing.
/// Cycling with `f` moves through Method → Url → Headers → Body → Scripts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestTab {
    Method,
    Url,
    Headers,
    Body,
    PreRequestScript,
    TestScript,
}

/// Sub-tabs in the response pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseTab {
    Body,
    Headers,
    Cookies,
    Tests,
}

/// Top-level application state.
pub struct App {
    pub collections: Vec<LoadedCollection>,
    pub collapsed: Vec<Vec<usize>>,
    /// Cached flattened rows (rebuilt whenever collections/collapsed change).
    pub rows: Vec<TreeRow>,
    /// Index into `rows` of the highlighted tree node.
    pub selected: usize,
    pub focus: Pane,
    pub mode: Mode,
    /// Directory collections were loaded from (for hot-reload).
    pub collections_dir: PathBuf,
    /// Resolved variables for substitution / display.
    pub scopes: VarScopes,
    pub should_quit: bool,
    /// Transient status line message (errors, hints).
    pub status: String,
    /// Result of the most recent send (None until first send).
    pub last_response: Option<golden_core::http::HttpResponse>,
    /// Error from the most recent send, if it failed.
    pub last_error: Option<String>,
    /// True while a send is in flight (drives the spinner + enables cancel).
    pub sending: bool,
    /// Which sub-tab of the response pane is active.
    pub response_tab: ResponseTab,
    /// Vertical scroll offset for the response body.
    pub response_scroll: u16,
    /// Live state for the run overlay (progress, results).
    pub run: RunState,
    /// Discovered env profiles (name, path).
    pub env_profiles: Vec<(String, PathBuf)>,
    /// Index of the currently selected env profile in `env_profiles`.
    pub env_selected: usize,
    /// Name of the currently active env profile.
    pub active_env: String,
    /// Current search query for the response filter.
    pub search_query: String,
    /// Active single-field editor session (Some while Mode::Edit is active).
    pub edit: Option<EditSession>,
    /// Active name-prompt session (Some while Mode::Prompt is active).
    pub prompt: Option<PromptSession>,
    /// Pending confirmation (Some while Mode::Confirm is active).
    pub confirm: Option<ConfirmOp>,
    /// Which request sub-field is focused (for `e` to know what to edit).
    pub request_tab: RequestTab,
    /// Index of the selected target collection in the MoveTarget picker.
    pub move_target_selected: usize,
}

impl App {
    pub fn new(
        collections_dir: PathBuf,
        collections: Vec<LoadedCollection>,
        scopes: VarScopes,
    ) -> Self {
        let mut app = App {
            collections,
            collapsed: Vec::new(),
            rows: Vec::new(),
            selected: 0,
            focus: Pane::Tree,
            mode: Mode::Normal,
            collections_dir,
            scopes,
            should_quit: false,
            status: String::new(),
            last_response: None,
            last_error: None,
            sending: false,
            response_tab: ResponseTab::Body,
            response_scroll: 0,
            run: RunState::default(),
            env_profiles: Vec::new(),
            env_selected: 0,
            active_env: "default".into(),
            search_query: String::new(),
            edit: None,
            prompt: None,
            confirm: None,
            request_tab: RequestTab::Method,
            move_target_selected: 0,
        };
        app.rebuild_rows();
        app
    }

    /// Recompute the flattened rows, clamping the selection into range.
    pub fn rebuild_rows(&mut self) {
        self.rows = flatten(&self.collections, &self.collapsed);
        if self.rows.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.rows.len() {
            self.selected = self.rows.len() - 1;
        }
    }

    pub fn select_next(&mut self) {
        if !self.rows.is_empty() {
            self.selected = (self.selected + 1).min(self.rows.len() - 1);
        }
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    pub fn select_last(&mut self) {
        if !self.rows.is_empty() {
            self.selected = self.rows.len() - 1;
        }
    }

    pub fn cycle_pane(&mut self) {
        self.focus = match self.focus {
            Pane::Tree => Pane::Request,
            Pane::Request => Pane::Response,
            Pane::Response => Pane::Tree,
        };
    }

    /// The currently highlighted row, if any.
    pub fn current_row(&self) -> Option<&TreeRow> {
        self.rows.get(self.selected)
    }

    /// Toggle collapse on the selected folder/collection (enter on a folder).
    pub fn toggle_collapse(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        if !row.has_children {
            return;
        }
        let path = row.path.clone();
        if let Some(pos) = self.collapsed.iter().position(|c| *c == path) {
            self.collapsed.remove(pos);
        } else {
            self.collapsed.push(path);
        }
        self.rebuild_rows();
    }

    /// Resolve the `Item` (folder or request) addressed by an index path.
    pub fn item_at(&self, path: &[usize]) -> Option<&golden_core::model::Item> {
        let (ci, rest) = path.split_first()?;
        let coll = &self.collections.get(*ci)?.collection;
        let mut items: &[golden_core::model::Item] = &coll.item;
        let mut node: Option<&golden_core::model::Item> = None;
        for &idx in rest {
            let item = items.get(idx)?;
            node = Some(item);
            items = item.item.as_deref().unwrap_or(&[]);
        }
        node
    }

    /// The request behind the current selection, if it is a request row.
    pub fn current_request(&self) -> Option<&golden_core::model::Request> {
        let row = self.current_row()?;
        if row.kind != NodeKind::Request {
            return None;
        }
        self.item_at(&row.path)?.request.as_ref()
    }

    /// Variables map for substitution (clone of resolved scopes).
    pub fn vars_map(&self) -> std::collections::HashMap<String, String> {
        self.scopes.as_map().clone()
    }

    /// Cycle the response sub-tab forward.
    pub fn next_response_tab(&mut self) {
        self.response_tab = match self.response_tab {
            ResponseTab::Body => ResponseTab::Headers,
            ResponseTab::Headers => ResponseTab::Cookies,
            ResponseTab::Cookies => ResponseTab::Tests,
            ResponseTab::Tests => ResponseTab::Body,
        };
    }

    /// Load the list of selectable env profiles from the workspace dir.
    pub fn refresh_env_profiles(&mut self, workspace: &std::path::Path) {
        self.env_profiles = super::loader::discover_env_profiles(workspace);
    }

    /// Cycle the focused request sub-tab forward (f key).
    pub fn next_request_tab(&mut self) {
        self.request_tab = match self.request_tab {
            RequestTab::Method => RequestTab::Url,
            RequestTab::Url => RequestTab::Headers,
            RequestTab::Headers => RequestTab::Body,
            RequestTab::Body => RequestTab::PreRequestScript,
            RequestTab::PreRequestScript => RequestTab::TestScript,
            RequestTab::TestScript => RequestTab::Method,
        };
    }

    /// The collection index + item-tree path for the currently selected request,
    /// or None if the selection is not a request row.
    ///
    /// The returned path is split into `(collection_index, item_path)`:
    /// - `collection_index` addresses `self.collections[collection_index]`
    /// - `item_path` is passed to `store::set_*` (indexes into `collection.item`)
    pub fn selected_request_path(&self) -> Option<(usize, Vec<usize>)> {
        let row = self.current_row()?;
        if row.kind != NodeKind::Request {
            return None;
        }
        // row.path == [collection_index, rest...]
        let (ci, rest) = row.path.split_first()?;
        Some((*ci, rest.to_vec()))
    }

    /// Open an edit session for the currently focused field of the selected request.
    /// Returns false if no request is selected.
    pub fn open_edit(&mut self) -> bool {
        let (ci, item_path) = match self.selected_request_path() {
            Some(v) => v,
            None => return false,
        };
        let coll = match self.collections.get(ci) {
            Some(c) => &c.collection,
            None => return false,
        };
        let field = match self.request_tab {
            RequestTab::Method => EditField::Method,
            RequestTab::Url => EditField::Url,
            RequestTab::Headers => EditField::HeadersJson,
            RequestTab::Body => EditField::BodyRaw,
            RequestTab::PreRequestScript => EditField::PreRequestScript,
            RequestTab::TestScript => EditField::TestScript,
        };
        let initial = super::edit::initial_text_for(coll, &item_path, &field);
        self.edit = Some(EditSession::new(field, item_path, initial));
        self.mode = Mode::Edit;
        true
    }

    /// Commit the current edit session to the in-memory collection and persist it.
    /// Clears `self.edit` and returns to Normal mode.  Surfaces errors in `self.status`.
    pub fn commit_edit(&mut self) {
        let session = match self.edit.take() {
            Some(s) => s,
            None => return,
        };
        // Resolve which collection owns this session's path.
        // `selected_request_path` returns (ci, item_path) — but we stored only
        // the item_path in the session.  Recover ci from the current selection.
        let ci = match self.current_row().and_then(|r| r.path.first().copied()) {
            Some(c) => c,
            None => {
                self.status = "edit: no collection selected".into();
                self.mode = Mode::Normal;
                return;
            }
        };
        let lc = match self.collections.get_mut(ci) {
            Some(c) => c,
            None => {
                self.status = "edit: collection not found".into();
                self.mode = Mode::Normal;
                return;
            }
        };
        match session.commit(&mut lc.collection) {
            Ok(()) => {
                let path = lc.path.clone();
                match golden_core::store::save_collection(&path, &lc.collection) {
                    Ok(()) => self.status = "saved".into(),
                    Err(e) => self.status = format!("save failed: {e}"),
                }
            }
            Err(e) => self.status = format!("edit failed: {e}"),
        }
        self.mode = Mode::Normal;
    }

    // ── tree CRUD helpers ──────────────────────────────────────────────────

    /// Reload a single collection from disk (by collection index) and rebuild rows.
    /// Surfaces errors in `self.status`.
    pub fn reload_collection(&mut self, ci: usize) {
        let path = match self.collections.get(ci) {
            Some(lc) => lc.path.clone(),
            None => {
                self.status = format!("reload: no collection at index {ci}");
                return;
            }
        };
        match golden_core::store::load_collection(&path) {
            Ok(coll) => {
                self.collections[ci].collection = coll;
                self.rebuild_rows();
            }
            Err(e) => self.status = format!("reload failed: {e}"),
        }
    }

    /// Open a name-prompt for adding a request under the currently selected node.
    /// Returns false if no valid context.
    pub fn open_add_request_prompt(&mut self) -> bool {
        let row = match self.current_row() {
            Some(r) => r.clone(),
            None => return false,
        };
        let (ci, parent) = match row.kind {
            super::tree::NodeKind::Collection => {
                let ci = row.path[0];
                (ci, vec![])
            }
            super::tree::NodeKind::Folder => {
                let ci = row.path[0];
                let parent = row.path[1..].to_vec();
                (ci, parent)
            }
            super::tree::NodeKind::Request => {
                // Add under the same parent folder as this request.
                let ci = row.path[0];
                let parent = if row.path.len() > 2 {
                    row.path[1..row.path.len() - 1].to_vec()
                } else {
                    vec![]
                };
                (ci, parent)
            }
        };
        self.prompt = Some(PromptSession::new(
            PromptOp::AddRequest { ci, parent },
            "Add request (name)",
        ));
        self.mode = Mode::Prompt;
        true
    }

    /// Open a name-prompt for adding a folder under the currently selected node.
    /// Returns false if no valid context.
    pub fn open_add_folder_prompt(&mut self) -> bool {
        let row = match self.current_row() {
            Some(r) => r.clone(),
            None => return false,
        };
        let (ci, parent) = match row.kind {
            super::tree::NodeKind::Collection => (row.path[0], vec![]),
            super::tree::NodeKind::Folder => {
                let ci = row.path[0];
                let parent = row.path[1..].to_vec();
                (ci, parent)
            }
            super::tree::NodeKind::Request => {
                let ci = row.path[0];
                let parent = if row.path.len() > 2 {
                    row.path[1..row.path.len() - 1].to_vec()
                } else {
                    vec![]
                };
                (ci, parent)
            }
        };
        self.prompt = Some(PromptSession::new(
            PromptOp::AddFolder { ci, parent },
            "Add folder (name)",
        ));
        self.mode = Mode::Prompt;
        true
    }

    /// Open a rename prompt for the currently selected item.
    /// Returns false if no valid context.
    pub fn open_rename_prompt(&mut self) -> bool {
        let row = match self.current_row() {
            Some(r) => r.clone(),
            None => return false,
        };
        let ci = row.path[0];
        let old_name = row.name.clone();
        let mut sess = PromptSession::new(
            PromptOp::Rename {
                ci,
                old_name: old_name.clone(),
            },
            "Rename (new name)",
        );
        // Pre-fill with the current name.
        sess.buffer = old_name;
        self.prompt = Some(sess);
        self.mode = Mode::Prompt;
        true
    }

    /// Start a delete confirmation for the currently selected item.
    /// Returns false if nothing is selected.
    pub fn start_delete_confirm(&mut self) -> bool {
        let row = match self.current_row() {
            Some(r) => r.clone(),
            None => return false,
        };
        let ci = row.path[0];
        let name = row.name.clone();
        let (message, action) = match row.kind {
            super::tree::NodeKind::Collection => (
                format!("delete collection '{name}'? (y/n)"),
                ConfirmAction::DeleteCollection { ci },
            ),
            _ => (
                format!("delete '{name}'? (y/n)"),
                ConfirmAction::DeleteItem {
                    ci,
                    name: name.clone(),
                },
            ),
        };
        self.confirm = Some(ConfirmOp { message, action });
        self.mode = Mode::Confirm;
        true
    }

    /// Start a duplicate for the currently selected item (no confirmation needed).
    /// Returns false if nothing is selected.
    pub fn start_duplicate(&mut self) -> bool {
        let row = match self.current_row() {
            Some(r) => r.clone(),
            None => return false,
        };
        let ci = row.path[0];
        let name = row.name.clone();
        match row.kind {
            super::tree::NodeKind::Collection => {
                // Duplicate the whole collection file.
                let path = match self.collections.get(ci) {
                    Some(lc) => lc.path.clone(),
                    None => return false,
                };
                match golden_core::store::duplicate_collection_file(&path) {
                    Ok(_new_path) => {
                        self.status = format!("duplicated '{name}'");
                        // The new file will be picked up by the watcher;
                        // for tests / immediate reload we add it ourselves.
                        // We don't reload here — the watcher handles it in the real loop.
                    }
                    Err(e) => self.status = format!("duplicate failed: {e}"),
                }
            }
            _ => {
                // Duplicate item within collection.
                let lc = match self.collections.get_mut(ci) {
                    Some(c) => c,
                    None => return false,
                };
                match golden_core::store::duplicate_item_by_name(&mut lc.collection.item, &name) {
                    Ok(()) => {
                        let path = lc.path.clone();
                        let coll = &lc.collection;
                        match golden_core::store::save_collection(&path, coll) {
                            Ok(()) => {
                                self.status = format!("duplicated '{name}'");
                            }
                            Err(e) => self.status = format!("save failed: {e}"),
                        }
                        self.reload_collection(ci);
                    }
                    Err(e) => self.status = format!("duplicate failed: {e}"),
                }
            }
        }
        true
    }

    /// Execute a confirmed destructive action.  Called when the user presses `y`.
    pub fn execute_confirm(&mut self) {
        let op = match self.confirm.take() {
            Some(c) => c.action,
            None => return,
        };
        self.mode = Mode::Normal;
        match op {
            ConfirmAction::DeleteItem { ci, name } => {
                let lc = match self.collections.get_mut(ci) {
                    Some(c) => c,
                    None => {
                        self.status = "delete: collection not found".into();
                        return;
                    }
                };
                if golden_core::store::delete_item_by_name(&mut lc.collection.item, &name) {
                    let path = lc.path.clone();
                    let coll = &lc.collection;
                    match golden_core::store::save_collection(&path, coll) {
                        Ok(()) => self.status = format!("deleted '{name}'"),
                        Err(e) => self.status = format!("save failed: {e}"),
                    }
                    self.reload_collection(ci);
                } else {
                    self.status = format!("'{name}' not found");
                }
            }
            ConfirmAction::DeleteCollection { ci } => {
                let path = match self.collections.get(ci) {
                    Some(lc) => lc.path.clone(),
                    None => {
                        self.status = "delete: collection not found".into();
                        return;
                    }
                };
                let name = self.collections[ci].collection.info.name.clone();
                match golden_core::store::delete_collection_file(&path) {
                    Ok(()) => {
                        self.collections.remove(ci);
                        self.rebuild_rows();
                        self.status = format!("deleted collection '{name}'");
                    }
                    Err(e) => self.status = format!("delete failed: {e}"),
                }
            }
            ConfirmAction::DuplicateItem { ci, name } => {
                let lc = match self.collections.get_mut(ci) {
                    Some(c) => c,
                    None => return,
                };
                match golden_core::store::duplicate_item_by_name(&mut lc.collection.item, &name) {
                    Ok(()) => {
                        let path = lc.path.clone();
                        let coll = &lc.collection;
                        match golden_core::store::save_collection(&path, coll) {
                            Ok(()) => self.status = format!("duplicated '{name}'"),
                            Err(e) => self.status = format!("save failed: {e}"),
                        }
                        self.reload_collection(ci);
                    }
                    Err(e) => self.status = format!("duplicate failed: {e}"),
                }
            }
        }
    }

    /// Commit the current prompt session (called on Enter in Prompt mode).
    pub fn commit_prompt(&mut self) {
        let sess = match self.prompt.take() {
            Some(s) => s,
            None => {
                self.mode = Mode::Normal;
                return;
            }
        };
        self.mode = Mode::Normal;
        let name = sess.buffer.trim().to_string();
        if name.is_empty() {
            self.status = "name cannot be empty".into();
            return;
        }
        match sess.op {
            PromptOp::AddRequest { ci, parent } => {
                let lc = match self.collections.get_mut(ci) {
                    Some(c) => c,
                    None => {
                        self.status = "add: collection not found".into();
                        return;
                    }
                };
                match golden_core::store::add_request(
                    &mut lc.collection.item,
                    &parent,
                    &name,
                    "GET",
                ) {
                    Ok(()) => {
                        let path = lc.path.clone();
                        let coll = &lc.collection;
                        match golden_core::store::save_collection(&path, coll) {
                            Ok(()) => self.status = format!("added request '{name}'"),
                            Err(e) => self.status = format!("save failed: {e}"),
                        }
                        self.reload_collection(ci);
                    }
                    Err(e) => self.status = format!("add failed: {e}"),
                }
            }
            PromptOp::AddFolder { ci, parent } => {
                let lc = match self.collections.get_mut(ci) {
                    Some(c) => c,
                    None => {
                        self.status = "add: collection not found".into();
                        return;
                    }
                };
                match golden_core::store::add_folder(&mut lc.collection.item, &parent, &name) {
                    Ok(()) => {
                        let path = lc.path.clone();
                        let coll = &lc.collection;
                        match golden_core::store::save_collection(&path, coll) {
                            Ok(()) => self.status = format!("added folder '{name}'"),
                            Err(e) => self.status = format!("save failed: {e}"),
                        }
                        self.reload_collection(ci);
                    }
                    Err(e) => self.status = format!("add failed: {e}"),
                }
            }
            PromptOp::Rename { ci, old_name } => {
                let lc = match self.collections.get_mut(ci) {
                    Some(c) => c,
                    None => {
                        self.status = "rename: collection not found".into();
                        return;
                    }
                };
                // Check if this is a collection-level rename (row is a Collection node).
                // We detect this by checking if `ci`'s collection name == old_name.
                if lc.collection.info.name == old_name {
                    // Rename the collection file itself.
                    let old_path = lc.path.clone();
                    match golden_core::store::rename_collection(&old_path, &name) {
                        Ok(new_path) => {
                            self.collections[ci].path = new_path;
                            self.collections[ci].collection.info.name = name.clone();
                            self.rebuild_rows();
                            self.status = format!("renamed to '{name}'");
                        }
                        Err(e) => self.status = format!("rename failed: {e}"),
                    }
                } else {
                    if golden_core::store::rename_item_by_name(
                        &mut lc.collection.item,
                        &old_name,
                        &name,
                    ) {
                        let path = lc.path.clone();
                        let coll = &lc.collection;
                        match golden_core::store::save_collection(&path, coll) {
                            Ok(()) => self.status = format!("renamed to '{name}'"),
                            Err(e) => self.status = format!("save failed: {e}"),
                        }
                        self.reload_collection(ci);
                    } else {
                        self.status = format!("'{old_name}' not found");
                    }
                }
            }
            PromptOp::CreateCollection { dir } => {
                match golden_core::store::create_collection(&dir, &name) {
                    Ok(path) => {
                        // Load and add the new collection.
                        match golden_core::store::load_collection(&path) {
                            Ok(coll) => {
                                self.collections.push(super::loader::LoadedCollection {
                                    path,
                                    collection: coll,
                                });
                                self.rebuild_rows();
                                self.status = format!("created collection '{name}'");
                            }
                            Err(e) => self.status = format!("load failed: {e}"),
                        }
                    }
                    Err(e) => self.status = format!("create failed: {e}"),
                }
            }
        }
    }

    /// Apply the selected profile: parse it, overlay onto re-resolved scopes,
    /// and record the active name.
    pub fn apply_selected_env(&mut self, workspace: &std::path::Path) {
        let Some((name, path)) = self.env_profiles.get(self.env_selected).cloned() else {
            return;
        };
        let coll_vars: Vec<golden_core::model::Variable> = self
            .collections
            .first()
            .map(|c| c.collection.variable.clone())
            .unwrap_or_default();
        // Base resolution (workspace .env + collection vars), then overlay the
        // chosen profile so named profiles win.
        let mut scopes = golden_core::env::resolve(workspace, &self.collections_dir, &coll_vars);
        if let Ok(content) = std::fs::read_to_string(&path) {
            for (k, v) in golden_core::env::parse_env(&content) {
                if !v.is_empty() {
                    scopes.set(k, v);
                }
            }
        }
        self.scopes = scopes;
        self.active_env = name;
    }

    // ── move-to-collection picker (Task 14) ───────────────────────────────

    /// Open the MoveTarget picker: lets the user choose a destination collection
    /// for the currently selected item.  Rejects collection-header selections.
    /// Returns false if nothing is selected or the selection is a collection.
    pub fn open_move_prompt(&mut self) -> bool {
        let row = match self.current_row() {
            Some(r) => r.clone(),
            None => {
                self.status = "nothing selected".into();
                return false;
            }
        };
        if row.kind == NodeKind::Collection {
            self.status = "cannot move a collection — select a request or folder".into();
            return false;
        }
        if self.collections.len() < 2 {
            self.status = "need at least two collections to move between".into();
            return false;
        }
        let ci = row.path[0];
        // Pre-select the first collection that is NOT the source.
        self.move_target_selected = if ci == 0 { 1 } else { 0 };
        self.mode = Mode::MoveTarget;
        true
    }

    // ── reorder + move helpers (Task 14) ──────────────────────────────────

    /// Move the currently selected item one position down within its container.
    /// For a top-level collection row, delegates to `reorder_root_collection`.
    /// No-op if the item is already last or nothing is selected.
    pub fn reorder_down(&mut self) {
        self.reorder_offset(1);
    }

    /// Move the currently selected item one position up within its container.
    /// For a top-level collection row, delegates to `reorder_root_collection`.
    /// No-op if the item is already first or nothing is selected.
    pub fn reorder_up(&mut self) {
        self.reorder_offset(-1);
    }

    fn reorder_offset(&mut self, delta: i64) {
        let row = match self.current_row() {
            Some(r) => r.clone(),
            None => return,
        };
        let ci = row.path[0];

        match row.kind {
            NodeKind::Collection => {
                // Reorder at the root level using numeric file prefixes.
                let lc = match self.collections.get(ci) {
                    Some(c) => c,
                    None => return,
                };
                let path = lc.path.clone();
                let dir = match path.parent() {
                    Some(d) => d.to_path_buf(),
                    None => return,
                };
                let fname = match path.file_name() {
                    Some(f) => f.to_string_lossy().into_owned(),
                    None => return,
                };
                let new_idx = (ci as i64) + delta;
                if new_idx < 0 || new_idx as usize >= self.collections.len() {
                    return; // already at boundary
                }
                match golden_core::store::reorder_root_collection(&dir, &fname, new_idx as usize) {
                    Ok(()) => {
                        // Update paths in-memory for the swapped collections.
                        // Reload from disk to pick up the renamed files.
                        self.reload_collections_after_reorder(&dir);
                        let new_sel =
                            (new_idx as usize).min(self.collections.len().saturating_sub(1));
                        self.selected = new_sel;
                        self.rebuild_rows();
                        self.status = "reordered".into();
                    }
                    Err(e) => self.status = format!("reorder failed: {e}"),
                }
            }
            NodeKind::Request | NodeKind::Folder => {
                // row.path == [ci, ...container_path, pos]
                let rest = &row.path[1..];
                if rest.is_empty() {
                    return;
                }
                let pos = *rest.last().unwrap();
                let container_path = &rest[..rest.len() - 1];
                let new_pos = (pos as i64) + delta;
                if new_pos < 0 {
                    return; // already at top
                }
                let new_pos = new_pos as usize;

                let lc = match self.collections.get_mut(ci) {
                    Some(c) => c,
                    None => return,
                };
                // Err means out-of-range — silently ignore (already at boundary).
                if golden_core::store::move_item_in_container(
                    &mut lc.collection.item,
                    container_path,
                    pos,
                    new_pos,
                )
                .is_ok()
                {
                    let path = lc.path.clone();
                    match golden_core::store::save_collection(&path, &lc.collection) {
                        Ok(()) => self.status = "reordered".into(),
                        Err(e) => {
                            self.status = format!("save failed: {e}");
                            // reload to discard the in-memory change
                            self.reload_collection(ci);
                            return;
                        }
                    }
                    self.reload_collection(ci);
                    // Try to keep the moved item selected: find the row with the same name.
                    let name = row.name.clone();
                    if let Some(idx) = self.rows.iter().position(|r| r.name == name) {
                        self.selected = idx;
                    }
                }
            }
        }
    }

    /// Reload all collection paths under `dir` (called after a root reorder
    /// which renames files with numeric prefixes).
    fn reload_collections_after_reorder(&mut self, dir: &std::path::Path) {
        use std::fs;
        // Gather the new file names (sorted, as the loader does).
        let mut json_files: Vec<std::path::PathBuf> = fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
                    .collect()
            })
            .unwrap_or_default();
        json_files.sort();
        let mut new_collections = Vec::new();
        for p in json_files {
            match golden_core::store::load_collection(&p) {
                Ok(coll) => new_collections.push(LoadedCollection {
                    path: p,
                    collection: coll,
                }),
                Err(e) => self.status = format!("reload failed: {e}"),
            }
        }
        self.collections = new_collections;
        self.rebuild_rows();
    }

    /// Move the currently selected request or folder to `target_ci` (another
    /// collection in `self.collections`), appending it at the root level.
    /// No-op if the selection is a collection header or the target is the same.
    pub fn move_to_collection(&mut self, target_ci: usize) {
        let row = match self.current_row() {
            Some(r) => r.clone(),
            None => {
                self.status = "nothing selected".into();
                return;
            }
        };
        let ci = row.path[0];
        if row.kind == NodeKind::Collection {
            self.status = "cannot move a collection to another collection".into();
            return;
        }
        if ci == target_ci {
            self.status = "source and target are the same collection".into();
            return;
        }
        let src_path = match self.collections.get(ci) {
            Some(lc) => lc.path.clone(),
            None => return,
        };
        let dst_path = match self.collections.get(target_ci) {
            Some(lc) => lc.path.clone(),
            None => {
                self.status = format!("target collection {target_ci} not found");
                return;
            }
        };
        let item_name = row.name.clone();
        match golden_core::store::move_item_across_collections(
            &src_path, &item_name, &dst_path, None,
        ) {
            Ok(()) => {
                self.status = format!("moved '{item_name}' to collection {target_ci}");
                self.reload_collection(ci);
                self.reload_collection(target_ci);
            }
            Err(e) => self.status = format!("move failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golden_core::model::Collection;

    fn app_with(json: &str) -> App {
        let lc = LoadedCollection {
            path: "/tmp/x.json".into(),
            collection: serde_json::from_str::<Collection>(json).unwrap(),
        };
        App::new("/tmp".into(), vec![lc], VarScopes::default())
    }

    #[test]
    fn apply_selected_env_overlays_profile() {
        use std::fs;
        use tempfile::tempdir;
        let ws = tempdir().unwrap();
        let coll_dir = ws.path().join("collections");
        fs::create_dir_all(&coll_dir).unwrap();
        fs::write(ws.path().join(".env"), "HOST=base").unwrap();
        fs::write(ws.path().join(".env.staging"), "HOST=staging").unwrap();

        let lc = LoadedCollection {
            path: coll_dir.join("x.json"),
            collection: serde_json::from_str::<Collection>(J).unwrap(),
        };
        let mut app = App::new(coll_dir, vec![lc], VarScopes::default());
        app.refresh_env_profiles(ws.path());
        // profiles: ["default", "staging"]; select staging (index 1)
        app.env_selected = 1;
        app.apply_selected_env(ws.path());
        assert_eq!(app.active_env, "staging");
        assert_eq!(app.scopes.get("HOST").map(String::as_str), Some("staging"));
    }

    const J: &str = r#"{
      "info": { "name": "Sample" },
      "item": [
        { "name": "auth", "item": [
          { "name": "login", "request": { "method": "POST", "url": "{{base}}/login" } }
        ]},
        { "name": "ping", "request": { "method": "GET", "url": "{{base}}/ping" } }
      ]
    }"#;

    #[test]
    fn nav_moves_within_bounds() {
        let mut app = app_with(J);
        assert_eq!(app.selected, 0);
        app.select_prev(); // clamps at 0
        assert_eq!(app.selected, 0);
        app.select_last();
        assert_eq!(app.selected, app.rows.len() - 1);
        app.select_next(); // clamps at last
        assert_eq!(app.selected, app.rows.len() - 1);
        app.select_first();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn current_request_resolves_for_request_row() {
        let mut app = app_with(J);
        // row 2 is login (POST)
        app.selected = 2;
        let req = app.current_request().expect("login is a request");
        assert_eq!(req.method, "POST");
        assert_eq!(req.url.raw(), "{{base}}/login");
        // selecting a folder yields no request
        app.selected = 1; // auth folder
        assert!(app.current_request().is_none());
    }

    #[test]
    fn toggle_collapse_hides_and_reshows_children() {
        let mut app = app_with(J);
        app.selected = 1; // auth folder
        app.toggle_collapse();
        assert!(app.rows.iter().all(|r| r.name != "login"));
        app.toggle_collapse();
        assert!(app.rows.iter().any(|r| r.name == "login"));
    }

    #[test]
    fn response_tab_cycles() {
        let mut app = app_with(J);
        assert_eq!(app.response_tab, ResponseTab::Body);
        app.next_response_tab();
        assert_eq!(app.response_tab, ResponseTab::Headers);
        app.next_response_tab();
        app.next_response_tab();
        assert_eq!(app.response_tab, ResponseTab::Tests);
        app.next_response_tab();
        assert_eq!(app.response_tab, ResponseTab::Body);
    }
}

// ── tree_op_tests: from plan Task 10 Step 1 ───────────────────────────────
#[cfg(test)]
mod tree_op_tests {
    use golden_core::store;
    use tempfile::tempdir;

    #[test]
    fn add_request_op_persists_to_file() {
        let dir = tempdir().unwrap();
        let path = store::create_collection(dir.path(), "C").unwrap();
        let mut coll = store::load_collection(&path).unwrap();
        // simulate "a" on the collection root: add request "ping"
        store::add_request(&mut coll.item, &[], "ping", "GET").unwrap();
        store::save_collection(&path, &coll).unwrap();
        let reloaded = store::load_collection(&path).unwrap();
        assert_eq!(reloaded.item[0].name, "ping");
    }

    #[test]
    fn delete_request_op_persists_to_file() {
        let dir = tempdir().unwrap();
        let path = store::create_collection(dir.path(), "C").unwrap();
        let mut coll = store::load_collection(&path).unwrap();
        store::add_request(&mut coll.item, &[], "ping", "GET").unwrap();
        store::save_collection(&path, &coll).unwrap();

        let mut coll = store::load_collection(&path).unwrap();
        assert!(store::delete_item_by_name(&mut coll.item, "ping"));
        store::save_collection(&path, &coll).unwrap();
        assert!(store::load_collection(&path).unwrap().item.is_empty());
    }
}

// ── crud_integration_tests: App-level CRUD via commit_prompt / execute_confirm ─
#[cfg(test)]
mod crud_integration_tests {
    use super::*;
    use golden_core::store;
    use tempfile::tempdir;

    /// Build an App backed by a real temp-dir collection file.
    fn app_with_file(json: &str) -> (App, tempfile::TempDir) {
        use std::fs;
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.json");
        fs::write(&path, json).unwrap();
        let lc = LoadedCollection {
            path,
            collection: serde_json::from_str::<golden_core::model::Collection>(json).unwrap(),
        };
        let app = App::new(dir.path().into(), vec![lc], VarScopes::default());
        (app, dir)
    }

    const J: &str = r#"{
      "info": { "name": "TestColl" },
      "item": [
        { "name": "ping", "request": { "method": "GET", "url": "https://x/ping" } }
      ]
    }"#;

    const J_EMPTY: &str = r#"{"info": {"name": "Empty"}, "item": []}"#;

    // ── add request ────────────────────────────────────────────────────────

    #[test]
    fn add_request_prompt_appears_on_collection_row() {
        let (mut app, _dir) = app_with_file(J);
        app.selected = 0; // collection row
        let ok = app.open_add_request_prompt();
        assert!(ok);
        assert_eq!(app.mode, Mode::Prompt);
        let sess = app.prompt.as_ref().unwrap();
        assert!(
            sess.title.contains("request"),
            "title should mention 'request'"
        );
    }

    #[test]
    fn add_request_commit_persists_new_row() {
        let (mut app, dir) = app_with_file(J_EMPTY);
        app.selected = 0; // collection row

        app.open_add_request_prompt();
        app.prompt.as_mut().unwrap().buffer = "health".into();
        app.commit_prompt();

        assert_eq!(app.mode, Mode::Normal);
        assert!(
            app.status.contains("added"),
            "status should say added, got: {}",
            app.status
        );
        // tree row must now include the new request
        assert!(
            app.rows.iter().any(|r| r.name == "health"),
            "health request should appear in tree"
        );
        // disk file must have it too
        let coll_path = dir.path().join("c.json");
        let reloaded = store::load_collection(&coll_path).unwrap();
        assert_eq!(reloaded.item[0].name, "health");
        assert_eq!(reloaded.item[0].request.as_ref().unwrap().method, "GET");
    }

    #[test]
    fn add_request_empty_name_is_rejected() {
        let (mut app, _dir) = app_with_file(J_EMPTY);
        app.selected = 0;
        app.open_add_request_prompt();
        // leave buffer empty
        app.commit_prompt();
        assert_eq!(app.mode, Mode::Normal);
        assert!(
            app.status.contains("empty"),
            "empty name should be rejected, got: {}",
            app.status
        );
        // no row added
        assert_eq!(app.collections[0].collection.item.len(), 0);
    }

    // ── add folder ─────────────────────────────────────────────────────────

    #[test]
    fn add_folder_commit_persists_new_folder() {
        let (mut app, dir) = app_with_file(J_EMPTY);
        app.selected = 0;

        app.open_add_folder_prompt();
        app.prompt.as_mut().unwrap().buffer = "auth".into();
        app.commit_prompt();

        assert!(app.status.contains("added"), "status: {}", app.status);
        assert!(
            app.rows.iter().any(|r| r.name == "auth"),
            "auth folder should appear in tree"
        );
        let coll_path = dir.path().join("c.json");
        let reloaded = store::load_collection(&coll_path).unwrap();
        assert_eq!(reloaded.item[0].name, "auth");
        assert!(reloaded.item[0].item.is_some(), "folder should have item[]");
    }

    // ── rename ─────────────────────────────────────────────────────────────

    #[test]
    fn rename_request_updates_in_memory_and_on_disk() {
        let (mut app, dir) = app_with_file(J);
        // row 1 = "ping" request
        app.selected = 1;

        app.open_rename_prompt();
        let sess = app.prompt.as_mut().unwrap();
        sess.buffer = "pong".into();
        app.commit_prompt();

        assert_eq!(app.mode, Mode::Normal);
        assert!(app.status.contains("renamed"), "status: {}", app.status);
        assert!(
            app.rows.iter().any(|r| r.name == "pong"),
            "pong should be in tree"
        );
        assert!(
            !app.rows.iter().any(|r| r.name == "ping"),
            "ping should be gone"
        );
        let coll_path = dir.path().join("c.json");
        let reloaded = store::load_collection(&coll_path).unwrap();
        assert_eq!(reloaded.item[0].name, "pong");
    }

    #[test]
    fn rename_prompt_is_prefilled_with_current_name() {
        let (mut app, _dir) = app_with_file(J);
        app.selected = 1; // "ping"
        app.open_rename_prompt();
        let sess = app.prompt.as_ref().unwrap();
        assert_eq!(
            sess.buffer, "ping",
            "rename prompt should be pre-filled with current name"
        );
    }

    // ── delete ─────────────────────────────────────────────────────────────

    #[test]
    fn delete_request_requires_confirm_then_persists() {
        let (mut app, dir) = app_with_file(J);
        app.selected = 1; // "ping"

        // Start delete — should enter Confirm mode.
        app.start_delete_confirm();
        assert_eq!(app.mode, Mode::Confirm);
        let msg = app.confirm.as_ref().unwrap().message.clone();
        assert!(
            msg.contains("ping"),
            "confirm message should name the item, got: {msg}"
        );

        // Execute (user presses y).
        app.execute_confirm();

        assert_eq!(app.mode, Mode::Normal);
        assert!(app.status.contains("deleted"), "status: {}", app.status);
        assert!(
            !app.rows.iter().any(|r| r.name == "ping"),
            "ping should be removed from tree"
        );
        let coll_path = dir.path().join("c.json");
        let reloaded = store::load_collection(&coll_path).unwrap();
        assert!(
            reloaded.item.is_empty(),
            "on-disk collection should be empty"
        );
    }

    #[test]
    fn delete_cancel_on_n_leaves_item_intact() {
        let (mut app, _dir) = app_with_file(J);
        app.selected = 1; // "ping"

        app.start_delete_confirm();
        assert_eq!(app.mode, Mode::Confirm);
        // Cancel the confirmation.
        app.confirm = None;
        app.status = "cancelled".into();
        app.mode = Mode::Normal;

        assert_eq!(app.mode, Mode::Normal);
        assert!(
            app.rows.iter().any(|r| r.name == "ping"),
            "ping should still be in tree after cancel"
        );
        assert_eq!(app.collections[0].collection.item.len(), 1);
    }

    // ── duplicate (copy) ────────────────────────────────────────────────────

    #[test]
    fn duplicate_request_creates_copy_row() {
        let (mut app, dir) = app_with_file(J);
        app.selected = 1; // "ping"

        app.start_duplicate();

        assert!(app.status.contains("duplicated"), "status: {}", app.status);
        assert!(
            app.rows.iter().any(|r| r.name == "ping (Copy)"),
            "ping (Copy) should appear in tree"
        );
        let coll_path = dir.path().join("c.json");
        let reloaded = store::load_collection(&coll_path).unwrap();
        assert_eq!(reloaded.item.len(), 2);
        assert_eq!(reloaded.item[1].name, "ping (Copy)");
    }

    // ── Esc cancels prompts ────────────────────────────────────────────────

    #[test]
    fn esc_in_prompt_mode_cancels_without_change() {
        let (mut app, _dir) = app_with_file(J_EMPTY);
        app.selected = 0;
        app.open_add_request_prompt();
        assert_eq!(app.mode, Mode::Prompt);
        app.prompt.as_mut().unwrap().buffer = "something".into();

        // Simulate Esc.
        app.prompt = None;
        app.mode = Mode::Normal;

        assert_eq!(app.mode, Mode::Normal);
        assert!(app.prompt.is_none());
        assert_eq!(app.collections[0].collection.item.len(), 0);
    }

    #[test]
    fn esc_in_confirm_mode_cancels_delete() {
        let (mut app, _dir) = app_with_file(J);
        app.selected = 1;
        app.start_delete_confirm();
        assert_eq!(app.mode, Mode::Confirm);

        app.confirm = None;
        app.mode = Mode::Normal;

        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.collections[0].collection.item.len(), 1);
    }
}

// ── move_reorder_op_tests: Task 14 — reorder ([/]) and move (m) ──────────────
#[cfg(test)]
mod move_reorder_op_tests {
    use super::*;
    use golden_core::store;
    use tempfile::tempdir;

    /// App backed by a real temp-dir collection file at path `dir/c.json`.
    fn app_with_file(dir: &tempfile::TempDir, json: &str) -> App {
        use std::fs;
        let path = dir.path().join("c.json");
        fs::write(&path, json).unwrap();
        let lc = LoadedCollection {
            path,
            collection: serde_json::from_str::<golden_core::model::Collection>(json).unwrap(),
        };
        App::new(dir.path().into(), vec![lc], VarScopes::default())
    }

    const J2: &str = r#"{
      "info": { "name": "C" },
      "item": [
        { "name": "a", "request": { "method": "GET", "url": "https://x/a" } },
        { "name": "b", "request": { "method": "GET", "url": "https://x/b" } }
      ]
    }"#;

    // ── reorder within collection: ] moves item down ────────────────────────

    #[test]
    fn reorder_down_moves_selected_item() {
        let dir = tempdir().unwrap();
        let mut app = app_with_file(&dir, J2);
        // row 0 = collection header, row 1 = "a", row 2 = "b"
        app.selected = 1; // "a" request, path=[0, 0]
        app.reorder_down();
        // "a" should now be at index 1 in the collection (after "b")
        let coll = store::load_collection(&app.collections[0].path).unwrap();
        assert_eq!(coll.item[0].name, "b");
        assert_eq!(coll.item[1].name, "a");
        // tree row for "a" should now be at row 2 (after "b")
        assert_eq!(app.rows[2].name, "a");
    }

    #[test]
    fn reorder_up_moves_selected_item() {
        let dir = tempdir().unwrap();
        let mut app = app_with_file(&dir, J2);
        // row 2 = "b"
        app.selected = 2; // "b" request, path=[0, 1]
        app.reorder_up();
        let coll = store::load_collection(&app.collections[0].path).unwrap();
        assert_eq!(coll.item[0].name, "b");
        assert_eq!(coll.item[1].name, "a");
        // after reorder, "b" is now at row 1
        assert_eq!(app.rows[1].name, "b");
    }

    #[test]
    fn reorder_down_at_last_is_noop() {
        let dir = tempdir().unwrap();
        let mut app = app_with_file(&dir, J2);
        app.selected = 2; // "b" is already last
        app.reorder_down();
        // nothing should change
        let coll = store::load_collection(&app.collections[0].path).unwrap();
        assert_eq!(coll.item[0].name, "a");
        assert_eq!(coll.item[1].name, "b");
    }

    #[test]
    fn reorder_up_at_first_is_noop() {
        let dir = tempdir().unwrap();
        let mut app = app_with_file(&dir, J2);
        app.selected = 1; // "a" is already first
        app.reorder_up();
        let coll = store::load_collection(&app.collections[0].path).unwrap();
        assert_eq!(coll.item[0].name, "a");
        assert_eq!(coll.item[1].name, "b");
    }

    // ── cross-collection move ───────────────────────────────────────────────

    #[test]
    fn move_item_across_collections_op() {
        let dir = tempdir().unwrap();
        let src = store::create_collection(dir.path(), "Src").unwrap();
        let dst = store::create_collection(dir.path(), "Dst").unwrap();
        let mut s = store::load_collection(&src).unwrap();
        store::add_request(&mut s.item, &[], "x", "GET").unwrap();
        store::save_collection(&src, &s).unwrap();

        store::move_item_across_collections(&src, "x", &dst, None).unwrap();
        assert!(store::load_collection(&src).unwrap().item.is_empty());
        assert_eq!(store::load_collection(&dst).unwrap().item[0].name, "x");
    }

    #[test]
    fn app_move_to_collection_via_prompt() {
        use std::fs;
        // src collection: c0.json with item "ping"
        let dir = tempdir().unwrap();
        let src_path = dir.path().join("c0.json");
        let dst_path = dir.path().join("c1.json");
        let src_json = r#"{"info":{"name":"Src"},"item":[{"name":"ping","request":{"method":"GET","url":"https://x/ping"}}]}"#;
        let dst_json = r#"{"info":{"name":"Dst"},"item":[]}"#;
        fs::write(&src_path, src_json).unwrap();
        fs::write(&dst_path, dst_json).unwrap();

        let lc0 = LoadedCollection {
            path: src_path.clone(),
            collection: serde_json::from_str(src_json).unwrap(),
        };
        let lc1 = LoadedCollection {
            path: dst_path.clone(),
            collection: serde_json::from_str(dst_json).unwrap(),
        };
        let mut app = App::new(dir.path().into(), vec![lc0, lc1], VarScopes::default());
        // row 0 = Src (collection), row 1 = ping (request), row 2 = Dst (collection)
        app.selected = 1; // "ping"
                          // move "ping" to collection index 1 (Dst)
        app.move_to_collection(1);
        // src should now be empty
        let src_reloaded = store::load_collection(&src_path).unwrap();
        assert!(
            src_reloaded.item.is_empty(),
            "src should be empty after move"
        );
        // dst should have "ping"
        let dst_reloaded = store::load_collection(&dst_path).unwrap();
        assert_eq!(dst_reloaded.item[0].name, "ping");
    }
}
