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
    /// Read-only overlay showing the generated curl command (stored in `App::curl_text`).
    Curl,
    /// Request-history overlay (j/k to navigate, Enter to replay the selected entry).
    History,
    /// Collection variable manager overlay (j/k navigate, a add, e edit, d delete).
    /// The target collection index is held in `App::var_ci`.
    Variables,
    /// Folder picker for a cross-collection move into a logical folder (j/k to
    /// navigate, Enter to confirm). Pending move state is held in `App::move_pending`.
    MoveFolder,
}

/// What tree-CRUD operation a prompt is driving.
// Note: not `PartialEq`/`Eq` — `DownloadResponse` carries a `Request`, which does
// not implement them.
#[derive(Debug, Clone)]
pub enum PromptOp {
    /// Add a new request — carries (collection_index, parent_item_path, default_method).
    AddRequest { ci: usize, parent: Vec<usize> },
    /// Add a new folder — carries (collection_index, parent_item_path).
    AddFolder { ci: usize, parent: Vec<usize> },
    /// Rename an existing item by its current name — carries (ci, old_name).
    Rename { ci: usize, old_name: String },
    /// Create a brand-new top-level collection in the given directory.
    CreateCollection { dir: std::path::PathBuf },
    /// Add or edit a collection variable. The buffer is `key=value`. When
    /// `edit_key` is `Some(old)`, an existing variable is being edited (renaming
    /// the key deletes the old entry); when `None`, a new variable is added.
    SetVariable { ci: usize, edit_key: Option<String> },
    /// Save the current response to a path. When `request` is `Some`, the save does
    /// a fresh streamed download (reusing golden-core `download_to_file`); when
    /// `None`, the already-fetched `last_response` bytes are written.
    DownloadResponse {
        request: Option<Box<golden_core::model::Request>>,
    },
    /// Import a source into the workspace's `collections/` and reload the tree.
    /// `from` is one of auto|postman|raw|folder|openapi|curl.
    Import { from: String },
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
    /// Delete a collection variable by key.
    DeleteVariable { ci: usize, key: String },
}

/// A cross-collection move awaiting a folder choice (drives `Mode::MoveFolder`).
#[derive(Debug, Clone)]
pub struct PendingMove {
    /// Source collection index (where the item currently lives).
    pub src_ci: usize,
    /// Destination collection index.
    pub dst_ci: usize,
    /// Name of the item being moved.
    pub item_name: String,
    /// Folder names available in the destination collection (display order).
    /// The first entry is always the synthetic "(collection root)" target.
    pub folders: Vec<String>,
    /// Index into `folders` of the highlighted destination.
    pub selected: usize,
}

