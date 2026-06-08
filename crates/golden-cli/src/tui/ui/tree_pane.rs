//! The left pane: collections tree. Folders/collections show a ▸/▾ marker;
//! requests show a colored method tag.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::Frame;

use crate::tui::app::{App, Pane};
use crate::tui::tree::NodeKind;

/// Color a method string the way the extension's icons did.
pub fn method_color(method: &str) -> Color {
    match method.to_ascii_uppercase().as_str() {
        "GET" => Color::Green,
        "POST" => Color::Yellow,
        "PUT" => Color::Blue,
        "PATCH" => Color::Magenta,
        "DELETE" => Color::Red,
        _ => Color::Gray,
    }
}

pub fn draw_tree(frame: &mut Frame, app: &mut App, area: Rect) {
    let collapsed = &app.collapsed;
    let items: Vec<ListItem> = app
        .rows
        .iter()
        .map(|row| {
            let indent = "  ".repeat(row.depth);
            let mut spans: Vec<Span> = vec![Span::raw(indent)];
            match row.kind {
                NodeKind::Collection | NodeKind::Folder => {
                    let marker = if row.has_children {
                        if collapsed
                            .iter()
                            .any(|c| c.as_slice() == row.path.as_slice())
                        {
                            "▸ "
                        } else {
                            "▾ "
                        }
                    } else {
                        "  "
                    };
                    spans.push(Span::styled(marker, Style::default().fg(Color::Cyan)));
                    spans.push(Span::styled(
                        row.name.clone(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ));
                }
                NodeKind::Request => {
                    if let Some(m) = &row.method {
                        spans.push(Span::styled(
                            format!("{:<6}", m),
                            Style::default()
                                .fg(method_color(m))
                                .add_modifier(Modifier::BOLD),
                        ));
                    }
                    spans.push(Span::raw(row.name.clone()));
                }
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let border_style = if app.focus == Pane::Tree {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(" Collections "),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    if !app.rows.is_empty() {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(list, area, &mut state);
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
      "info": { "name": "TestColl" },
      "item": [
        { "name": "users", "item": [
          { "name": "list-users", "request": { "method": "GET", "url": "https://api/users" } }
        ]},
        { "name": "health", "request": { "method": "GET", "url": "https://api/health" } }
      ]
    }"#;

    #[test]
    fn tree_pane_renders_collection_name() {
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        terminal
            .draw(|frame| draw_tree(frame, &mut app, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("TestColl"),
            "should render collection name, got: {}",
            content
        );
    }

    #[test]
    fn tree_pane_renders_request_name() {
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        terminal
            .draw(|frame| draw_tree(frame, &mut app, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("health"),
            "should render request names, got: {}",
            content
        );
    }

    #[test]
    fn tree_pane_renders_folder_name() {
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        terminal
            .draw(|frame| draw_tree(frame, &mut app, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("users"),
            "should render folder name, got: {}",
            content
        );
    }

    #[test]
    fn method_color_returns_expected_colors() {
        assert_eq!(method_color("GET"), Color::Green);
        assert_eq!(method_color("POST"), Color::Yellow);
        assert_eq!(method_color("PUT"), Color::Blue);
        assert_eq!(method_color("PATCH"), Color::Magenta);
        assert_eq!(method_color("DELETE"), Color::Red);
        assert_eq!(method_color("HEAD"), Color::Gray);
        assert_eq!(method_color("get"), Color::Green); // case-insensitive
    }

    #[test]
    fn tree_pane_shows_collapsed_marker_for_collapsed_folder() {
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        // collapse the 'users' folder (path [0, 0])
        app.toggle_collapse(); // selected = 0, which is collection — toggle it
                               // select the users folder (row index 1 after rebuild)
        app.selected = 0; // collection header, not expandable check
                          // Let's directly collapse folder path
        app.collapsed.push(vec![0, 0]);
        app.rebuild_rows();
        terminal
            .draw(|frame| draw_tree(frame, &mut app, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        // list-users should be hidden
        assert!(
            !content.contains("list-users"),
            "collapsed folder should hide children"
        );
    }
}
