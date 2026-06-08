//! Bottom status bar: active env + keybinding hints + transient status text.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::app::App;

pub fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let hints =
        " a add  A folder  d del  r rename  c copy  s send  R run-all  / search  x env  ? help  q quit ";
    // In Confirm mode, the confirm message overrides the status line.
    let left = if app.mode == crate::tui::app::Mode::Confirm {
        if let Some(c) = &app.confirm {
            format!(" {} ", c.message)
        } else {
            " (y/n) ".to_string()
        }
    } else if app.status.is_empty() {
        hints.to_string()
    } else {
        format!(" {} ", app.status)
    };
    let line = Line::from(vec![
        Span::styled(
            format!("[env: {}] ", app.active_env),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw(left),
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
        let app = empty_app();
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