/// Which sub-section of the request pane is "focused" for editing.
/// Cycling with `f` moves through Method → Url → Headers → Body → Scripts.
/// When the selected request's body is in graphql mode, two extra stops appear
/// after Body: GraphqlQuery and GraphqlVariables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestTab {
    Method,
    Url,
    Headers,
    Body,
    GraphqlQuery,
    GraphqlVariables,
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
    /// Assertions from the most recent single send (cleared on a new send).
    pub last_assertions: Vec<golden_core::result::Assertion>,
    /// Test/pre-request script error from the most recent send, if any (response present).
    pub last_script_error: Option<String>,
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
    /// First-open welcome hint; cleared on the first keypress.
    pub show_welcome: bool,
    /// Iteration count to launch the next collection/folder run with (>= 1),
    /// adjusted with `+`/`-` before a run.
    pub run_iterations: u32,
    /// The most recently generated curl command, shown in the `Mode::Curl` overlay.
    pub curl_text: String,
    /// Recent request history (oldest first), loaded from `.golden/history.jsonl`
    /// for the `Mode::History` overlay.
    pub history: Vec<golden_core::history::HistoryEntry>,
    /// Index of the highlighted entry in `history` (the History overlay shows
    /// newest first, so this indexes the display order — see `history_display`).
    pub history_selected: usize,
    /// Collection index whose variables the `Mode::Variables` overlay manages.
    pub var_ci: usize,
    /// Index of the highlighted variable in the `Mode::Variables` overlay.
    pub var_selected: usize,
    /// Pending cross-collection move awaiting a destination folder choice
    /// (Some while `Mode::MoveFolder` is active).
    pub move_pending: Option<PendingMove>,
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
            last_assertions: Vec::new(),
            last_script_error: None,
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
            show_welcome: true,
            run_iterations: 1,
            curl_text: String::new(),
            history: Vec::new(),
            history_selected: 0,
            var_ci: 0,
            var_selected: 0,
            move_pending: None,
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

    /// True when the selected request's body is in graphql mode (so the
    /// GraphqlQuery / GraphqlVariables tabs are reachable).
    pub fn selected_body_is_graphql(&self) -> bool {
        self.current_request()
            .and_then(|r| r.body.as_ref())
            .map(|b| b.mode == "graphql")
            .unwrap_or(false)
    }

    /// Cycle the focused request sub-tab forward (f key). The GraphqlQuery /
    /// GraphqlVariables stops are only visited when the body is graphql; otherwise
    /// Body steps straight to the scripts.
    pub fn next_request_tab(&mut self) {
        let graphql = self.selected_body_is_graphql();
        self.request_tab = match self.request_tab {
            RequestTab::Method => RequestTab::Url,
            RequestTab::Url => RequestTab::Headers,
            RequestTab::Headers => RequestTab::Body,
            RequestTab::Body => {
                if graphql {
                    RequestTab::GraphqlQuery
                } else {
                    RequestTab::PreRequestScript
                }
            }
            RequestTab::GraphqlQuery => RequestTab::GraphqlVariables,
            RequestTab::GraphqlVariables => RequestTab::PreRequestScript,
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

    /// The (collection, target_path_within_collection) for the selected row, used by
    /// the send worker. `target_path` is the row path minus the leading collection index.
    pub fn selected_send_target(&self) -> Option<(golden_core::model::Collection, Vec<usize>)> {
        let row = self.current_row()?;
        if row.kind != NodeKind::Request {
            return None;
        }
        let coll = self.collections.get(row.path[0])?.collection.clone();
        Some((coll, row.path[1..].to_vec()))
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
            RequestTab::GraphqlQuery => EditField::GraphqlQuery,
            RequestTab::GraphqlVariables => EditField::GraphqlVariables,
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

    /// Open a name-prompt for creating a brand-new top-level collection. The new
    /// collection file is written into `<workspace>/collections` (the same dir the
    /// loader scans), matching the `golden new` / extension behaviour. Always
    /// available (works even when nothing is selected).
    pub fn open_create_collection_prompt(&mut self) -> bool {
        let dir = self.collections_dir.join("collections");
        self.prompt = Some(PromptSession::new(
            PromptOp::CreateCollection { dir },
            "New collection (name)",
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
            ConfirmAction::DeleteVariable { ci, key } => {
                let lc = match self.collections.get_mut(ci) {
                    Some(c) => c,
                    None => {
                        self.status = "variable: collection not found".into();
                        return;
                    }
                };
                if golden_core::store::delete_variable(&mut lc.collection, &key) {
                    let path = lc.path.clone();
                    match golden_core::store::save_collection(&path, &lc.collection) {
                        Ok(()) => self.status = format!("deleted variable '{key}'"),
                        Err(e) => self.status = format!("save failed: {e}"),
                    }
                    self.reload_collection(ci);
                } else {
                    self.status = format!("variable '{key}' not found");
                }
                // Return to the variable manager (clamped) rather than Normal.
                self.var_ci = ci;
                let n = self
                    .collections
                    .get(ci)
                    .map(|c| c.collection.variable.len())
                    .unwrap_or(0);
                self.var_selected = self.var_selected.min(n.saturating_sub(1));
                self.mode = Mode::Variables;
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
            PromptOp::SetVariable { ci, edit_key } => {
                // The buffer is `key=value` (value may be empty / contain '=').
                let (key, value) = match name.split_once('=') {
                    Some((k, v)) => (k.trim().to_string(), v.trim().to_string()),
                    None => (name.clone(), String::new()),
                };
                if key.is_empty() {
                    self.status = "variable key cannot be empty".into();
                    return;
                }
                let lc = match self.collections.get_mut(ci) {
                    Some(c) => c,
                    None => {
                        self.status = "variable: collection not found".into();
                        return;
                    }
                };
                // Editing a variable to a new key drops the old entry first.
                if let Some(old) = edit_key.as_deref() {
                    if old != key {
                        golden_core::store::delete_variable(&mut lc.collection, old);
                    }
                }
                golden_core::store::set_variable(&mut lc.collection, &key, &value);
                let path = lc.path.clone();
                match golden_core::store::save_collection(&path, &lc.collection) {
                    Ok(()) => self.status = format!("set variable '{key}'"),
                    Err(e) => self.status = format!("save failed: {e}"),
                }
                self.reload_collection(ci);
                // Keep the variable manager open on the edited collection.
                self.var_ci = ci;
                let n = self
                    .collections
                    .get(ci)
                    .map(|c| c.collection.variable.len())
                    .unwrap_or(0);
                self.var_selected = self.var_selected.min(n.saturating_sub(1));
                self.mode = Mode::Variables;
            }
            PromptOp::DownloadResponse { request } => {
                let target = std::path::PathBuf::from(&name);
                match super::worker::download_response(
                    request.as_deref(),
                    self.last_response.as_ref().map(|r| r.body.as_slice()),
                    &self.scopes,
                    &super::worker::tui_http_config(),
                    &target,
                ) {
                    Ok(bytes) => {
                        self.status = format!("saved {bytes} bytes -> {}", target.display())
                    }
                    Err(e) => self.status = format!("save failed: {e}"),
                }
            }
            PromptOp::Import { from } => {
                // `name` is the trimmed buffer: "<source> [--from <kind>]".
                let (source, parsed_from) = parse_import_input(&name, &from);
                if source.is_empty() {
                    self.status = "import failed: source path required".into();
                    return;
                }
                let kind = match super::worker::parse_import_from(&parsed_from) {
                    Ok(k) => k,
                    Err(e) => {
                        self.status = format!("import failed: {e}");
                        return;
                    }
                };
                // Mirror `golden import`: write into `<workspace>/collections`.
                let root = self.collections_dir.join("collections");
                match crate::commands::import::import_into(
                    &root,
                    &source,
                    None,
                    golden_core::import::MergeStrategy::Add,
                    kind,
                ) {
                    Ok(outcomes) => {
                        let imported = outcomes
                            .iter()
                            .filter(|o| {
                                matches!(o, crate::commands::import::ImportOutcome::Imported(_))
                            })
                            .count();
                        let skipped = outcomes.len() - imported;
                        self.status = if skipped > 0 {
                            format!("imported {imported} ({skipped} skipped)")
                        } else {
                            format!("imported {imported} collection(s)")
                        };
                        // Reload the tree so the new collections appear immediately
                        // (the watcher also fires, but this makes it deterministic).
                        let (collections, _errors) =
                            super::loader::load_collections(&self.collections_dir, &[]);
                        self.collections = collections;
                        self.rebuild_rows();
                    }
                    Err(e) => self.status = format!("import failed: {e}"),
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

    // ── collection variable manager (Mode::Variables) ─────────────────────

    /// The collection index that owns the selected row (any node kind). Used to
    /// open the variable manager for the collection the selection belongs to.
    fn selected_collection_index(&self) -> Option<usize> {
        self.current_row().and_then(|r| r.path.first().copied())
    }

    /// Open the variable manager for the collection the current selection belongs
    /// to. Returns false when nothing is selected.
    pub fn open_variables(&mut self) -> bool {
        let ci = match self.selected_collection_index() {
            Some(c) => c,
            None => {
                self.status = "select a collection first".into();
                return false;
            }
        };
        self.var_ci = ci;
        self.var_selected = 0;
        self.mode = Mode::Variables;
        true
    }

    /// Variables of the collection currently targeted by the manager.
    pub fn current_variables(&self) -> &[golden_core::model::Variable] {
        self.collections
            .get(self.var_ci)
            .map(|c| c.collection.variable.as_slice())
            .unwrap_or(&[])
    }

    /// Move the variable-manager selection down, clamped.
    pub fn var_select_next(&mut self) {
        let n = self.current_variables().len();
        if n > 0 {
            self.var_selected = (self.var_selected + 1).min(n - 1);
        }
    }

    /// Move the variable-manager selection up, clamped.
    pub fn var_select_prev(&mut self) {
        self.var_selected = self.var_selected.saturating_sub(1);
    }

    /// Open the add-variable prompt for the manager's collection.
    pub fn open_add_variable_prompt(&mut self) {
        let ci = self.var_ci;
        self.prompt = Some(PromptSession::new(
            PromptOp::SetVariable { ci, edit_key: None },
            "Add variable (key=value)",
        ));
        self.mode = Mode::Prompt;
    }

    /// Open the edit-variable prompt pre-filled with the selected variable's
    /// `key=value`. No-op (sets a status) when there is no variable selected.
    pub fn open_edit_variable_prompt(&mut self) {
        let ci = self.var_ci;
        let var = match self.current_variables().get(self.var_selected) {
            Some(v) => v.clone(),
            None => {
                self.status = "no variable selected".into();
                return;
            }
        };
        let mut sess = PromptSession::new(
            PromptOp::SetVariable {
                ci,
                edit_key: Some(var.key.clone()),
            },
            "Edit variable (key=value)",
        );
        sess.buffer = format!("{}={}", var.key, var.value);
        self.prompt = Some(sess);
        self.mode = Mode::Prompt;
    }

    /// Start a delete confirmation for the selected variable. No-op (status) when
    /// there is no variable selected.
    pub fn start_delete_variable_confirm(&mut self) {
        let ci = self.var_ci;
        let key = match self.current_variables().get(self.var_selected) {
            Some(v) => v.key.clone(),
            None => {
                self.status = "no variable selected".into();
                return;
            }
        };
        self.confirm = Some(ConfirmOp {
            message: format!("delete variable '{key}'? (y/n)"),
            action: ConfirmAction::DeleteVariable { ci, key },
        });
        self.mode = Mode::Confirm;
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
    ///
    /// This is the "drop at root" path. To choose a destination folder, go
    /// through `begin_move_to_collection` (opens the folder picker) instead.
    pub fn move_to_collection(&mut self, target_ci: usize) {
        self.move_to_collection_folder(target_ci, None);
    }

    /// Execute a cross-collection move of the current selection into `target_ci`,
    /// optionally dropping it into the logical folder named `dst_folder` (`None`
    /// = collection root). Reuses golden-core `move_item_across_collections`.
    pub fn move_to_collection_folder(&mut self, target_ci: usize, dst_folder: Option<&str>) {
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
            &src_path, &item_name, &dst_path, dst_folder,
        ) {
            Ok(()) => {
                self.status = match dst_folder {
                    Some(f) => format!("moved '{item_name}' to '{f}'"),
                    None => format!("moved '{item_name}' to collection {target_ci}"),
                };
                self.reload_collection(ci);
                self.reload_collection(target_ci);
            }
            Err(e) => self.status = format!("move failed: {e}"),
        }
    }

    /// Begin a move into `target_ci`: gather the destination collection's folder
    /// names and open the `Mode::MoveFolder` picker. When the destination has no
    /// folders, this moves straight to the root (skipping the picker).
    pub fn begin_move_to_collection(&mut self, target_ci: usize) {
        let row = match self.current_row() {
            Some(r) => r.clone(),
            None => {
                self.status = "nothing selected".into();
                self.mode = Mode::Normal;
                return;
            }
        };
        let src_ci = row.path[0];
        let item_name = row.name.clone();
        let folder_names = match self.collections.get(target_ci) {
            Some(lc) => collect_folder_names(&lc.collection.item),
            None => {
                self.status = format!("target collection {target_ci} not found");
                self.mode = Mode::Normal;
                return;
            }
        };
        // No folders to pick — drop at root directly.
        if folder_names.is_empty() {
            self.mode = Mode::Normal;
            self.move_to_collection_folder(target_ci, None);
            return;
        }
        // First entry is the synthetic "collection root" target.
        let mut folders = vec![ROOT_TARGET_LABEL.to_string()];
        folders.extend(folder_names);
        self.move_pending = Some(PendingMove {
            src_ci,
            dst_ci: target_ci,
            item_name,
            folders,
            selected: 0,
        });
        self.mode = Mode::MoveFolder;
    }

    /// Move the folder-picker selection down, clamped.
    pub fn move_folder_select_next(&mut self) {
        if let Some(p) = self.move_pending.as_mut() {
            if !p.folders.is_empty() {
                p.selected = (p.selected + 1).min(p.folders.len() - 1);
            }
        }
    }

    /// Move the folder-picker selection up, clamped.
    pub fn move_folder_select_prev(&mut self) {
        if let Some(p) = self.move_pending.as_mut() {
            p.selected = p.selected.saturating_sub(1);
        }
    }

    /// Execute the pending cross-collection move using the highlighted folder
    /// (the synthetic root entry maps to `None`). Clears the pending state.
    pub fn confirm_move_to_folder(&mut self) {
        let pending = match self.move_pending.take() {
            Some(p) => p,
            None => {
                self.mode = Mode::Normal;
                return;
            }
        };
        self.mode = Mode::Normal;
        let folder = pending.folders.get(pending.selected).cloned();
        let dst_folder = match folder.as_deref() {
            Some(ROOT_TARGET_LABEL) | None => None,
            Some(f) => Some(f),
        };
        self.move_to_collection_folder(pending.dst_ci, dst_folder);
    }

    // ── response/request gestures: cURL copy + open-in-browser ─────────────

    /// Build a curl command for the currently selected request, substituting the
    /// active scopes. Returns None when the selection is not a request row.
    /// Mirrors the `golden curl` command's default (unmasked) flags.
    pub fn build_curl_for_selected(&self) -> Option<String> {
        let request = self.current_request()?;
        let opts = golden_core::curl::CurlOptions::default();
        Some(golden_core::curl::generate(
            request,
            self.scopes.as_map(),
            &opts,
        ))
    }

    /// cURL COPY gesture: generate a curl command for the selected request, copy it
    /// to the system clipboard, and open the read-only `Mode::Curl` overlay showing
    /// it. If the clipboard cannot be initialised or written, the overlay is still
    /// shown (no panic) and the status notes the copy failure.
    pub fn copy_curl_to_clipboard(&mut self) {
        let Some(line) = self.build_curl_for_selected() else {
            self.status = "select a request first".into();
            return;
        };
        self.curl_text = line.clone();
        self.mode = Mode::Curl;
        match copy_to_clipboard(&line) {
            Ok(()) => self.status = "curl copied to clipboard".into(),
            Err(e) => self.status = format!("curl ready (clipboard unavailable: {e})"),
        }
    }

    /// OPEN-IN-BROWSER gesture: write the most recent response body to a temp file
    /// and open it in the default browser (the `send --open` path, reusing the
    /// golden-core viewers). Surfaces any failure in `self.status`.
    pub fn open_response_in_browser(&mut self) {
        let Some(resp) = self.last_response.as_ref() else {
            self.status = "no response to open".into();
            return;
        };
        match golden_core::viewers::write_html_temp(&resp.body) {
            Ok(path) => {
                let shown = path.display().to_string();
                match open::that(&path) {
                    Ok(()) => self.status = format!("opened {shown}"),
                    Err(e) => self.status = format!("could not open browser: {e}"),
                }
            }
            Err(e) => self.status = format!("could not write preview: {e}"),
        }
    }

    // ── download response to disk + import (Mode::Prompt) ──────────────────

    /// Suggest a filename for saving the current response. Prefers the last path
    /// segment of the selected request's (substituted) URL; falls back to a
    /// content-type-derived "response.<ext>" and finally "response.bin".
    pub fn suggested_download_filename(&self) -> String {
        // Try the selected request's URL path segment first.
        if let Some(req) = self.current_request() {
            let url = golden_core::subst::substitute(req.url.raw(), self.scopes.as_map());
            // Strip query/fragment, then isolate the path *after* the authority so
            // a host-only URL (e.g. "https://api.test") can't be mistaken for a
            // filename. Drop the scheme, then keep only what follows the first '/'.
            let no_query = url.split(['?', '#']).next().unwrap_or(&url);
            let after_scheme = no_query
                .split_once("://")
                .map_or(no_query, |(_, rest)| rest);
            if let Some((_authority, path)) = after_scheme.split_once('/') {
                if let Some(seg) = path.trim_end_matches('/').rsplit('/').next() {
                    if !seg.is_empty() && seg.contains('.') {
                        return seg.to_string();
                    }
                }
            }
        }
        // Fall back to a content-type-derived extension.
        let ext = self
            .last_response
            .as_ref()
            .and_then(|r| {
                r.headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                    .map(|(_, v)| v.clone())
            })
            .map(|ct| ext_for_content_type(&ct))
            .unwrap_or("bin");
        format!("response.{ext}")
    }

    /// Open the save-to-disk prompt for the current response. The prompt is
    /// pre-filled with a suggested filename. Requires a response to be present.
    /// When the selection is a request row, the save re-downloads fresh (so the
    /// streamed `download_to_file` path is reused); otherwise the cached bytes
    /// are written. Returns false (with a status) when there is no response.
    pub fn open_download_prompt(&mut self) -> bool {
        if self.last_response.is_none() {
            self.status = "no response to save".into();
            return false;
        }
        let request = self.current_request().cloned().map(Box::new);
        let suggested = self.suggested_download_filename();
        let mut sess = PromptSession::new(
            PromptOp::DownloadResponse { request },
            "Save response (path)",
        );
        sess.buffer = suggested;
        self.prompt = Some(sess);
        self.mode = Mode::Prompt;
        true
    }

    /// Open the import prompt (source path + optional inline `--from <kind>`).
    /// On commit the source is imported into the workspace `collections/` dir via
    /// the same code path as `golden import`, then the tree is reloaded.
    pub fn open_import_prompt(&mut self) -> bool {
        let sess = PromptSession::new(
            PromptOp::Import {
                from: "auto".into(),
            },
            "Import (source path · optional --from auto|postman|raw|folder|openapi|curl)",
        );
        self.prompt = Some(sess);
        self.mode = Mode::Prompt;
        true
    }

    // ── request history overlay (Mode::History) ───────────────────────────

    /// Load recent request history from `<workspace>/.golden/history.jsonl` and
    /// open the History overlay. Reuses `golden_core::history::read_all` (parsing
    /// lives in golden-core). On a read error the overlay still opens (empty list)
    /// with the error surfaced in `self.status`.
    pub fn open_history(&mut self, workspace: &std::path::Path) {
        match golden_core::history::read_all(workspace) {
            Ok(entries) => self.history = entries,
            Err(e) => {
                self.history = Vec::new();
                self.status = format!("history: {e}");
            }
        }
        // Newest entry highlighted first (display order is newest-first).
        self.history_selected = 0;
        self.mode = Mode::History;
    }

    /// History entries in display order (newest first), as returned to the UI.
    pub fn history_display(&self) -> Vec<&golden_core::history::HistoryEntry> {
        self.history.iter().rev().collect()
    }

    /// Move the history selection down (toward older entries), clamped.
    pub fn history_select_next(&mut self) {
        if !self.history.is_empty() {
            self.history_selected = (self.history_selected + 1).min(self.history.len() - 1);
        }
    }

    /// Move the history selection up (toward newer entries), clamped.
    pub fn history_select_prev(&mut self) {
        self.history_selected = self.history_selected.saturating_sub(1);
    }

    /// The history entry currently highlighted in the overlay, if any.
    pub fn selected_history_entry(&self) -> Option<&golden_core::history::HistoryEntry> {
        self.history_display().get(self.history_selected).copied()
    }

    /// Build a single-item synthetic `Collection` wrapping the selected history
    /// entry's request, suitable for the existing send worker (`spawn_send` with
    /// `target_path = [0]`). Mirrors the CLI `history replay` request construction.
    /// Returns None when there is no selected entry.
    pub fn history_replay_collection(&self) -> Option<golden_core::model::Collection> {
        let e = self.selected_history_entry()?;
        let request = golden_core::model::Request {
            method: e.method.clone(),
            url: golden_core::model::Url::Raw(e.url.clone()),
            header: e
                .request_headers
                .iter()
                .map(|(k, v)| golden_core::model::Header {
                    key: k.clone(),
                    value: v.clone(),
                    disabled: false,
                    extra: serde_json::Map::new(),
                })
                .collect(),
            body: e.request_body.clone().map(|raw| golden_core::model::Body {
                mode: "raw".into(),
                raw: Some(serde_json::Value::String(raw)),
                graphql: None,
                formdata: vec![],
            }),
        };
        let item = golden_core::model::Item {
            name: format!("{} {}", e.method, e.url),
            description: None,
            item: None,
            request: Some(request),
            event: vec![],
            extra: serde_json::Map::new(),
        };
        Some(golden_core::model::Collection {
            info: golden_core::model::Info {
                name: "history-replay".into(),
                extra: serde_json::Map::new(),
            },
            variable: vec![],
            item: vec![item],
            extra: serde_json::Map::new(),
        })
    }
}

/// Label for the synthetic "drop at collection root" entry in the move-folder picker.
pub(crate) const ROOT_TARGET_LABEL: &str = "(collection root)";

/// Collect the names of all logical folders in an item tree (depth-first). Used
/// to populate the move-to-folder picker. The names mirror what
/// `move_item_across_collections`'s `dst_folder` matches on (by name, recursive).
pub(crate) fn collect_folder_names(items: &[golden_core::model::Item]) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(items: &[golden_core::model::Item], out: &mut Vec<String>) {
        for item in items {
            if item.is_folder() {
                out.push(item.name.clone());
                if let Some(children) = item.item.as_deref() {
                    walk(children, out);
                }
            }
        }
    }
    walk(items, &mut out);
    out
}

/// Copy `text` to the system clipboard. Returns the error string on failure so the
/// caller can degrade gracefully (e.g. when no clipboard backend is available).
fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| e.to_string())
}

/// Map a Content-Type header value to a file extension for a suggested filename.
/// Only the type token before any `;` parameters matters.
fn ext_for_content_type(content_type: &str) -> &'static str {
    let main = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match main.as_str() {
        "application/json" => "json",
        "text/html" => "html",
        "text/plain" => "txt",
        "text/csv" => "csv",
        "application/xml" | "text/xml" => "xml",
        "application/pdf" => "pdf",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "application/octet-stream" => "bin",
        _ => "bin",
    }
}

/// Parse the import prompt's single-line buffer into `(source, from)`.
/// Supports an optional trailing `--from <kind>`; when absent, `default_from`
/// (typically "auto") is used. The source is everything before the flag.
pub(crate) fn parse_import_input(buffer: &str, default_from: &str) -> (String, String) {
    if let Some(idx) = buffer.find("--from") {
        let source = buffer[..idx].trim().to_string();
        let rest = buffer[idx + "--from".len()..].trim();
        // The kind is the next whitespace-delimited token after --from.
        let kind = rest.split_whitespace().next().unwrap_or(default_from);
        let kind = if kind.is_empty() {
            default_from.to_string()
        } else {
            kind.to_string()
        };
        (source, kind)
    } else {
        (buffer.trim().to_string(), default_from.to_string())
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

    const J_GQL: &str = r#"{
      "info": { "name": "G" },
      "item": [
        { "name": "gql", "request": {
          "method": "POST",
          "url": "https://x/graphql",
          "body": { "mode": "graphql", "graphql": { "query": "{ me { id } }" } }
        }}
      ]
    }"#;

    #[test]
    fn request_tab_cycle_visits_graphql_when_body_is_graphql() {
        let mut app = app_with(J_GQL);
        // row 1 = the gql request
        app.selected = 1;
        assert!(app.selected_body_is_graphql());
        app.request_tab = RequestTab::Body;
        app.next_request_tab();
        assert_eq!(app.request_tab, RequestTab::GraphqlQuery);
        app.next_request_tab();
        assert_eq!(app.request_tab, RequestTab::GraphqlVariables);
        app.next_request_tab();
        assert_eq!(app.request_tab, RequestTab::PreRequestScript);
    }

    #[test]
    fn request_tab_cycle_skips_graphql_for_non_graphql_body() {
        let mut app = app_with(J);
        app.selected = 2; // login (raw body)
        assert!(!app.selected_body_is_graphql());
        app.request_tab = RequestTab::Body;
        app.next_request_tab();
        assert_eq!(
            app.request_tab,
            RequestTab::PreRequestScript,
            "non-graphql body should skip the graphql tabs"
        );
    }

    #[test]
    fn open_edit_on_graphql_query_tab_opens_graphql_query_field() {
        let mut app = app_with(J_GQL);
        app.selected = 1;
        app.request_tab = RequestTab::GraphqlQuery;
        assert!(app.open_edit(), "graphql query editor should be reachable");
        assert_eq!(app.mode, Mode::Edit);
        let session = app.edit.as_ref().unwrap();
        assert_eq!(session.field, EditField::GraphqlQuery);
        assert_eq!(session.buffer, "{ me { id } }");
    }

    #[test]
    fn open_edit_on_graphql_variables_tab_opens_graphql_variables_field() {
        let mut app = app_with(J_GQL);
        app.selected = 1;
        app.request_tab = RequestTab::GraphqlVariables;
        assert!(
            app.open_edit(),
            "graphql variables editor should be reachable"
        );
        assert_eq!(app.mode, Mode::Edit);
        assert_eq!(
            app.edit.as_ref().unwrap().field,
            EditField::GraphqlVariables
        );
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

    // ── curl + open-in-browser gestures ───────────────────────────────────

    #[test]
    fn build_curl_for_selected_substitutes_vars_and_targets_request() {
        let mut app = app_with(J);
        app.scopes.set("base".into(), "https://api.test".into());
        app.selected = 2; // "login" (POST {{base}}/login)
        let line = app
            .build_curl_for_selected()
            .expect("login is a request row");
        assert!(line.starts_with("curl -X POST"), "got: {line}");
        // The {{base}} var must be substituted before the URL is emitted.
        assert!(line.contains("'https://api.test/login'"), "got: {line}");
    }

    #[test]
    fn build_curl_for_selected_is_none_on_non_request_row() {
        let mut app = app_with(J);
        app.selected = 1; // "auth" folder — not a request
        assert!(app.build_curl_for_selected().is_none());
    }

    #[test]
    fn copy_curl_opens_overlay_and_fills_curl_text() {
        let mut app = app_with(J);
        app.scopes.set("base".into(), "https://api.test".into());
        app.selected = 2; // "login"
        app.copy_curl_to_clipboard();
        // Regardless of clipboard availability, the overlay must open with the text.
        assert_eq!(app.mode, Mode::Curl);
        assert!(
            app.curl_text.starts_with("curl -X POST"),
            "overlay text should hold the generated curl, got: {}",
            app.curl_text
        );
    }

    #[test]
    fn copy_curl_on_non_request_row_does_not_open_overlay() {
        let mut app = app_with(J);
        app.selected = 1; // "auth" folder
        app.copy_curl_to_clipboard();
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.status, "select a request first");
        assert!(app.curl_text.is_empty());
    }

    // ── request history overlay ───────────────────────────────────────────

    fn hist_entry(
        method: &str,
        url: &str,
        status: Option<u16>,
    ) -> golden_core::history::HistoryEntry {
        golden_core::history::HistoryEntry {
            timestamp: "2026-06-09T00:00:00Z".into(),
            method: method.into(),
            url: url.into(),
            request_headers: vec![("Accept".into(), "application/json".into())],
            request_body: None,
            status,
            time_ms: 7,
        }
    }

    #[test]
    fn open_history_loads_entries_and_enters_history_mode() {
        use std::fs;
        use tempfile::tempdir;
        let ws = tempdir().unwrap();
        // Persist two entries via golden-core so we exercise the real parser.
        golden_core::history::append(
            ws.path(),
            hist_entry("GET", "https://api.test/a", Some(200)),
            false,
        )
        .unwrap();
        golden_core::history::append(
            ws.path(),
            hist_entry("POST", "https://api.test/b", Some(201)),
            false,
        )
        .unwrap();
        // Sanity: file exists under .golden.
        assert!(fs::metadata(ws.path().join(".golden").join("history.jsonl")).is_ok());

        let mut app = app_with(J);
        app.open_history(ws.path());
        assert_eq!(app.mode, Mode::History);
        assert_eq!(app.history.len(), 2);
        // Display order is newest-first; selection starts at the newest entry.
        assert_eq!(app.history_selected, 0);
        let top = app.selected_history_entry().unwrap();
        assert_eq!(top.method, "POST");
        assert_eq!(top.url, "https://api.test/b");
    }

    #[test]
    fn open_history_on_missing_file_opens_empty() {
        use tempfile::tempdir;
        let ws = tempdir().unwrap();
        let mut app = app_with(J);
        app.open_history(ws.path());
        assert_eq!(app.mode, Mode::History);
        assert!(app.history.is_empty());
        assert!(app.selected_history_entry().is_none());
    }

    #[test]
    fn history_navigation_moves_within_bounds_newest_first() {
        let mut app = app_with(J);
        // oldest first in storage; display reverses to newest-first.
        app.history = vec![
            hist_entry("GET", "https://api.test/old", Some(200)),
            hist_entry("GET", "https://api.test/mid", Some(200)),
            hist_entry("POST", "https://api.test/new", Some(201)),
        ];
        app.history_selected = 0;
        // selected starts at newest (last stored)
        assert_eq!(
            app.selected_history_entry().unwrap().url,
            "https://api.test/new"
        );
        app.history_select_prev(); // clamps at 0
        assert_eq!(app.history_selected, 0);
        app.history_select_next();
        assert_eq!(
            app.selected_history_entry().unwrap().url,
            "https://api.test/mid"
        );
        app.history_select_next();
        assert_eq!(
            app.selected_history_entry().unwrap().url,
            "https://api.test/old"
        );
        app.history_select_next(); // clamps at last
        assert_eq!(app.history_selected, 2);
        app.history_select_prev();
        assert_eq!(
            app.selected_history_entry().unwrap().url,
            "https://api.test/mid"
        );
    }

    #[test]
    fn history_replay_collection_builds_single_item_request() {
        let mut app = app_with(J);
        let mut e = hist_entry("POST", "https://api.test/login", Some(200));
        e.request_body = Some("{\"u\":1}".into());
        app.history = vec![e];
        app.history_selected = 0;
        let coll = app
            .history_replay_collection()
            .expect("a selected entry yields a replay collection");
        assert_eq!(coll.item.len(), 1);
        let req = coll.item[0].request.as_ref().unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.url.raw(), "https://api.test/login");
        assert!(req.body.is_some(), "replayed request keeps its body");
    }

    #[test]
    fn history_replay_collection_is_none_when_empty() {
        let app = app_with(J);
        assert!(app.history.is_empty());
        assert!(app.history_replay_collection().is_none());
    }

    #[test]
    fn open_response_in_browser_without_response_sets_status() {
        let mut app = app_with(J);
        assert!(app.last_response.is_none());
        app.open_response_in_browser();
        assert_eq!(app.status, "no response to open");
        // Mode is untouched on the no-response path.
        assert_eq!(app.mode, Mode::Normal);
    }

    // ── download response + import prompts ─────────────────────────────────

    fn resp_with(content_type: Option<&str>, body: &[u8]) -> golden_core::http::HttpResponse {
        golden_core::http::HttpResponse {
            status: 200,
            headers: content_type
                .map(|ct| vec![("content-type".to_string(), ct.to_string())])
                .unwrap_or_default(),
            body: body.to_vec(),
            time_ms: 1,
        }
    }

    #[test]
    fn open_download_prompt_without_response_sets_status_and_stays_normal() {
        let mut app = app_with(J);
        assert!(app.last_response.is_none());
        assert!(!app.open_download_prompt(), "no response → no prompt");
        assert_eq!(app.status, "no response to save");
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.prompt.is_none());
    }

    #[test]
    fn open_download_prompt_prefills_suggested_filename_from_url() {
        let mut app = app_with(J);
        app.scopes.set("base".into(), "https://api.test".into());
        // Select a request whose URL ends in a filename-like segment.
        app.collections[0].collection.item[1]
            .request
            .as_mut()
            .unwrap()
            .url = golden_core::model::Url::Raw("https://api.test/files/report.pdf".into());
        app.rebuild_rows();
        app.selected = 3; // "ping" request row, now pointing at /files/report.pdf
        app.last_response = Some(resp_with(Some("application/pdf"), b"%PDF-1.4"));
        assert!(app.open_download_prompt());
        assert_eq!(app.mode, Mode::Prompt);
        let sess = app.prompt.as_ref().unwrap();
        assert!(
            matches!(sess.op, PromptOp::DownloadResponse { .. }),
            "prompt op should be DownloadResponse"
        );
        assert_eq!(sess.buffer, "report.pdf");
    }

    #[test]
    fn suggested_filename_falls_back_to_content_type_extension() {
        let mut app = app_with(J);
        // Select the collection header (no request URL to derive a name from).
        app.selected = 0;
        app.last_response = Some(resp_with(Some("application/json; charset=utf-8"), b"{}"));
        assert_eq!(app.suggested_download_filename(), "response.json");
    }

    #[test]
    fn suggested_filename_falls_back_when_url_has_no_path() {
        let mut app = app_with(J);
        // `{{base}}` resolves to a host-only URL (no path after the authority);
        // the dot-bearing host must NOT be mistaken for a filename.
        app.scopes.set("base".into(), "https://api.test".into());
        app.collections[0].collection.item[1]
            .request
            .as_mut()
            .unwrap()
            .url = golden_core::model::Url::Raw("{{base}}".into());
        app.rebuild_rows();
        app.selected = 3; // "ping" request row, now host-only
        app.last_response = Some(resp_with(Some("application/json"), b"{}"));
        assert_eq!(
            app.suggested_download_filename(),
            "response.json",
            "host-only URL should fall back to the content-type name"
        );

        // A URL with a real path after the authority still yields the segment.
        app.collections[0].collection.item[1]
            .request
            .as_mut()
            .unwrap()
            .url = golden_core::model::Url::Raw("{{base}}/files/report.pdf".into());
        app.rebuild_rows();
        app.selected = 3;
        app.last_response = Some(resp_with(Some("application/pdf"), b"%PDF-1.4"));
        assert_eq!(app.suggested_download_filename(), "report.pdf");
    }

    #[test]
    fn commit_download_prompt_writes_cached_bytes_when_no_request() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let target = dir.path().join("out.json");
        let mut app = app_with(J);
        app.selected = 0; // collection header → current_request() is None
        app.last_response = Some(resp_with(Some("application/json"), b"{\"ok\":true}"));
        assert!(app.open_download_prompt());
        // Overwrite the suggested name with our temp target path.
        app.prompt.as_mut().unwrap().buffer = target.to_string_lossy().into_owned();
        app.commit_prompt();
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.status.starts_with("saved"), "status: {}", app.status);
        assert_eq!(std::fs::read(&target).unwrap(), b"{\"ok\":true}");
    }

    #[test]
    fn open_import_prompt_enters_prompt_mode_with_import_op() {
        let mut app = app_with(J);
        assert!(app.open_import_prompt());
        assert_eq!(app.mode, Mode::Prompt);
        let sess = app.prompt.as_ref().unwrap();
        assert!(matches!(sess.op, PromptOp::Import { .. }));
    }

    #[test]
    fn commit_import_prompt_imports_file_and_reloads_tree() {
        use std::fs;
        use tempfile::tempdir;
        let ws = tempdir().unwrap();
        // A source postman file to import.
        let src = ws.path().join("api.json");
        fs::write(
            &src,
            r#"{"info":{"name":"Imported"},"item":[{"name":"hi","request":{"method":"GET","url":"https://x/hi"}}]}"#,
        )
        .unwrap();

        // App rooted at the workspace (collections_dir == workspace, as the TUI sets it).
        let mut app = App::new(ws.path().into(), vec![], VarScopes::default());
        assert!(app.open_import_prompt());
        app.prompt.as_mut().unwrap().buffer = src.to_string_lossy().into_owned();
        app.commit_prompt();

        assert_eq!(app.mode, Mode::Normal);
        assert!(app.status.contains("imported"), "status: {}", app.status);
        // A collection JSON file was written under <workspace>/collections.
        let coll_dir = ws.path().join("collections");
        let written: Vec<_> = fs::read_dir(&coll_dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
            .collect();
        assert_eq!(written.len(), 1, "exactly one collection file written");
        // The reloaded tree now contains the imported request item.
        assert!(
            app.rows.iter().any(|r| r.name == "hi"),
            "imported request should appear in the tree rows, got: {:?}",
            app.rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn commit_import_prompt_with_bad_from_flag_sets_error() {
        use tempfile::tempdir;
        let ws = tempdir().unwrap();
        let mut app = App::new(ws.path().into(), vec![], VarScopes::default());
        assert!(app.open_import_prompt());
        app.prompt.as_mut().unwrap().buffer = "some.json --from bogus".into();
        app.commit_prompt();
        assert_eq!(app.mode, Mode::Normal);
        assert!(
            app.status.contains("import failed") && app.status.contains("bogus"),
            "status: {}",
            app.status
        );
    }

    #[test]
    fn parse_import_input_splits_source_and_from() {
        assert_eq!(
            parse_import_input("api.json", "auto"),
            ("api.json".to_string(), "auto".to_string())
        );
        assert_eq!(
            parse_import_input("spec.json --from openapi", "auto"),
            ("spec.json".to_string(), "openapi".to_string())
        );
        // Trailing --from with no value falls back to the default.
        assert_eq!(
            parse_import_input("x.json --from", "auto"),
            ("x.json".to_string(), "auto".to_string())
        );
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

    // ── create collection (Task: tree/edit parity item a) ───────────────────

    #[test]
    fn open_create_collection_prompt_builds_create_op_in_collections_dir() {
        let (mut app, dir) = app_with_file(J_EMPTY);
        assert!(app.open_create_collection_prompt());
        assert_eq!(app.mode, Mode::Prompt);
        let sess = app.prompt.as_ref().unwrap();
        match &sess.op {
            PromptOp::CreateCollection { dir: d } => {
                // Must target <workspace>/collections so the loader picks it up.
                assert_eq!(*d, dir.path().join("collections"));
            }
            other => panic!("expected CreateCollection op, got {other:?}"),
        }
    }

    #[test]
    fn create_collection_commit_writes_file_and_adds_row() {
        let (mut app, dir) = app_with_file(J_EMPTY);
        app.open_create_collection_prompt();
        app.prompt.as_mut().unwrap().buffer = "New API".into();
        app.commit_prompt();

        assert_eq!(app.mode, Mode::Normal);
        assert!(app.status.contains("created"), "status: {}", app.status);
        // The new collection appears as a tree row.
        assert!(
            app.rows.iter().any(|r| r.name == "New API"),
            "new collection should appear in the tree"
        );
        // The file lands under <workspace>/collections with a slugified name.
        let written = dir.path().join("collections").join("new-api.json");
        assert!(
            written.exists(),
            "collection file should be created on disk"
        );
        let reloaded = store::load_collection(&written).unwrap();
        assert_eq!(reloaded.info.name, "New API");
    }

    // ── collection variable CRUD (Task: tree/edit parity item b) ────────────

    const J_VARS: &str = r#"{
      "info": { "name": "VarColl" },
      "variable": [ { "key": "base", "value": "https://x", "type": "string" } ],
      "item": [
        { "name": "ping", "request": { "method": "GET", "url": "{{base}}/ping" } }
      ]
    }"#;

    #[test]
    fn open_variables_targets_selected_collection() {
        let (mut app, _dir) = app_with_file(J_VARS);
        app.selected = 1; // the "ping" request row (belongs to collection 0)
        assert!(app.open_variables());
        assert_eq!(app.mode, Mode::Variables);
        assert_eq!(app.var_ci, 0);
        assert_eq!(app.current_variables().len(), 1);
        assert_eq!(app.current_variables()[0].key, "base");
    }

    #[test]
    fn add_variable_commit_persists_new_variable() {
        let (mut app, dir) = app_with_file(J_VARS);
        app.selected = 0;
        app.open_variables();
        app.open_add_variable_prompt();
        assert_eq!(app.mode, Mode::Prompt);
        app.prompt.as_mut().unwrap().buffer = "token=abc123".into();
        app.commit_prompt();

        // Returns to the variable manager, not Normal.
        assert_eq!(app.mode, Mode::Variables);
        assert!(
            app.status.contains("set variable"),
            "status: {}",
            app.status
        );
        let reloaded = store::load_collection(&dir.path().join("c.json")).unwrap();
        let token = reloaded.variable.iter().find(|v| v.key == "token").unwrap();
        assert_eq!(token.value, "abc123");
    }

    #[test]
    fn edit_variable_commit_updates_value_and_preserves_extra() {
        let (mut app, dir) = app_with_file(J_VARS);
        app.selected = 0;
        app.open_variables();
        app.var_selected = 0; // "base"
        app.open_edit_variable_prompt();
        let sess = app.prompt.as_ref().unwrap();
        // Prompt pre-filled with the current key=value.
        assert_eq!(sess.buffer, "base=https://x");
        app.prompt.as_mut().unwrap().buffer = "base=https://y".into();
        app.commit_prompt();

        assert_eq!(app.mode, Mode::Variables);
        let reloaded = store::load_collection(&dir.path().join("c.json")).unwrap();
        let base = reloaded.variable.iter().find(|v| v.key == "base").unwrap();
        assert_eq!(base.value, "https://y");
        // The Postman `type` extra survives an edit (set_variable preserves extra).
        assert_eq!(
            base.extra.get("type").and_then(|v| v.as_str()),
            Some("string")
        );
    }

    #[test]
    fn delete_variable_requires_confirm_then_persists() {
        let (mut app, dir) = app_with_file(J_VARS);
        app.selected = 0;
        app.open_variables();
        app.var_selected = 0; // "base"
        app.start_delete_variable_confirm();
        assert_eq!(app.mode, Mode::Confirm);
        assert!(
            app.confirm.as_ref().unwrap().message.contains("base"),
            "confirm should name the variable"
        );
        app.execute_confirm();
        // Returns to the manager, variable gone on disk.
        assert_eq!(app.mode, Mode::Variables);
        let reloaded = store::load_collection(&dir.path().join("c.json")).unwrap();
        assert!(reloaded.variable.iter().all(|v| v.key != "base"));
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

    // ── move into a destination folder (Task: tree/edit parity item c) ───────

    /// Build a two-collection app: src has "ping" at root; dst has folder "auth".
    fn app_two_collections(
        dir: &tempfile::TempDir,
    ) -> (App, std::path::PathBuf, std::path::PathBuf) {
        use std::fs;
        let src_path = dir.path().join("c0.json");
        let dst_path = dir.path().join("c1.json");
        let src_json = r#"{"info":{"name":"Src"},"item":[{"name":"ping","request":{"method":"GET","url":"https://x/ping"}}]}"#;
        let dst_json = r#"{"info":{"name":"Dst"},"item":[{"name":"auth","item":[]}]}"#;
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
        let app = App::new(dir.path().into(), vec![lc0, lc1], VarScopes::default());
        (app, src_path, dst_path)
    }

    #[test]
    fn begin_move_opens_folder_picker_when_dest_has_folders() {
        let dir = tempdir().unwrap();
        let (mut app, _src, _dst) = app_two_collections(&dir);
        app.selected = 1; // "ping" in Src
        app.begin_move_to_collection(1);
        assert_eq!(app.mode, Mode::MoveFolder);
        let pending = app.move_pending.as_ref().unwrap();
        // First entry is the synthetic root, then the destination's folders.
        assert_eq!(pending.folders[0], ROOT_TARGET_LABEL);
        assert!(pending.folders.iter().any(|f| f == "auth"));
        assert_eq!(pending.item_name, "ping");
        assert_eq!(pending.dst_ci, 1);
    }

    #[test]
    fn confirm_move_into_folder_drops_item_inside_folder() {
        let dir = tempdir().unwrap();
        let (mut app, src_path, dst_path) = app_two_collections(&dir);
        app.selected = 1; // "ping"
        app.begin_move_to_collection(1);
        // Select the "auth" folder entry (index 1; index 0 is the root label).
        app.move_folder_select_next();
        assert_eq!(app.move_pending.as_ref().unwrap().selected, 1);
        app.confirm_move_to_folder();
        assert_eq!(app.mode, Mode::Normal);

        // src emptied, dst gained the item *inside* the "auth" folder.
        assert!(store::load_collection(&src_path).unwrap().item.is_empty());
        let dst = store::load_collection(&dst_path).unwrap();
        let auth = dst.item.iter().find(|i| i.name == "auth").unwrap();
        let children = auth.item.as_ref().expect("auth is a folder");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "ping");
        // Root of dst has only the folder (no stray copy at root).
        assert_eq!(dst.item.len(), 1);
    }

    #[test]
    fn confirm_move_into_root_label_drops_item_at_root() {
        let dir = tempdir().unwrap();
        let (mut app, src_path, dst_path) = app_two_collections(&dir);
        app.selected = 1; // "ping"
        app.begin_move_to_collection(1);
        // Keep the default selection (index 0 = root label).
        app.confirm_move_to_folder();
        assert_eq!(app.mode, Mode::Normal);
        assert!(store::load_collection(&src_path).unwrap().item.is_empty());
        let dst = store::load_collection(&dst_path).unwrap();
        // "ping" lands at the root alongside the still-empty "auth" folder.
        assert!(dst
            .item
            .iter()
            .any(|i| i.name == "ping" && i.item.is_none()));
    }

    #[test]
    fn begin_move_skips_picker_when_dest_has_no_folders() {
        use std::fs;
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
        app.selected = 1; // "ping"
        app.begin_move_to_collection(1);
        // No folders → no picker; the move happens immediately.
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.move_pending.is_none());
        assert!(store::load_collection(&src_path).unwrap().item.is_empty());
        assert_eq!(
            store::load_collection(&dst_path).unwrap().item[0].name,
            "ping"
        );
    }

    #[test]
    fn collect_folder_names_is_recursive() {
        let coll: golden_core::model::Collection = serde_json::from_str(
            r#"{"info":{"name":"C"},"item":[
                {"name":"top","item":[{"name":"nested","item":[]}]},
                {"name":"req","request":{"method":"GET","url":"https://x"}}
            ]}"#,
        )
        .unwrap();
        let names = collect_folder_names(&coll.item);
        assert_eq!(names, vec!["top".to_string(), "nested".to_string()]);
    }
}
