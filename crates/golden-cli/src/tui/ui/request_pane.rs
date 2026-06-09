//! The middle pane: read-only detail of the selected request — method + resolved
//! URL, headers, body, and a tests count. Editing is Spec 3.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::app::{App, Pane, RequestTab};
use crate::tui::tree::NodeKind;
use crate::tui::ui::tree_pane::method_color;

/// Return the abbreviated label for the active request tab.
fn tab_label(tab: RequestTab) -> &'static str {
    match tab {
        RequestTab::Method => "Method",
        RequestTab::Url => "URL",
        RequestTab::Headers => "Headers",
        RequestTab::Body => "Body",
        RequestTab::GraphqlQuery => "GQL Query",
        RequestTab::GraphqlVariables => "GQL Vars",
        RequestTab::PreRequestScript => "Pre-req",
        RequestTab::TestScript => "Tests",
    }
}

pub fn draw_request(frame: &mut Frame, app: &App, area: Rect) {
    let border_style = if app.focus == Pane::Request {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let focused_tab = tab_label(app.request_tab);
    let title = format!(" Request [{focused_tab}] e=edit f=next ");
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title.as_str());

    let mut lines: Vec<Line> = Vec::new();

    match app.current_row().map(|r| r.kind.clone()) {
        Some(NodeKind::Request) => {
            if let Some(req) = app.current_request() {
                let vars = app.vars_map();
                let resolved_url = golden_core::subst::substitute(req.url.raw(), &vars);
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{} ", req.method),
                        Style::default()
                            .fg(method_color(&req.method))
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(resolved_url),
                ]));
                lines.push(Line::raw(""));

                let active: Vec<_> = req.header.iter().filter(|h| !h.disabled).collect();
                lines.push(Line::styled(
                    format!("Headers ({})", active.len()),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                for h in active {
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {}: ", h.key), Style::default().fg(Color::Gray)),
                        Span::raw(golden_core::subst::substitute(&h.value, &vars)),
                    ]));
                }
                lines.push(Line::raw(""));

                if let Some(body) = &req.body {
                    lines.push(Line::styled(
                        format!("Body: {}", body.mode),
                        Style::default().add_modifier(Modifier::BOLD),
                    ));
                    if body.mode == "graphql" {
                        // GraphQL body: show the query and its variables separately so
                        // both editable fields are visible (f cycles to them, e edits).
                        let gql = body.graphql.as_ref();
                        lines.push(Line::styled("  Query", Style::default().fg(Color::Gray)));
                        let query = gql.map(|g| g.query.as_str()).unwrap_or("");
                        for l in query.lines().take(10) {
                            lines.push(Line::raw(format!("    {l}")));
                        }
                        let vars = gql
                            .and_then(|g| g.variables.as_ref())
                            .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
                            .unwrap_or_default();
                        lines.push(Line::styled(
                            "  Variables",
                            Style::default().fg(Color::Gray),
                        ));
                        if vars.trim().is_empty() {
                            lines.push(Line::styled(
                                "    (none)",
                                Style::default().fg(Color::DarkGray),
                            ));
                        } else {
                            for l in vars.lines().take(8) {
                                lines.push(Line::raw(format!("    {l}")));
                            }
                        }
                    } else {
                        // body.raw is Option<serde_json::Value>: extract string preview
                        let preview = body
                            .raw
                            .as_ref()
                            .map(|v| match v {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            })
                            .unwrap_or_default();
                        for l in preview.lines().take(12) {
                            lines.push(Line::raw(format!("  {l}")));
                        }
                    }
                } else {
                    lines.push(Line::styled(
                        "Body: (none)",
                        Style::default().fg(Color::DarkGray),
                    ));
                }

                lines.push(Line::raw(""));
                let test_count = app
                    .item_at(&app.current_row().unwrap().path)
                    .map(|item| item.event.iter().filter(|e| e.listen == "test").count())
                    .unwrap_or(0);
                lines.push(Line::styled(
                    format!("Tests ({test_count})"),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
            }
        }
        Some(_) => {
            lines.push(Line::styled(
                "Folder — select a request to view detail",
                Style::default().fg(Color::DarkGray),
            ));
        }
        None => {
            lines.push(Line::styled(
                "No selection",
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
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
          { "name": "login", "request": {
            "method": "POST",
            "url": "{{base}}/login",
            "header": [{ "key": "Content-Type", "value": "application/json" }],
            "body": { "mode": "raw", "raw": "{\"user\":\"bob\"}" }
          }}
        ]},
        { "name": "ping", "request": { "method": "GET", "url": "{{base}}/ping" } }
      ]
    }"#;

    fn buf_content(terminal: &ratatui::Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .clone()
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect()
    }

    #[test]
    fn request_pane_shows_method_and_url() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        // row 2 is login (POST {{base}}/login)
        app.selected = 2;
        terminal
            .draw(|frame| draw_request(frame, &app, frame.area()))
            .unwrap();
        let content = buf_content(&terminal);
        assert!(
            content.contains("POST"),
            "should show method POST, got: {content}"
        );
        assert!(
            content.contains("/login"),
            "should show URL fragment, got: {content}"
        );
    }

    #[test]
    fn request_pane_shows_header_name() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.selected = 2; // login request
        terminal
            .draw(|frame| draw_request(frame, &app, frame.area()))
            .unwrap();
        let content = buf_content(&terminal);
        assert!(
            content.contains("Content-Type"),
            "should show header name, got: {content}"
        );
    }

    #[test]
    fn request_pane_shows_body_mode() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.selected = 2; // login has body
        terminal
            .draw(|frame| draw_request(frame, &app, frame.area()))
            .unwrap();
        let content = buf_content(&terminal);
        assert!(
            content.contains("Body:"),
            "should show body label, got: {content}"
        );
    }

    #[test]
    fn request_pane_shows_tests_count() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.selected = 2; // login request
        terminal
            .draw(|frame| draw_request(frame, &app, frame.area()))
            .unwrap();
        let content = buf_content(&terminal);
        assert!(
            content.contains("Tests"),
            "should show Tests count, got: {content}"
        );
    }

    #[test]
    fn request_pane_shows_folder_hint_when_folder_selected() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.selected = 1; // auth folder
        terminal
            .draw(|frame| draw_request(frame, &app, frame.area()))
            .unwrap();
        let content = buf_content(&terminal);
        assert!(
            content.contains("Folder"),
            "should show folder hint, got: {content}"
        );
    }

    #[test]
    fn request_pane_border_cyan_when_focused() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.focus = Pane::Request;
        app.selected = 2;
        terminal
            .draw(|frame| draw_request(frame, &app, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        // Check that at least one border cell has the cyan color
        let has_cyan = buf
            .content()
            .iter()
            .any(|c| c.fg == ratatui::style::Color::Cyan);
        assert!(has_cyan, "focused request pane should have cyan border");
    }

    const J_GQL: &str = r#"{
      "info": { "name": "G" },
      "item": [
        { "name": "gql", "request": {
          "method": "POST",
          "url": "https://x/graphql",
          "body": { "mode": "graphql", "graphql": {
            "query": "query Me { me { id } }",
            "variables": { "limit": 5 }
          }}
        }}
      ]
    }"#;

    #[test]
    fn request_pane_renders_graphql_query_and_variables() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J_GQL);
        app.selected = 1; // gql request
        terminal
            .draw(|frame| draw_request(frame, &app, frame.area()))
            .unwrap();
        let content = buf_content(&terminal);
        assert!(
            content.contains("graphql"),
            "should show graphql body mode, got: {content}"
        );
        assert!(
            content.contains("Query"),
            "should label the GraphQL query, got: {content}"
        );
        assert!(
            content.contains("Variables"),
            "should label the GraphQL variables, got: {content}"
        );
        assert!(
            content.contains("query Me"),
            "should render the query text, got: {content}"
        );
        assert!(
            content.contains("limit"),
            "should render the variables, got: {content}"
        );
    }

    #[test]
    fn request_pane_shows_no_body_hint_when_no_body() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        // ping is row 3: GET, no body
        app.selected = 3;
        terminal
            .draw(|frame| draw_request(frame, &app, frame.area()))
            .unwrap();
        let content = buf_content(&terminal);
        assert!(
            content.contains("none"),
            "no-body request should show (none) hint, got: {content}"
        );
    }
}
