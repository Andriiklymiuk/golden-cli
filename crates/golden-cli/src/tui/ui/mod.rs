//! Top-level draw: split the frame into three panes + a status bar, dispatch to
//! per-pane draw functions and overlays.

pub mod overlay;
pub mod request_pane;
pub mod response_pane;
pub mod statusbar;
pub mod tree_pane;

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::Frame;

use crate::tui::app::App;

/// Render the whole UI for one frame.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(28),
            Constraint::Percentage(36),
            Constraint::Percentage(36),
        ])
        .split(outer[0]);

    tree_pane::draw_tree(frame, app, panes[0]);
    request_pane::draw_request(frame, app, panes[1]);
    response_pane::draw_response(frame, app, panes[2]);

    statusbar::draw_status_bar(frame, app, outer[1]);

    // Run overlay (drawn on top of everything else when active).
    if app.mode == crate::tui::app::Mode::Run {
        overlay::draw_run(frame, app, frame.area());
    }

    // Env switcher overlay.
    if app.mode == crate::tui::app::Mode::EnvSwitch {
        overlay::draw_env_switch(frame, app, frame.area());
    }

    // Search bar overlay.
    if app.mode == crate::tui::app::Mode::Search {
        overlay::draw_search(frame, app, frame.area());
    }

    // Help overlay.
    if app.mode == crate::tui::app::Mode::Help {
        overlay::draw_help(frame, frame.area());
    }

    // Edit overlay (single-field inline editor).
    if app.mode == crate::tui::app::Mode::Edit {
        overlay::draw_edit(frame, app, frame.area());
    }

    // Prompt overlay (tree-CRUD name input).
    if app.mode == crate::tui::app::Mode::Prompt {
        overlay::draw_prompt(frame, app, frame.area());
    }

    // Move-to-collection picker overlay.
    if app.mode == crate::tui::app::Mode::MoveTarget {
        overlay::draw_move_target(frame, app, frame.area());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::App;
    use crate::tui::loader::LoadedCollection;
    use golden_core::env::VarScopes;
    use golden_core::model::Collection;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_app(json: &str) -> App {
        let lc = LoadedCollection {
            path: "/tmp/x.json".into(),
            collection: serde_json::from_str::<Collection>(json).unwrap(),
        };
        App::new("/tmp".into(), vec![lc], VarScopes::default())
    }

    const J: &str = r#"{
      "info": { "name": "MySuite" },
      "item": [
        { "name": "auth", "item": [
          { "name": "login", "request": { "method": "POST", "url": "{{base}}/login" } }
        ]},
        { "name": "ping", "request": { "method": "GET", "url": "{{base}}/ping" } }
      ]
    }"#;

    #[test]
    fn draw_renders_without_panic() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }

    #[test]
    fn draw_shows_collection_name_in_tree() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("MySuite"),
            "tree pane should show the collection name"
        );
    }

    #[test]
    fn draw_shows_request_names() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("ping"),
            "tree pane should show request names"
        );
    }

    #[test]
    fn draw_shows_help_overlay_in_help_mode() {
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.mode = crate::tui::app::Mode::Help;
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("Help"),
            "draw should show help overlay in Help mode"
        );
    }

    #[test]
    fn draw_shows_placeholder_panes() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("Request"), "Request placeholder pane");
        assert!(content.contains("Response"), "Response placeholder pane");
    }
}
