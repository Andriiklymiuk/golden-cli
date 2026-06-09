//! Translate crossterm key events into App mutations.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::app::{App, Mode, Pane, ResponseTab};
use crate::tui::worker::{spawn_send, RunHandle, RunMsg, SendHandle, SendMsg};

/// Drain any pending message from the send worker and update App state.
/// Returns true if a message was processed (handle should be taken/cleared).
pub fn drain_send(app: &mut App, handle: &mut Option<SendHandle>) -> bool {
    let Some(h) = handle else { return false };
    match h.rx.try_recv() {
        Ok(msg) => {
            match msg {
                SendMsg::Done(out) => {
                    let out = *out;
                    app.last_assertions = out.result.assertions.clone();
                    if let Some(resp) = out.response {
                        app.last_response = Some(resp);
                        app.last_error = None;
                        app.last_script_error = out.result.error.clone();
                        app.response_scroll = 0;
                        app.response_tab = ResponseTab::Body;
                        // Reveal the response (matters on narrow layouts that show one
                        // detail pane at a time).
                        app.focus = Pane::Response;
                    } else {
                        app.last_error = out.result.error.clone();
                        app.last_response = None;
                        // A transport / pre-request-script failure has no response, but
                        // the Tests tab still shows the script error (and any pre-request
                        // assertions, kept above), so surface it here.
                        app.last_script_error = out.result.error.clone();
                        app.focus = Pane::Response;
                    }
                    app.status.clear();
                }
                SendMsg::Cancelled => {
                    app.status = "send cancelled".into();
                }
            }
            app.sending = false;
            *handle = None;
            true
        }
        Err(_) => false,
    }
}

/// Drain any pending message from the run worker and update App state.
pub fn drain_run(app: &mut App, handle: &mut Option<RunHandle>) {
    let Some(h) = handle else { return };
    if let Ok(RunMsg::Done(result)) = h.rx.try_recv() {
        app.run.done = app.run.total;
        app.run.running = false;
        app.run.result = Some(result);
        *handle = None;
    }
}

/// Fire the currently-selected request (shared by `s`, `Enter`, and chord-Enter).
fn send_current(app: &mut App, send_handle: &mut Option<SendHandle>) {
    // A send is already in flight: don't spawn a second worker (which would drop
    // the first handle without cancelling it, orphaning a thread until timeout).
    if app.sending {
        return;
    }
    if let Some((coll, target_path)) = app.selected_send_target() {
        app.sending = true;
        // Just the verb — the status bar owns the " · Esc cancel " suffix while
        // `app.sending` is true (avoids a doubled hint).
        app.status = "sending\u{2026}".into();
        app.last_assertions.clear();
        app.last_script_error = None;
        *send_handle = Some(spawn_send(
            coll,
            target_path,
            app.scopes.clone(),
            crate::tui::worker::tui_http_config(),
        ));
    } else {
        app.status = "select a request first".into();
    }
}

/// Run the node under the cursor: a request runs (with its tests), a folder runs
/// its subtree, a collection runs the whole collection — all via the run overlay.
fn run_selected(
    app: &mut App,
    send_handle: &mut Option<SendHandle>,
    run_handle: &mut Option<RunHandle>,
) {
    use crate::tui::tree::NodeKind;
    let Some(row) = app.current_row() else {
        app.status = "nothing selected".into();
        return;
    };
    match row.kind {
        NodeKind::Request => send_current(app, send_handle),
        NodeKind::Collection => {
            let ci = row.path[0];
            if let Some(lc) = app.collections.get(ci) {
                let coll = lc.collection.clone();
                start_run(app, run_handle, vec![coll]);
            }
        }
        NodeKind::Folder => {
            let path = row.path.clone();
            let ci = path[0];
            if let (Some(lc), Some(folder)) = (app.collections.get(ci), app.item_at(&path).cloned())
            {
                let parent = &lc.collection;
                let synthetic = golden_core::model::Collection {
                    info: parent.info.clone(),
                    variable: parent.variable.clone(),
                    item: vec![folder],
                    extra: parent.extra.clone(),
                };
                start_run(app, run_handle, vec![synthetic]);
            }
        }
    }
}

/// Open the run overlay and spawn a (cancellable) run over `colls`.
fn start_run(
    app: &mut App,
    run_handle: &mut Option<RunHandle>,
    colls: Vec<golden_core::model::Collection>,
) {
    // A run is already in flight: don't launch a second (which would drop the
    // first handle without cancelling it, orphaning its worker thread).
    if app.run.running || run_handle.is_some() {
        return;
    }
    let total: usize = colls.iter().map(crate::tui::worker::count_requests).sum();
    app.run = crate::tui::run_state::RunState {
        running: true,
        total,
        done: 0,
        result: None,
    };
    app.mode = Mode::Run;
    *run_handle = Some(crate::tui::worker::spawn_run(
        colls,
        app.scopes.clone(),
        1,
        crate::tui::worker::tui_http_config(),
    ));
}

