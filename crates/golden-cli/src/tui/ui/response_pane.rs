//! The right pane: response status/time + tabbed Body / Headers / Cookies / Tests.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Tabs, Wrap};
use ratatui::Frame;

use crate::tui::app::{App, Pane, ResponseTab};
use crate::tui::format::{cookies, header_value, pretty_body, status_color};

pub fn draw_response(frame: &mut Frame, app: &App, area: Rect) {
    let border_style = if app.focus == Pane::Response {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(" Response / Tests ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner);

    // Status line.
    let status_line = if app.sending {
        Line::from(Span::styled(
            "sending...",
            Style::default().fg(Color::Yellow),
        ))
    } else if let Some(err) = &app.last_error {
        Line::from(Span::styled(
            format!("Error: {err}"),
            Style::default().fg(Color::Red),
        ))
    } else if let Some(resp) = &app.last_response {
        Line::from(vec![
            Span::styled(
                format!("{} ", resp.status),
                Style::default()
                    .fg(status_color(resp.status))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{}ms", resp.time_ms),
                Style::default().fg(Color::Gray),
            ),
        ])
    } else {
        Line::from(Span::styled(
            "press Enter or s to send",
            Style::default().fg(Color::DarkGray),
        ))
    };
    frame.render_widget(Paragraph::new(status_line), rows[0]);

    // Tab headers.
    let titles = vec!["Body", "Headers", "Cookies", "Tests"];
    let selected = match app.response_tab {
        ResponseTab::Body => 0,
        ResponseTab::Headers => 1,
        ResponseTab::Cookies => 2,
        ResponseTab::Tests => 3,
    };
    frame.render_widget(
        Tabs::new(titles).select(selected).highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        rows[1],
    );

    // Tab body.
    let content = tab_content(app);
    frame.render_widget(
        Paragraph::new(content)
            .wrap(Wrap { trim: false })
            .scroll((app.response_scroll, 0)),
        rows[2],
    );
}

/// Concatenated plain text of a rendered line, used to match the search query
/// against styled lines (Cookies / Tests) the same way as the raw Body/Headers.
fn line_text(line: &Line<'static>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// Number of lines in the current tab that match the active search query. Used
/// by the search bar to show "/query (N matches)".
pub fn match_count(app: &App) -> usize {
    unfiltered_lines(app)
        .iter()
        .filter(|l| crate::tui::search::matches(&line_text(l), &app.search_query))
        .count()
}

/// Build the full, unfiltered set of lines for the current tab.
fn unfiltered_lines(app: &App) -> Vec<Line<'static>> {
    // The Tests tab renders independently of the response: a transport or
    // pre-request-script failure has no response yet still produces a script
    // error and/or pre-request assertions we must surface.
    if app.response_tab == ResponseTab::Tests {
        return tests_tab(app);
    }
    let Some(resp) = &app.last_response else {
        return vec![Line::styled(
            "(no response)",
            Style::default().fg(Color::DarkGray),
        )];
    };
    match app.response_tab {
        ResponseTab::Body => {
            let ct = header_value(&resp.headers, "content-type");
            pretty_body(&resp.body, ct)
                .lines()
                .map(|l| Line::raw(l.to_string()))
                .collect()
        }
        ResponseTab::Headers => resp
            .headers
            .iter()
            .map(|(k, v)| Line::raw(format!("{k}: {v}")))
            .collect(),
        ResponseTab::Cookies => {
            let cs = cookies(&resp.headers);
            if cs.is_empty() {
                vec![Line::styled(
                    "(no Set-Cookie)",
                    Style::default().fg(Color::DarkGray),
                )]
            } else {
                cs.into_iter().map(Line::raw).collect()
            }
        }
        // Tests is handled before the no-response guard above.
        ResponseTab::Tests => tests_tab(app),
    }
}

fn tab_content(app: &App) -> Vec<Line<'static>> {
    let lines = unfiltered_lines(app);
    if app.search_query.is_empty() {
        return lines;
    }
    lines
        .into_iter()
        .filter(|l| crate::tui::search::matches(&line_text(l), &app.search_query))
        .collect()
}

/// Render the Tests tab: a script error (red) above the assertion pass/fail list.
/// Rendered even when there is no response (transport / pre-script failure).
fn tests_tab(app: &App) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(err) = &app.last_script_error {
        lines.push(Line::styled(
            format!("script error: {err}"),
            Style::default().fg(Color::Red),
        ));
    }
    if app.last_assertions.is_empty() {
        lines.push(Line::styled(
            "(no tests defined for this request)",
            Style::default().fg(Color::DarkGray),
        ));
        return lines;
    }
    let passed = app.last_assertions.iter().filter(|a| a.passed).count();
    let failed = app.last_assertions.len() - passed;
    lines.push(Line::from(vec![
        Span::styled(
            format!("{passed} passed"),
            Style::default().fg(Color::Green),
        ),
        Span::raw(", "),
        Span::styled(format!("{failed} failed"), Style::default().fg(Color::Red)),
    ]));
    for a in &app.last_assertions {
        if a.passed {
            lines.push(Line::styled(
                format!("\u{2713} {}", a.name),
                Style::default().fg(Color::Green),
            ));
        } else {
            lines.push(Line::styled(
                format!("\u{2717} {}", a.name),
                Style::default().fg(Color::Red),
            ));
            if let Some(e) = &a.error {
                lines.push(Line::styled(
                    format!("    {e}"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::App;
    use crate::tui::loader::LoadedCollection;
    use golden_core::env::VarScopes;
    use golden_core::http::HttpResponse;
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

    fn make_app_with_response(json: &str, resp: HttpResponse) -> App {
        let mut app = make_app(json);
        app.last_response = Some(resp);
        app
    }

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

    const J: &str = r#"{
      "info": { "name": "MySuite" },
      "item": [
        { "name": "ping", "request": { "method": "GET", "url": "https://api/ping" } }
      ]
    }"#;

    fn json_response() -> HttpResponse {
        HttpResponse {
            status: 200,
            headers: vec![
                ("content-type".into(), "application/json".into()),
                ("x-request-id".into(), "abc123".into()),
            ],
            body: br#"{"ok":true,"value":42}"#.to_vec(),
            time_ms: 123,
        }
    }

    #[test]
    fn response_pane_shows_tab_labels() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = make_app(J);
        terminal
            .draw(|frame| draw_response(frame, &app, frame.area()))
            .unwrap();
        let content = buf_content(&terminal);
        assert!(
            content.contains("Body"),
            "should show Body tab, got: {content}"
        );
        assert!(
            content.contains("Headers"),
            "should show Headers tab, got: {content}"
        );
        assert!(
            content.contains("Cookies"),
            "should show Cookies tab, got: {content}"
        );
        assert!(
            content.contains("Tests"),
            "should show Tests tab, got: {content}"
        );
    }

    #[test]
    fn response_pane_shows_status_code() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = make_app_with_response(J, json_response());
        terminal
            .draw(|frame| draw_response(frame, &app, frame.area()))
            .unwrap();
        let content = buf_content(&terminal);
        assert!(
            content.contains("200"),
            "should show status code 200, got: {content}"
        );
    }

    #[test]
    fn response_pane_shows_time_ms() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = make_app_with_response(J, json_response());
        terminal
            .draw(|frame| draw_response(frame, &app, frame.area()))
            .unwrap();
        let content = buf_content(&terminal);
        assert!(
            content.contains("123ms"),
            "should show time in ms, got: {content}"
        );
    }

    #[test]
    fn response_pane_body_tab_shows_pretty_json() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app_with_response(J, json_response());
        app.response_tab = ResponseTab::Body;
        terminal
            .draw(|frame| draw_response(frame, &app, frame.area()))
            .unwrap();
        let content = buf_content(&terminal);
        // Pretty-printed JSON will have "ok" and "true" on separate lines
        assert!(
            content.contains("ok") || content.contains("true"),
            "body tab should show JSON content, got: {content}"
        );
    }

    #[test]
    fn response_pane_headers_tab_shows_header_names() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app_with_response(J, json_response());
        app.response_tab = ResponseTab::Headers;
        terminal
            .draw(|frame| draw_response(frame, &app, frame.area()))
            .unwrap();
        let content = buf_content(&terminal);
        assert!(
            content.contains("x-request-id") || content.contains("content-type"),
            "headers tab should show header names, got: {content}"
        );
    }

    #[test]
    fn response_pane_cookies_tab_shows_no_cookie_hint() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app_with_response(J, json_response());
        app.response_tab = ResponseTab::Cookies;
        terminal
            .draw(|frame| draw_response(frame, &app, frame.area()))
            .unwrap();
        let content = buf_content(&terminal);
        // no Set-Cookie headers in json_response, should show hint
        assert!(
            content.contains("no Set-Cookie") || content.contains("no"),
            "cookies tab should show no-cookie hint, got: {content}"
        );
    }

    #[test]
    fn response_pane_shows_sending_when_in_flight() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.sending = true;
        terminal
            .draw(|frame| draw_response(frame, &app, frame.area()))
            .unwrap();
        let content = buf_content(&terminal);
        assert!(
            content.contains("sending"),
            "should show sending state, got: {content}"
        );
    }

    #[test]
    fn response_pane_shows_error_when_last_send_failed() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.last_error = Some("Connection refused".to_string());
        terminal
            .draw(|frame| draw_response(frame, &app, frame.area()))
            .unwrap();
        let content = buf_content(&terminal);
        assert!(
            content.contains("Connection refused") || content.contains("Error"),
            "should show error message, got: {content}"
        );
    }

    #[test]
    fn response_pane_shows_no_response_hint_when_empty() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = make_app(J);
        terminal
            .draw(|frame| draw_response(frame, &app, frame.area()))
            .unwrap();
        let content = buf_content(&terminal);
        assert!(
            content.contains("press s to send") || content.contains("no response"),
            "should show hint when no response, got: {content}"
        );
    }

    #[test]
    fn response_pane_border_cyan_when_focused() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.focus = Pane::Response;
        terminal
            .draw(|frame| draw_response(frame, &app, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let has_cyan = buf
            .content()
            .iter()
            .any(|c| c.fg == ratatui::style::Color::Cyan);
        assert!(has_cyan, "focused response pane should have cyan border");
    }

    #[test]
    fn tests_tab_renders_pass_fail_and_summary() {
        use golden_core::result::Assertion;
        let mut app = make_app(J);
        app.last_assertions = vec![
            Assertion {
                name: "status 200".into(),
                passed: true,
                error: None,
            },
            Assertion {
                name: "has id".into(),
                passed: false,
                error: Some("expected id".into()),
            },
        ];
        app.response_tab = ResponseTab::Tests;
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw_response(f, &app, f.area())).unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("status 200"));
        assert!(text.contains("has id"));
        assert!(text.contains("1 passed") && text.contains("1 failed"));
    }

    #[test]
    fn tests_tab_renders_empty_state_when_no_tests_defined() {
        // No assertions and no script error → the Tests tab shows the empty-state
        // hint, not a pass/fail summary.
        let mut app = make_app(J);
        app.last_assertions = vec![];
        app.last_script_error = None;
        app.response_tab = ResponseTab::Tests;
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw_response(f, &app, f.area())).unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            text.contains("no tests defined for this request"),
            "Tests tab should show the empty-state hint, got: {text}"
        );
        assert!(
            !text.contains("passed"),
            "empty Tests tab must not render a pass/fail summary, got: {text}"
        );
    }

    #[test]
    fn tests_tab_renders_script_error_with_no_response() {
        // A transport / pre-request-script failure leaves last_response = None but
        // sets last_script_error. The Tests tab must still render the error (it must
        // not short-circuit to "(no response)").
        let mut app = make_app(J);
        app.last_response = None;
        app.last_script_error = Some("ReferenceError: foo is not defined".into());
        app.response_tab = ResponseTab::Tests;
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw_response(f, &app, f.area())).unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            text.contains("script error"),
            "Tests tab should render the script error even with no response, got: {text}"
        );
        assert!(
            !text.contains("(no response)"),
            "Tests tab must not short-circuit to (no response), got: {text}"
        );
    }

    #[test]
    fn cookies_tab_is_filtered_by_search_query() {
        // Two Set-Cookie headers; searching for one cookie name hides the other.
        let mut resp = json_response();
        resp.headers
            .push(("set-cookie".into(), "session=abc; Path=/".into()));
        resp.headers
            .push(("set-cookie".into(), "theme=dark; Path=/".into()));
        let mut app = make_app_with_response(J, resp);
        app.response_tab = ResponseTab::Cookies;
        app.search_query = "session".to_string();
        let lines = tab_content(&app);
        let text: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(
            text.contains("session"),
            "kept matching cookie, got: {text}"
        );
        assert!(
            !text.contains("theme"),
            "non-matching cookie should be filtered out, got: {text}"
        );
    }

    #[test]
    fn tests_tab_is_filtered_by_search_query() {
        use golden_core::result::Assertion;
        let mut app = make_app(J);
        app.last_assertions = vec![
            Assertion {
                name: "status 200".into(),
                passed: true,
                error: None,
            },
            Assertion {
                name: "has id".into(),
                passed: false,
                error: Some("expected id".into()),
            },
        ];
        app.response_tab = ResponseTab::Tests;
        app.search_query = "status".to_string();
        let lines = tab_content(&app);
        let text: String = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(
            text.contains("status 200"),
            "kept matching assertion, got: {text}"
        );
        assert!(
            !text.contains("has id"),
            "non-matching assertion should be filtered out, got: {text}"
        );
    }

    #[test]
    fn match_count_counts_matching_lines_for_current_tab() {
        // Headers tab: content-type + x-request-id. Searching "x-request" matches one.
        let mut app = make_app_with_response(J, json_response());
        app.response_tab = ResponseTab::Headers;
        app.search_query = "x-request".to_string();
        assert_eq!(match_count(&app), 1);
        // Empty query counts every line (here: 2 headers).
        app.search_query = String::new();
        assert_eq!(match_count(&app), 2);
    }

    #[test]
    fn search_bar_shows_match_count() {
        // The search overlay should surface the live count, e.g. "(1 match)".
        let mut app = make_app_with_response(J, json_response());
        app.response_tab = ResponseTab::Headers;
        app.search_query = "x-request".to_string();
        app.mode = crate::tui::app::Mode::Search;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| crate::tui::ui::overlay::draw_search(f, &app, f.area()))
            .unwrap();
        let content = buf_content(&terminal);
        assert!(
            content.contains("1 match"),
            "search bar should show match count, got: {content}"
        );
    }

    #[test]
    fn response_pane_cookies_tab_shows_cookie_values() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut resp = json_response();
        resp.headers
            .push(("set-cookie".into(), "session=abc; Path=/".into()));
        let mut app = make_app_with_response(J, resp);
        app.response_tab = ResponseTab::Cookies;
        terminal
            .draw(|frame| draw_response(frame, &app, frame.area()))
            .unwrap();
        let content = buf_content(&terminal);
        assert!(
            content.contains("session") || content.contains("abc"),
            "cookies tab should show cookie value, got: {content}"
        );
    }
}
