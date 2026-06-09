//! Bottom status bar: active env + keybinding hints + transient status text.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::app::App;

pub fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    use crate::tui::app::Pane;
    // Short, context-aware hints — the essentials for the focused pane. The full
    // keymap lives in `?` help so the bar stays readable on narrow terminals.
    let hints = match app.focus {
        Pane::Tree => {
            " ↑↓ move · Enter send · e edit · a add · r run · F2 rename · R run-all · Tab panes · ? help · ^C quit "
        }
        Pane::Request => " f field · e edit · Enter send · Tab panes · ? help · ^C quit ",
        Pane::Response => " ↑↓ scroll · t tab · Tab panes · ? help · ^C quit ",
    };
    // First-open welcome banner (highlighted) takes the line until the first key.
    // Then: Confirm message > transient status > context hints.
    let (left, left_style) = if app.sending {
        // A send is in flight: advertise the abort key so the UI is never wedged.
        let msg = if app.status.is_empty() {
            " sending… · Esc cancel ".to_string()
        } else {
            format!(" {} · Esc cancel ", app.status)
        };
        (msg, Style::default())
    } else if app.show_welcome && app.mode == crate::tui::app::Mode::Normal && app.status.is_empty()
    {
        (
            " 👋 Welcome — ↑↓ pick a request · Enter to send · ? for help · ^C to quit "
                .to_string(),
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
    } else if app.mode == crate::tui::app::Mode::Confirm {
        let msg = match &app.confirm {
            Some(c) => format!(" {} ", c.message),
            None => " (y/n) ".to_string(),
        };
        (msg, Style::default())
    } else if app.status.is_empty() {
        (hints.to_string(), Style::default())
    } else {
        (format!(" {} ", app.status), Style::default())
    };
    let line = Line::from(vec![
        Span::styled(
            format!("[env: {}] ", app.active_env),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(left, left_style),
    ]);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(Color::Black).fg(Color::White)),
        area,
    );
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

    fn empty_app() -> App {
        let lc = LoadedCollection {
            path: "/tmp/x.json".into(),
            collection: serde_json::from_str::<Collection>(r#"{"info":{"name":"X"},"item":[]}"#)
                .unwrap(),
        };
        App::new("/tmp".into(), vec![lc], VarScopes::default())
    }

    #[test]
    fn status_bar_shows_key_hints_when_status_empty() {
        let backend = TestBackend::new(120, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = empty_app();
        app.show_welcome = false; // dismiss banner so the tree hint shows
        terminal
            .draw(|frame| draw_status_bar(frame, &app, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("quit"), "should show key hints");
        assert!(
            content.contains("F2 rename"),
            "tree hint should advertise F2 rename, got: {content}"
        );
    }

    #[test]
    fn status_bar_shows_cancel_hint_while_sending() {
        let backend = TestBackend::new(120, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = empty_app();
        app.sending = true;
        terminal
            .draw(|frame| draw_status_bar(frame, &app, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("Esc"),
            "while sending the status bar should advertise Esc, got: {content}"
        );
    }

    #[test]
    fn status_bar_shows_welcome_on_first_open() {
        let backend = TestBackend::new(120, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = empty_app(); // show_welcome = true from App::new
        assert!(app.show_welcome);
        terminal
            .draw(|frame| draw_status_bar(frame, &app, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("Welcome"),
            "first open should show welcome"
        );
    }

    #[test]
    fn status_bar_shows_status_message_when_set() {
        let backend = TestBackend::new(120, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = empty_app();
        app.status = "2 file(s) failed to parse".to_string();
        terminal
            .draw(|frame| draw_status_bar(frame, &app, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("failed"),
            "should show the status message, got: {}",
            content
        );
    }
}