/// Apply one key event to the app.
pub fn handle_key(
    app: &mut App,
    key: KeyEvent,
    send_handle: &mut Option<SendHandle>,
    run_handle: &mut Option<RunHandle>,
    workspace: &std::path::Path,
) {
    // First keypress dismisses the welcome hint.
    app.show_welcome = false;

    // Ctrl+C always quits, from any mode. In raw mode the terminal does NOT turn
    // Ctrl+C into a signal, so the app must handle it — otherwise it looks frozen.
    // This is the universal escape hatch (Esc only closes modals).
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.should_quit = true;
        return;
    }

    // Esc closes any modal overlay (search mode also clears the query;
    // edit mode discards the buffer).
    if key.code == KeyCode::Esc && app.mode != Mode::Normal {
        if app.mode == Mode::Search {
            app.search_query.clear();
        }
        if app.mode == Mode::Edit {
            app.edit = None;
        }
        if app.mode == Mode::Prompt {
            app.prompt = None;
        }
        if app.mode == Mode::Confirm {
            app.confirm = None;
            app.status = "cancelled".into();
        }
        if app.mode == Mode::Run {
            // Cancel an *in-flight* run (checked between requests) and report it.
            // A finished run still shows its results in Mode::Run; dismissing that
            // overlay must just close — "run cancelled" there would be misleading.
            let was_running = app.run.running;
            if let Some(h) = run_handle.take() {
                h.cancel();
            }
            app.run.running = false;
            if was_running {
                app.status = "run cancelled".into();
            }
        }
        app.mode = Mode::Normal;
        return;
    }

    // Edit mode key handling (single-field inline editor).
    if app.mode == Mode::Edit {
        match key.code {
            KeyCode::Enter => {
                app.commit_edit();
            }
            KeyCode::Backspace => {
                if let Some(s) = app.edit.as_mut() {
                    s.buffer.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(s) = app.edit.as_mut() {
                    s.buffer.push(c);
                }
            }
            _ => {}
        }
        return;
    }

    // Env switcher mode key handling.
    if app.mode == Mode::EnvSwitch {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if !app.env_profiles.is_empty() {
                    app.env_selected = (app.env_selected + 1).min(app.env_profiles.len() - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                app.env_selected = app.env_selected.saturating_sub(1);
            }
            KeyCode::Enter => {
                app.apply_selected_env(workspace);
                app.status = format!("env: {}", app.active_env);
                app.mode = Mode::Normal;
            }
            _ => {}
        }
        return;
    }

    // Search mode key handling.
    if app.mode == Mode::Search {
        match key.code {
            KeyCode::Enter => app.mode = Mode::Normal,
            KeyCode::Backspace => {
                app.search_query.pop();
            }
            KeyCode::Char(c) => app.search_query.push(c),
            _ => {}
        }
        return;
    }

    // Prompt mode key handling (single-line name input for tree CRUD).
    if app.mode == Mode::Prompt {
        match key.code {
            KeyCode::Enter => {
                app.commit_prompt();
            }
            KeyCode::Backspace => {
                if let Some(s) = app.prompt.as_mut() {
                    s.buffer.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(s) = app.prompt.as_mut() {
                    s.buffer.push(c);
                }
            }
            _ => {}
        }
        return;
    }

    // Confirm mode key handling (y/n for destructive ops).
    if app.mode == Mode::Confirm {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.execute_confirm();
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                app.confirm = None;
                app.status = "cancelled".into();
                app.mode = Mode::Normal;
            }
            _ => {}
        }
        return;
    }

    // MoveTarget mode key handling (collection picker for cross-collection move).
    if app.mode == crate::tui::app::Mode::MoveTarget {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if !app.collections.is_empty() {
                    app.move_target_selected =
                        (app.move_target_selected + 1).min(app.collections.len() - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                app.move_target_selected = app.move_target_selected.saturating_sub(1);
            }
            KeyCode::Enter => {
                let target = app.move_target_selected;
                app.mode = crate::tui::app::Mode::Normal;
                app.move_to_collection(target);
            }
            _ => {}
        }
        return;
    }

    if app.mode != Mode::Normal {
        return;
    }

    // While a send is in flight, Esc or `c` aborts it from ANY pane. Reset the UI
    // immediately and detach the worker thread (its timeout-bounded send finishes
    // into a dropped channel). This must run before the tree's `c`=duplicate.
    if app.sending && matches!(key.code, KeyCode::Esc | KeyCode::Char('c')) {
        if let Some(h) = send_handle.take() {
            h.cancel();
        }
        app.sending = false;
        app.status = "send cancelled".into();
        return;
    }

    // Ctrl+D = duplicate the selected item (works from any pane). Intercept before
    // the match so it doesn't fall through to `d` (= delete in the tree pane).
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('d') {
        if app.current_row().is_some() {
            app.start_duplicate();
        } else {
            app.status = "nothing selected".into();
        }
        return;
    }
    // Ctrl/Cmd/Alt+Enter = send from anywhere, for terminals that report the chord.
    // (Plain Enter on a request row also sends — see the match below.)
    if key.code == KeyCode::Enter
        && key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER | KeyModifiers::ALT)
    {
        send_current(app, send_handle);
        return;
    }

    match key.code {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Char('j') | KeyCode::Down => match app.focus {
            Pane::Tree => app.select_next(),
            Pane::Response => app.response_scroll = app.response_scroll.saturating_add(1),
            _ => {}
        },
        KeyCode::Char('k') | KeyCode::Up => match app.focus {
            Pane::Tree => app.select_prev(),
            Pane::Response => app.response_scroll = app.response_scroll.saturating_sub(1),
            _ => {}
        },
        KeyCode::Char('g') => app.select_first(),
        KeyCode::Char('G') => app.select_last(),
        KeyCode::Tab => app.cycle_pane(),
        // Enter on a request = send it (the intuitive gesture); on a folder/
        // collection = expand/collapse.
        KeyCode::Enter => {
            if app.current_request().is_some() {
                send_current(app, send_handle);
            } else {
                app.toggle_collapse();
            }
        }
        KeyCode::Char('s') => send_current(app, send_handle),
        // `c` = duplicate (copy) the selected item. (Cancelling an in-flight send is
        // handled by the earlier intercept while `app.sending` is true.)
        KeyCode::Char('c') => {
            if app.current_row().is_some() {
                app.start_duplicate();
            } else {
                app.status = "nothing selected".into();
            }
        }
        KeyCode::Char('t') => app.next_response_tab(),
        KeyCode::Char('x') => {
            app.refresh_env_profiles(workspace);
            app.mode = Mode::EnvSwitch;
        }
        KeyCode::Char('/') => app.mode = Mode::Search,
        KeyCode::Char('?') => app.mode = Mode::Help,
        // Cycle the focused request field (Method → Url → Headers → Body → Scripts).
        KeyCode::Char('f') => app.next_request_tab(),
        // Open an inline editor for the currently focused request field.
        KeyCode::Char('e') => {
            if !app.open_edit() {
                app.status = "select a request first".into();
            }
        }
        // Tree CRUD keys — only active when the tree pane is focused.
        KeyCode::Char('a') if app.focus == Pane::Tree => {
            if !app.open_add_request_prompt() {
                app.status = "select a collection or folder first".into();
            }
        }
        KeyCode::Char('A') if app.focus == Pane::Tree => {
            if !app.open_add_folder_prompt() {
                app.status = "select a collection or folder first".into();
            }
        }
        KeyCode::Char('d') if app.focus == Pane::Tree => {
            if !app.start_delete_confirm() {
                app.status = "nothing selected".into();
            }
        }
        // Reorder selected item one position down (]) or up ([) within its container.
        KeyCode::Char(']') if app.focus == Pane::Tree => {
            app.reorder_down();
        }
        KeyCode::Char('[') if app.focus == Pane::Tree => {
            app.reorder_up();
        }
        // `m` in the tree pane — move selected item to another collection.
        // Prompts via status and uses collection index from a pick overlay.
        // For now, wire to Mode::Move prompt: open the move-to-collection picker.
        KeyCode::Char('m') if app.focus == Pane::Tree => {
            app.open_move_prompt();
        }
        // `r` runs the selected node: a request runs (with tests), a folder runs its
        // subtree, a collection runs the whole collection.
        KeyCode::Char('r') => run_selected(app, send_handle, run_handle),
        // F2 = rename the selected tree row (works from any pane).
        KeyCode::F(2) => {
            if !app.open_rename_prompt() {
                app.status = "nothing selected".into();
            }
        }
        KeyCode::Char('R') => {
            let colls: Vec<_> = app
                .collections
                .iter()
                .map(|c| c.collection.clone())
                .collect();
            start_run(app, run_handle, colls);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::{App, Pane, ResponseTab};
    use crate::tui::loader::LoadedCollection;
    use crate::tui::worker::SendMsg;
    use golden_core::env::VarScopes;
    use golden_core::model::Collection;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn make_app(json: &str) -> App {
        let lc = LoadedCollection {
            path: "/tmp/x.json".into(),
            collection: serde_json::from_str::<Collection>(json).unwrap(),
        };
        App::new("/tmp".into(), vec![lc], VarScopes::default())
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
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
    fn ctrl_c_quits_from_any_mode() {
        for mode in [
            crate::tui::app::Mode::Normal,
            crate::tui::app::Mode::Edit,
            crate::tui::app::Mode::Search,
            crate::tui::app::Mode::Prompt,
        ] {
            let label = format!("{mode:?}");
            let mut app = make_app(J);
            app.mode = mode;
            handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                &mut None,
                &mut None,
                std::path::Path::new("/tmp"),
            );
            assert!(app.should_quit, "Ctrl+C should quit from mode {label}");
        }
    }

    #[test]
    fn first_key_dismisses_welcome() {
        let mut app = make_app(J);
        assert!(app.show_welcome, "welcome shows on open");
        handle_key(
            &mut app,
            key(KeyCode::Down),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert!(
            !app.show_welcome,
            "first keypress dismisses the welcome banner"
        );
    }

    #[test]
    fn enter_on_request_row_sends() {
        let (mut app, _dir) = make_app_with_file(J_FLAT);
        app.selected = 1; // the "ping" request row
        let mut handle: Option<SendHandle> = None;
        handle_key(
            &mut app,
            key(KeyCode::Enter),
            &mut handle,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert!(app.sending, "Enter on a request should start a send");
        assert!(handle.is_some(), "a send handle should be spawned");
    }

    #[test]
    fn ctrl_d_duplicates_and_does_not_quit() {
        let (mut app, _dir) = make_app_crud(J_ONE);
        app.selected = 1; // "ping"
        app.focus = Pane::Tree;
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert!(!app.should_quit, "Ctrl+D must not quit");
        assert!(
            app.rows.iter().any(|r| r.name == "ping (Copy)"),
            "Ctrl+D should duplicate the selected item"
        );
    }

    #[test]
    fn q_sets_should_quit() {
        let mut app = make_app(J);
        assert!(!app.should_quit);
        handle_key(
            &mut app,
            key(KeyCode::Char('q')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert!(app.should_quit);
    }

    #[test]
    fn j_moves_selection_down() {
        let mut app = make_app(J);
        assert_eq!(app.selected, 0);
        handle_key(
            &mut app,
            key(KeyCode::Char('j')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn k_moves_selection_up() {
        let mut app = make_app(J);
        app.selected = 2;
        handle_key(
            &mut app,
            key(KeyCode::Char('k')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn down_arrow_moves_selection_down() {
        let mut app = make_app(J);
        assert_eq!(app.selected, 0);
        handle_key(
            &mut app,
            key(KeyCode::Down),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn up_arrow_moves_selection_up() {
        let mut app = make_app(J);
        app.selected = 2;
        handle_key(
            &mut app,
            key(KeyCode::Up),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn g_goes_to_first() {
        let mut app = make_app(J);
        app.selected = 3;
        handle_key(
            &mut app,
            key(KeyCode::Char('g')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn capital_g_goes_to_last() {
        let mut app = make_app(J);
        assert_eq!(app.selected, 0);
        handle_key(
            &mut app,
            key(KeyCode::Char('G')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.selected, app.rows.len() - 1);
    }

    #[test]
    fn tab_cycles_pane_focus() {
        let mut app = make_app(J);
        assert_eq!(app.focus, Pane::Tree);
        handle_key(
            &mut app,
            key(KeyCode::Tab),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.focus, Pane::Request);
        handle_key(
            &mut app,
            key(KeyCode::Tab),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.focus, Pane::Response);
        handle_key(
            &mut app,
            key(KeyCode::Tab),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.focus, Pane::Tree);
    }

    #[test]
    fn enter_toggles_collapse_on_folder() {
        let mut app = make_app(J);
        // Row 0 = collection header (has children)
        assert_eq!(app.selected, 0);
        let initial_rows = app.rows.len();
        handle_key(
            &mut app,
            key(KeyCode::Enter),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        ); // collapse collection
        assert!(
            app.rows.len() < initial_rows,
            "collapsing should reduce visible rows"
        );
        handle_key(
            &mut app,
            key(KeyCode::Enter),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        ); // expand again
        assert_eq!(
            app.rows.len(),
            initial_rows,
            "expanding should restore rows"
        );
    }

    #[test]
    fn j_does_not_move_when_focus_is_not_tree() {
        let mut app = make_app(J);
        app.focus = Pane::Request;
        app.selected = 0;
        handle_key(
            &mut app,
            key(KeyCode::Char('j')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(
            app.selected, 0,
            "j should not move selection when focus is not Tree"
        );
    }

    #[test]
    fn k_does_not_move_when_focus_is_not_tree() {
        let mut app = make_app(J);
        app.focus = Pane::Response;
        app.selected = 2;
        handle_key(
            &mut app,
            key(KeyCode::Char('k')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(
            app.selected, 2,
            "k should not move selection when focus is not Tree"
        );
    }

    #[test]
    fn keys_are_ignored_in_non_normal_mode() {
        let mut app = make_app(J);
        app.mode = crate::tui::app::Mode::Help;
        handle_key(
            &mut app,
            key(KeyCode::Char('q')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert!(
            !app.should_quit,
            "keys should be ignored in non-Normal mode"
        );
    }

    #[test]
    fn question_mark_opens_help_modal() {
        let mut app = make_app(J);
        assert_eq!(app.mode, crate::tui::app::Mode::Normal);
        handle_key(
            &mut app,
            key(KeyCode::Char('?')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(
            app.mode,
            crate::tui::app::Mode::Help,
            "? should open help modal"
        );
    }

    #[test]
    fn esc_closes_help_modal() {
        let mut app = make_app(J);
        app.mode = crate::tui::app::Mode::Help;
        handle_key(
            &mut app,
            key(KeyCode::Esc),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(
            app.mode,
            crate::tui::app::Mode::Normal,
            "esc should close help modal"
        );
    }

    #[test]
    fn s_on_folder_row_sets_status_message() {
        let mut app = make_app(J);
        // row 0 is the collection root (a folder, no request)
        app.selected = 0;
        let mut handle: Option<SendHandle> = None;
        handle_key(
            &mut app,
            key(KeyCode::Char('s')),
            &mut handle,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        // No request selected → should not set sending=true
        assert!(!app.sending);
        assert_eq!(app.status, "select a request first");
    }

    #[test]
    fn t_cycles_response_tab() {
        let mut app = make_app(J);
        assert_eq!(app.response_tab, ResponseTab::Body);
        handle_key(
            &mut app,
            key(KeyCode::Char('t')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.response_tab, ResponseTab::Headers);
        handle_key(
            &mut app,
            key(KeyCode::Char('t')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.response_tab, ResponseTab::Cookies);
        handle_key(
            &mut app,
            key(KeyCode::Char('t')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.response_tab, ResponseTab::Tests);
        handle_key(
            &mut app,
            key(KeyCode::Char('t')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.response_tab, ResponseTab::Body);
    }

    fn outcome_ok(status: u16) -> golden_core::runner::SingleOutcome {
        golden_core::runner::SingleOutcome {
            response: Some(golden_core::http::HttpResponse {
                status,
                headers: vec![],
                body: b"ok".to_vec(),
                time_ms: 5,
            }),
            result: golden_core::result::RequestResult {
                name: "a".into(),
                method: "GET".into(),
                url: "http://x".into(),
                status: Some(status),
                time_ms: 5,
                assertions: vec![],
                error: None,
            },
        }
    }

    fn outcome_err(msg: &str) -> golden_core::runner::SingleOutcome {
        golden_core::runner::SingleOutcome {
            response: None,
            result: golden_core::result::RequestResult {
                name: "a".into(),
                method: "GET".into(),
                url: "http://x".into(),
                status: None,
                time_ms: 0,
                assertions: vec![],
                error: Some(msg.into()),
            },
        }
    }

    #[test]
    fn drain_send_processes_done_ok_message() {
        use std::sync::mpsc;
        use std::sync::{atomic::AtomicBool, Arc};

        let (tx, rx) = mpsc::channel::<SendMsg>();
        tx.send(SendMsg::Done(Box::new(outcome_ok(200)))).unwrap();

        let mut app = make_app(J);
        app.sending = true;
        let mut handle: Option<SendHandle> = Some(SendHandle {
            rx,
            cancel: Arc::new(AtomicBool::new(false)),
            join: None,
        });
        let processed = drain_send(&mut app, &mut handle);
        assert!(processed);
        assert!(!app.sending);
        assert!(handle.is_none());
        assert_eq!(app.last_response.as_ref().unwrap().status, 200);
        assert_eq!(app.response_tab, ResponseTab::Body);
    }

    #[test]
    fn drain_send_processes_done_err_message() {
        use std::sync::mpsc;
        use std::sync::{atomic::AtomicBool, Arc};

        let (tx, rx) = mpsc::channel::<SendMsg>();
        tx.send(SendMsg::Done(Box::new(outcome_err("connection refused"))))
            .unwrap();

        let mut app = make_app(J);
        app.sending = true;
        let mut handle: Option<SendHandle> = Some(SendHandle {
            rx,
            cancel: Arc::new(AtomicBool::new(false)),
            join: None,
        });
        drain_send(&mut app, &mut handle);
        assert!(!app.sending);
        assert!(app.last_response.is_none());
        assert_eq!(app.last_error.as_deref(), Some("connection refused"));
        // The script/transport error is also surfaced on the Tests tab (which now
        // renders even with no response), so last_script_error must be populated.
        assert_eq!(app.last_script_error.as_deref(), Some("connection refused"));
        // The stale "sending…" status must be cleared after completion.
        assert!(app.status.is_empty(), "status cleared, got: {}", app.status);
    }

    #[test]
    fn drain_send_processes_cancelled_message() {
        use std::sync::mpsc;
        use std::sync::{atomic::AtomicBool, Arc};

        let (tx, rx) = mpsc::channel::<SendMsg>();
        tx.send(SendMsg::Cancelled).unwrap();

        let mut app = make_app(J);
        app.sending = true;
        let mut handle: Option<SendHandle> = Some(SendHandle {
            rx,
            cancel: Arc::new(AtomicBool::new(false)),
            join: None,
        });
        drain_send(&mut app, &mut handle);
        assert!(!app.sending);
        assert_eq!(app.status, "send cancelled");
    }

    #[test]
    fn drain_send_returns_false_when_no_message() {
        use std::sync::mpsc;
        use std::sync::{atomic::AtomicBool, Arc};

        let (_tx, rx) = mpsc::channel::<SendMsg>();
        let mut app = make_app(J);
        let mut handle: Option<SendHandle> = Some(SendHandle {
            rx,
            cancel: Arc::new(AtomicBool::new(false)),
            join: None,
        });
        let processed = drain_send(&mut app, &mut handle);
        assert!(!processed);
        // handle is still present since nothing was received
        assert!(handle.is_some());
    }

    #[test]
    fn j_scrolls_response_when_focus_is_response() {
        let mut app = make_app(J);
        app.focus = Pane::Response;
        app.response_scroll = 0;
        handle_key(
            &mut app,
            key(KeyCode::Char('j')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.response_scroll, 1);
        handle_key(
            &mut app,
            key(KeyCode::Char('k')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.response_scroll, 0);
        // saturating sub should not underflow
        handle_key(
            &mut app,
            key(KeyCode::Char('k')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.response_scroll, 0);
    }

    #[test]
    fn r_opens_run_overlay_for_current_collection() {
        use crate::tui::app::Mode;
        // J has a collection at index 0; select row 0 (the collection header).
        // `r` on a collection runs the whole collection (from any pane).
        let mut app = make_app(J);
        app.selected = 0;
        let mut run_handle = None;
        handle_key(
            &mut app,
            key(KeyCode::Char('r')),
            &mut None,
            &mut run_handle,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.mode, Mode::Run);
        assert!(app.run.running);
        assert!(run_handle.is_some());
    }

    #[test]
    fn r_on_folder_row_runs_only_folder_subtree() {
        use crate::tui::app::Mode;
        // J: collection "Sample" with folder "auth" (1 request "login") and a
        // sibling request "ping" outside the folder. Row 1 is the "auth" folder.
        let mut app = make_app(J);
        app.selected = 1; // the "auth" folder row
        assert_eq!(
            app.current_row().map(|r| r.name.clone()),
            Some("auth".to_string()),
            "row 1 should be the auth folder"
        );
        let mut run_handle = None;
        handle_key(
            &mut app,
            key(KeyCode::Char('r')),
            &mut None,
            &mut run_handle,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.mode, Mode::Run);
        assert!(run_handle.is_some(), "a run handle should be spawned");
        // Only the folder's leaf requests count toward the total (1 "login"),
        // NOT the sibling "ping" outside the folder.
        assert_eq!(
            app.run.total, 1,
            "folder run should cover only the folder subtree, got {}",
            app.run.total
        );
    }

    #[test]
    fn r_on_request_row_starts_send() {
        let (mut app, _dir) = make_app_with_file(J_FLAT);
        app.selected = 1; // the "ping" request row
        let mut send_handle: Option<SendHandle> = None;
        handle_key(
            &mut app,
            key(KeyCode::Char('r')),
            &mut send_handle,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert!(app.sending, "r on a request should start a send");
        assert!(send_handle.is_some(), "a send handle should be spawned");
    }

    #[test]
    fn capital_r_opens_run_overlay_for_all_collections() {
        use crate::tui::app::Mode;
        let mut app = make_app(J);
        let mut run_handle = None;
        handle_key(
            &mut app,
            key(KeyCode::Char('R')),
            &mut None,
            &mut run_handle,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.mode, Mode::Run);
        assert!(app.run.running);
        assert!(run_handle.is_some());
    }

    #[test]
    fn esc_closes_run_overlay() {
        use crate::tui::app::Mode;
        let mut app = make_app(J);
        app.mode = Mode::Run;
        handle_key(
            &mut app,
            key(KeyCode::Esc),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn esc_cancels_in_flight_run() {
        use crate::tui::app::Mode;
        use std::sync::{atomic::AtomicBool, mpsc, Arc};

        let mut app = make_app(J);
        app.mode = Mode::Run;
        app.run.running = true;

        // A real, live run handle so Esc exercises the cancel branch.
        let (_tx, rx) = mpsc::channel::<RunMsg>();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let mut run_handle: Option<RunHandle> = Some(RunHandle {
            rx,
            cancel: cancel_flag.clone(),
            join: None,
        });

        handle_key(
            &mut app,
            key(KeyCode::Esc),
            &mut None,
            &mut run_handle,
            std::path::Path::new("/tmp"),
        );

        assert!(
            cancel_flag.load(std::sync::atomic::Ordering::SeqCst),
            "Esc should signal the run's cancel flag"
        );
        assert!(!app.run.running, "run.running should be reset");
        assert_eq!(app.mode, Mode::Normal, "overlay should close");
        assert_eq!(app.status, "run cancelled");
        assert!(run_handle.is_none(), "the run handle should be taken");
    }

    #[test]
    fn drain_run_processes_done_message() {
        use golden_core::result::{RunResult, Totals};
        use std::sync::mpsc;

        let (tx, rx) = mpsc::channel::<RunMsg>();
        let result = RunResult {
            collections: vec![],
            totals: Totals {
                requests: 3,
                ..Default::default()
            },
        };
        tx.send(RunMsg::Done(result)).unwrap();

        let mut app = make_app(J);
        app.run.running = true;
        app.run.total = 3;
        let mut handle: Option<RunHandle> = Some(RunHandle {
            rx,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            join: None,
        });
        drain_run(&mut app, &mut handle);
        assert!(!app.run.running);
        assert_eq!(app.run.done, 3);
        assert!(app.run.result.is_some());
        assert!(handle.is_none());
    }

    #[test]
    fn drain_run_does_nothing_when_no_message() {
        use std::sync::mpsc;
        let (_tx, rx) = mpsc::channel::<RunMsg>();
        let mut app = make_app(J);
        app.run.running = true;
        let mut handle: Option<RunHandle> = Some(RunHandle {
            rx,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            join: None,
        });
        drain_run(&mut app, &mut handle);
        assert!(app.run.running); // unchanged
        assert!(handle.is_some()); // still present
    }

    #[test]
    fn x_opens_env_switcher_overlay() {
        use crate::tui::app::Mode;
        use std::fs;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".env"), "A=1").unwrap();
        let lc = LoadedCollection {
            path: dir.path().join("x.json"),
            collection: serde_json::from_str::<Collection>(J).unwrap(),
        };
        let mut app = App::new(dir.path().into(), vec![lc], VarScopes::default());
        handle_key(
            &mut app,
            key(KeyCode::Char('x')),
            &mut None,
            &mut None,
            dir.path(),
        );
        assert_eq!(app.mode, Mode::EnvSwitch);
        assert!(!app.env_profiles.is_empty());
    }

    #[test]
    fn env_switcher_jk_navigate_profiles() {
        use crate::tui::app::Mode;
        use std::fs;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".env"), "A=1").unwrap();
        fs::write(dir.path().join(".env.prod"), "A=2").unwrap();
        let lc = LoadedCollection {
            path: dir.path().join("x.json"),
            collection: serde_json::from_str::<Collection>(J).unwrap(),
        };
        let mut app = App::new(dir.path().into(), vec![lc], VarScopes::default());
        app.mode = Mode::EnvSwitch;
        app.refresh_env_profiles(dir.path());
        assert_eq!(app.env_selected, 0);
        handle_key(
            &mut app,
            key(KeyCode::Char('j')),
            &mut None,
            &mut None,
            dir.path(),
        );
        assert_eq!(app.env_selected, 1);
        handle_key(
            &mut app,
            key(KeyCode::Char('k')),
            &mut None,
            &mut None,
            dir.path(),
        );
        assert_eq!(app.env_selected, 0);
    }

    #[test]
    fn env_switcher_enter_applies_and_closes() {
        use crate::tui::app::Mode;
        use std::fs;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".env"), "HOST=base").unwrap();
        fs::write(dir.path().join(".env.staging"), "HOST=staging").unwrap();
        let lc = LoadedCollection {
            path: dir.path().join("x.json"),
            collection: serde_json::from_str::<Collection>(J).unwrap(),
        };
        let mut app = App::new(dir.path().into(), vec![lc], VarScopes::default());
        app.mode = Mode::EnvSwitch;
        app.refresh_env_profiles(dir.path());
        app.env_selected = 1; // select staging
        handle_key(
            &mut app,
            key(KeyCode::Enter),
            &mut None,
            &mut None,
            dir.path(),
        );
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.active_env, "staging");
        assert_eq!(app.scopes.get("HOST").map(String::as_str), Some("staging"));
    }

    #[test]
    fn esc_closes_env_switcher() {
        use crate::tui::app::Mode;
        let mut app = make_app(J);
        app.mode = Mode::EnvSwitch;
        handle_key(
            &mut app,
            key(KeyCode::Esc),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.mode, Mode::Normal);
    }

    // ── edit mode tests ──────────────────────────────────────────────────────

    fn make_app_with_file(json: &str) -> (App, tempfile::TempDir) {
        use std::fs;
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let path = dir.path().join("coll.json");
        fs::write(&path, json).unwrap();
        let lc = LoadedCollection {
            path,
            collection: serde_json::from_str::<Collection>(json).unwrap(),
        };
        let app = App::new(dir.path().into(), vec![lc], VarScopes::default());
        (app, dir)
    }

    /// J_FLAT: a collection with a top-level request (row 1 = "ping", path=[0,0]).
    const J_FLAT: &str = r#"{
      "info": { "name": "Flat" },
      "item": [
        { "name": "ping", "request": { "method": "GET", "url": "http://x/ping", "header": [] } }
      ]
    }"#;

    #[test]
    fn e_on_request_row_opens_edit_mode() {
        use crate::tui::app::Mode;
        let (mut app, _dir) = make_app_with_file(J_FLAT);
        // row 1 = "ping" request
        app.selected = 1;
        handle_key(
            &mut app,
            key(KeyCode::Char('e')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(
            app.mode,
            Mode::Edit,
            "e should open Edit mode on request row"
        );
        assert!(app.edit.is_some(), "edit session should be set");
    }

    #[test]
    fn e_on_folder_row_sets_status_not_edit_mode() {
        use crate::tui::app::Mode;
        let (mut app, _dir) = make_app_with_file(J);
        // row 0 is the collection header (folder-like)
        app.selected = 0;
        handle_key(
            &mut app,
            key(KeyCode::Char('e')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(
            app.mode,
            Mode::Normal,
            "e on folder should NOT open Edit mode"
        );
        assert_eq!(app.status, "select a request first");
    }

    #[test]
    fn esc_in_edit_mode_cancels_without_saving() {
        use crate::tui::app::Mode;
        let (mut app, _dir) = make_app_with_file(J_FLAT);
        app.selected = 1;
        // Open edit mode
        handle_key(
            &mut app,
            key(KeyCode::Char('e')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.mode, Mode::Edit);
        // Type something
        handle_key(
            &mut app,
            key(KeyCode::Char('P')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        // Cancel
        handle_key(
            &mut app,
            key(KeyCode::Esc),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.mode, Mode::Normal, "Esc should close Edit mode");
        assert!(app.edit.is_none(), "edit session should be cleared on Esc");
        // Original method unchanged
        assert_eq!(
            app.collections[0].collection.item[0]
                .request
                .as_ref()
                .unwrap()
                .method,
            "GET"
        );
    }

    #[test]
    fn typing_in_edit_mode_appends_to_buffer() {
        use crate::tui::app::Mode;
        let (mut app, _dir) = make_app_with_file(J_FLAT);
        app.selected = 1;
        handle_key(
            &mut app,
            key(KeyCode::Char('e')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.mode, Mode::Edit);
        // buffer starts with "GET"
        handle_key(
            &mut app,
            key(KeyCode::Char('X')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        let buf = app.edit.as_ref().unwrap().buffer.clone();
        assert!(
            buf.ends_with('X'),
            "typing should append to buffer, got: {buf}"
        );
    }

    #[test]
    fn backspace_in_edit_mode_removes_last_char() {
        let (mut app, _dir) = make_app_with_file(J_FLAT);
        app.selected = 1;
        handle_key(
            &mut app,
            key(KeyCode::Char('e')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        // buffer = "GET"; backspace once → "GE"
        handle_key(
            &mut app,
            key(KeyCode::Backspace),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        let buf = app.edit.as_ref().unwrap().buffer.clone();
        assert_eq!(buf, "GE", "backspace should remove last char, got: {buf}");
    }

    #[test]
    fn enter_in_edit_mode_commits_method_and_saves_to_disk() {
        use crate::tui::app::Mode;
        use golden_core::store::load_collection;

        let (mut app, dir) = make_app_with_file(J_FLAT);
        let coll_path = dir.path().join("coll.json");
        app.selected = 1;

        // Open edit, clear "GET", type "DELETE", commit.
        handle_key(
            &mut app,
            key(KeyCode::Char('e')),
            &mut None,
            &mut None,
            dir.path(),
        );
        // Clear existing buffer ("GET") with backspaces
        for _ in 0..3 {
            handle_key(
                &mut app,
                key(KeyCode::Backspace),
                &mut None,
                &mut None,
                dir.path(),
            );
        }
        for ch in "DELETE".chars() {
            handle_key(
                &mut app,
                key(KeyCode::Char(ch)),
                &mut None,
                &mut None,
                dir.path(),
            );
        }
        assert_eq!(app.mode, Mode::Edit);
        handle_key(
            &mut app,
            key(KeyCode::Enter),
            &mut None,
            &mut None,
            dir.path(),
        );

        assert_eq!(app.mode, Mode::Normal, "Enter should close Edit mode");
        assert!(app.edit.is_none());
        assert_eq!(app.status, "saved");

        // In-memory collection updated.
        assert_eq!(
            app.collections[0].collection.item[0]
                .request
                .as_ref()
                .unwrap()
                .method,
            "DELETE"
        );

        // On-disk file updated.
        let reloaded = load_collection(&coll_path).unwrap();
        assert_eq!(reloaded.item[0].request.as_ref().unwrap().method, "DELETE");
    }

    #[test]
    fn f_cycles_request_tab() {
        use crate::tui::app::RequestTab;
        let mut app = make_app(J);
        assert_eq!(app.request_tab, RequestTab::Method);
        handle_key(
            &mut app,
            key(KeyCode::Char('f')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.request_tab, RequestTab::Url);
        handle_key(
            &mut app,
            key(KeyCode::Char('f')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.request_tab, RequestTab::Headers);
        handle_key(
            &mut app,
            key(KeyCode::Char('f')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.request_tab, RequestTab::Body);
        handle_key(
            &mut app,
            key(KeyCode::Char('f')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.request_tab, RequestTab::PreRequestScript);
        handle_key(
            &mut app,
            key(KeyCode::Char('f')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.request_tab, RequestTab::TestScript);
        handle_key(
            &mut app,
            key(KeyCode::Char('f')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(
            app.request_tab,
            RequestTab::Method,
            "should wrap back to Method"
        );
    }

    #[test]
    fn e_on_url_tab_opens_url_edit() {
        use crate::tui::app::{Mode, RequestTab};
        use crate::tui::edit::EditField;
        let (mut app, _dir) = make_app_with_file(J_FLAT);
        app.selected = 1;
        app.request_tab = RequestTab::Url;
        handle_key(
            &mut app,
            key(KeyCode::Char('e')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.mode, Mode::Edit);
        let session = app.edit.as_ref().unwrap();
        assert_eq!(session.field, EditField::Url);
        assert_eq!(session.buffer, "http://x/ping");
    }

    #[test]
    fn enter_in_edit_mode_with_bad_headers_json_sets_error_status() {
        use crate::tui::app::{Mode, RequestTab};
        let (mut app, _dir) = make_app_with_file(J_FLAT);
        app.selected = 1;
        app.request_tab = RequestTab::Headers;
        handle_key(
            &mut app,
            key(KeyCode::Char('e')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        // Clear existing buffer and type invalid JSON
        let buf_len = app.edit.as_ref().unwrap().buffer.len();
        for _ in 0..buf_len {
            handle_key(
                &mut app,
                key(KeyCode::Backspace),
                &mut None,
                &mut None,
                std::path::Path::new("/tmp"),
            );
        }
        for ch in "notjson".chars() {
            handle_key(
                &mut app,
                key(KeyCode::Char(ch)),
                &mut None,
                &mut None,
                std::path::Path::new("/tmp"),
            );
        }
        handle_key(
            &mut app,
            key(KeyCode::Enter),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.mode, Mode::Normal);
        assert!(
            app.status.contains("edit failed"),
            "bad JSON should set error status, got: {}",
            app.status
        );
    }

    // ── tree CRUD key event tests ──────────────────────────────────────────

    /// Build an App backed by a real temp-dir collection file.
    fn make_app_crud(json: &str) -> (App, tempfile::TempDir) {
        use std::fs;
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.json");
        fs::write(&path, json).unwrap();
        let lc = crate::tui::loader::LoadedCollection {
            path,
            collection: serde_json::from_str::<golden_core::model::Collection>(json).unwrap(),
        };
        let app = App::new(dir.path().into(), vec![lc], VarScopes::default());
        (app, dir)
    }

    const J_EMPTY: &str = r#"{"info": {"name": "Empty"}, "item": []}"#;
    const J_ONE: &str = r#"{
      "info": {"name": "One"},
      "item": [
        {"name": "ping", "request": {"method": "GET", "url": "https://x/ping"}}
      ]
    }"#;

    #[test]
    fn a_key_opens_add_request_prompt_in_tree_pane() {
        use crate::tui::app::Mode;
        let (mut app, _dir) = make_app_crud(J_EMPTY);
        app.selected = 0; // collection row
                          // focus is Pane::Tree by default
        handle_key(
            &mut app,
            key(KeyCode::Char('a')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(
            app.mode,
            Mode::Prompt,
            "a should open Prompt mode in tree pane"
        );
        assert!(app.prompt.is_some(), "prompt session should be set");
    }

    #[test]
    fn a_key_does_nothing_outside_tree_pane() {
        use crate::tui::app::{Mode, Pane};
        let (mut app, _dir) = make_app_crud(J_EMPTY);
        app.focus = Pane::Request;
        handle_key(
            &mut app,
            key(KeyCode::Char('a')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(
            app.mode,
            Mode::Normal,
            "a outside tree pane should not open prompt"
        );
    }

    #[test]
    fn capital_a_opens_add_folder_prompt() {
        use crate::tui::app::Mode;
        let (mut app, _dir) = make_app_crud(J_EMPTY);
        app.selected = 0;
        handle_key(
            &mut app,
            key(KeyCode::Char('A')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.mode, Mode::Prompt);
        let sess = app.prompt.as_ref().unwrap();
        assert!(
            sess.title.contains("folder"),
            "A should prompt for folder name, got: {}",
            sess.title
        );
    }

    #[test]
    fn a_then_name_then_enter_adds_request_to_tree_and_file() {
        use crate::tui::app::Mode;
        use golden_core::store;

        let (mut app, dir) = make_app_crud(J_EMPTY);
        app.selected = 0; // collection row
        let coll_path = dir.path().join("c.json");

        // Press 'a' to start prompt
        handle_key(
            &mut app,
            key(KeyCode::Char('a')),
            &mut None,
            &mut None,
            dir.path(),
        );
        assert_eq!(app.mode, Mode::Prompt);

        // Type "newreq"
        for ch in "newreq".chars() {
            handle_key(
                &mut app,
                key(KeyCode::Char(ch)),
                &mut None,
                &mut None,
                dir.path(),
            );
        }
        // Press Enter to commit
        handle_key(
            &mut app,
            key(KeyCode::Enter),
            &mut None,
            &mut None,
            dir.path(),
        );

        assert_eq!(app.mode, Mode::Normal);
        assert!(
            app.rows.iter().any(|r| r.name == "newreq"),
            "newreq should appear in tree rows"
        );
        let reloaded = store::load_collection(&coll_path).unwrap();
        assert_eq!(reloaded.item[0].name, "newreq");
    }

    #[test]
    fn esc_in_prompt_mode_cancels_add() {
        use crate::tui::app::Mode;
        let (mut app, _dir) = make_app_crud(J_EMPTY);
        app.selected = 0;

        handle_key(
            &mut app,
            key(KeyCode::Char('a')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.mode, Mode::Prompt);

        // Type something then Esc
        handle_key(
            &mut app,
            key(KeyCode::Char('x')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        handle_key(
            &mut app,
            key(KeyCode::Esc),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );

        assert_eq!(app.mode, Mode::Normal);
        assert!(app.prompt.is_none());
        assert_eq!(app.collections[0].collection.item.len(), 0);
    }

    #[test]
    fn d_key_opens_confirm_mode() {
        use crate::tui::app::Mode;
        let (mut app, _dir) = make_app_crud(J_ONE);
        app.selected = 1; // "ping" request row

        handle_key(
            &mut app,
            key(KeyCode::Char('d')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.mode, Mode::Confirm);
        let msg = app.confirm.as_ref().unwrap().message.clone();
        assert!(
            msg.contains("ping"),
            "confirm message should mention item name, got: {msg}"
        );
    }

    #[test]
    fn d_then_y_deletes_request() {
        use crate::tui::app::Mode;
        use golden_core::store;

        let (mut app, dir) = make_app_crud(J_ONE);
        app.selected = 1; // "ping"
        let coll_path = dir.path().join("c.json");

        handle_key(
            &mut app,
            key(KeyCode::Char('d')),
            &mut None,
            &mut None,
            dir.path(),
        );
        assert_eq!(app.mode, Mode::Confirm);

        handle_key(
            &mut app,
            key(KeyCode::Char('y')),
            &mut None,
            &mut None,
            dir.path(),
        );
        assert_eq!(app.mode, Mode::Normal);
        assert!(
            !app.rows.iter().any(|r| r.name == "ping"),
            "ping should be gone from tree"
        );
        let reloaded = store::load_collection(&coll_path).unwrap();
        assert!(reloaded.item.is_empty(), "collection file should be empty");
    }

    #[test]
    fn d_then_n_cancels_delete() {
        use crate::tui::app::Mode;
        let (mut app, _dir) = make_app_crud(J_ONE);
        app.selected = 1;

        handle_key(
            &mut app,
            key(KeyCode::Char('d')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        handle_key(
            &mut app,
            key(KeyCode::Char('n')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );

        assert_eq!(app.mode, Mode::Normal);
        assert!(
            app.rows.iter().any(|r| r.name == "ping"),
            "ping should still be there"
        );
    }

    #[test]
    fn esc_in_confirm_mode_cancels_delete() {
        use crate::tui::app::Mode;
        let (mut app, _dir) = make_app_crud(J_ONE);
        app.selected = 1;

        handle_key(
            &mut app,
            key(KeyCode::Char('d')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.mode, Mode::Confirm);

        handle_key(
            &mut app,
            key(KeyCode::Esc),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.mode, Mode::Normal);
        assert!(
            app.rows.iter().any(|r| r.name == "ping"),
            "ping should still exist"
        );
    }

    #[test]
    fn f2_key_in_tree_pane_opens_rename_prompt() {
        use crate::tui::app::{Mode, Pane};
        let (mut app, _dir) = make_app_crud(J_ONE);
        app.selected = 1; // "ping"
        app.focus = Pane::Tree;

        handle_key(
            &mut app,
            key(KeyCode::F(2)),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.mode, Mode::Prompt);
        let sess = app.prompt.as_ref().unwrap();
        assert_eq!(
            sess.buffer, "ping",
            "rename prompt pre-filled with current name"
        );
    }

    #[test]
    fn f2_then_name_then_enter_renames_request() {
        use crate::tui::app::Mode;
        use golden_core::store;

        let (mut app, dir) = make_app_crud(J_ONE);
        app.selected = 1; // "ping"
        let coll_path = dir.path().join("c.json");

        handle_key(
            &mut app,
            key(KeyCode::F(2)),
            &mut None,
            &mut None,
            dir.path(),
        );
        // Clear existing buffer (pre-filled with "ping")
        for _ in 0..4 {
            handle_key(
                &mut app,
                key(KeyCode::Backspace),
                &mut None,
                &mut None,
                dir.path(),
            );
        }
        for ch in "pong".chars() {
            handle_key(
                &mut app,
                key(KeyCode::Char(ch)),
                &mut None,
                &mut None,
                dir.path(),
            );
        }
        handle_key(
            &mut app,
            key(KeyCode::Enter),
            &mut None,
            &mut None,
            dir.path(),
        );

        assert_eq!(app.mode, Mode::Normal);
        assert!(
            app.rows.iter().any(|r| r.name == "pong"),
            "pong should be in tree"
        );
        assert!(
            !app.rows.iter().any(|r| r.name == "ping"),
            "ping should be gone"
        );
        let reloaded = store::load_collection(&coll_path).unwrap();
        assert_eq!(reloaded.item[0].name, "pong");
    }

    #[test]
    fn c_key_in_tree_pane_duplicates_request() {
        use crate::tui::app::Pane;
        use golden_core::store;

        let (mut app, dir) = make_app_crud(J_ONE);
        app.selected = 1; // "ping"
        app.focus = Pane::Tree;
        let coll_path = dir.path().join("c.json");

        handle_key(
            &mut app,
            key(KeyCode::Char('c')),
            &mut None,
            &mut None,
            dir.path(),
        );

        assert!(
            app.rows.iter().any(|r| r.name == "ping (Copy)"),
            "ping (Copy) should appear in tree"
        );
        let reloaded = store::load_collection(&coll_path).unwrap();
        assert_eq!(reloaded.item.len(), 2);
        assert_eq!(reloaded.item[1].name, "ping (Copy)");
    }

    #[test]
    fn c_while_sending_cancels_send_not_duplicate() {
        use crate::tui::app::Pane;
        use std::sync::{atomic::AtomicBool, mpsc, Arc};

        let (mut app, _dir) = make_app_crud(J_ONE);
        app.selected = 1;
        app.focus = Pane::Response; // NOT tree pane
        app.sending = true; // a send is in flight → `c` aborts it

        // Provide an active send handle so the 'c' = cancel path is taken.
        let (_tx, rx) = mpsc::channel::<crate::tui::worker::SendMsg>();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let mut handle: Option<crate::tui::worker::SendHandle> =
            Some(crate::tui::worker::SendHandle {
                rx,
                cancel: cancel_flag.clone(),
                join: None,
            });

        handle_key(
            &mut app,
            key(KeyCode::Char('c')),
            &mut handle,
            &mut None,
            std::path::Path::new("/tmp"),
        );

        // The cancel flag should be set (cancel path taken).
        assert!(
            cancel_flag.load(std::sync::atomic::Ordering::Relaxed),
            "cancel flag should be set"
        );
        assert!(!app.sending, "sending should be reset after cancel");
        assert!(handle.is_none(), "send handle should be detached on cancel");
        // No duplicate in tree.
        assert!(
            !app.rows.iter().any(|r| r.name == "ping (Copy)"),
            "c while sending should NOT duplicate"
        );
    }

    #[test]
    fn esc_while_sending_cancels_send() {
        use std::sync::{atomic::AtomicBool, mpsc, Arc};

        let mut app = make_app(J);
        app.mode = crate::tui::app::Mode::Normal;
        app.sending = true; // a send is in flight → Esc aborts it

        let (_tx, rx) = mpsc::channel::<crate::tui::worker::SendMsg>();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let mut handle: Option<SendHandle> = Some(SendHandle {
            rx,
            cancel: cancel_flag.clone(),
            join: None,
        });

        handle_key(
            &mut app,
            key(KeyCode::Esc),
            &mut handle,
            &mut None,
            std::path::Path::new("/tmp"),
        );

        assert!(
            cancel_flag.load(std::sync::atomic::Ordering::Relaxed),
            "Esc while sending should set the cancel flag"
        );
        assert!(!app.sending, "sending should be reset after Esc");
        assert!(handle.is_none(), "send handle should be taken on cancel");
    }

    #[test]
    fn second_send_keystroke_while_sending_is_ignored() {
        // Guard against double-send: while a send is in flight, pressing `s` again
        // must NOT replace the live handle (which would orphan the first worker).
        use std::sync::{atomic::AtomicBool, mpsc, Arc};

        let (mut app, _dir) = make_app_with_file(J_FLAT);
        app.selected = 1; // the "ping" request row
        app.sending = true; // a send is already in flight

        // A live handle whose identity we can check stays untouched.
        let (_tx, rx) = mpsc::channel::<crate::tui::worker::SendMsg>();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let mut handle: Option<SendHandle> = Some(SendHandle {
            rx,
            cancel: cancel_flag.clone(),
            join: None,
        });
        let original_cancel = Arc::as_ptr(&handle.as_ref().unwrap().cancel);

        handle_key(
            &mut app,
            key(KeyCode::Char('s')),
            &mut handle,
            &mut None,
            std::path::Path::new("/tmp"),
        );

        assert!(app.sending, "still sending after the ignored keystroke");
        let still = handle.as_ref().expect("handle must not be dropped");
        assert!(
            std::ptr::eq(Arc::as_ptr(&still.cancel), original_cancel),
            "the original send handle must not be replaced by a second send"
        );
    }

    #[test]
    fn backspace_in_prompt_mode_removes_char() {
        use crate::tui::app::Mode;
        let (mut app, _dir) = make_app_crud(J_EMPTY);
        app.selected = 0;

        handle_key(
            &mut app,
            key(KeyCode::Char('a')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        handle_key(
            &mut app,
            key(KeyCode::Char('h')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        handle_key(
            &mut app,
            key(KeyCode::Char('i')),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.prompt.as_ref().unwrap().buffer, "hi");
        handle_key(
            &mut app,
            key(KeyCode::Backspace),
            &mut None,
            &mut None,
            std::path::Path::new("/tmp"),
        );
        assert_eq!(app.prompt.as_ref().unwrap().buffer, "h");
        assert_eq!(app.mode, Mode::Prompt);
    }
}
