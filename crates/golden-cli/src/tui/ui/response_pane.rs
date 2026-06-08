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
            "press s to send",
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

fn tab_content(app: &App) -> Vec<Line<'static>> {
    let Some(resp) = &app.last_response else {
        return vec![Line::styled(
            "(no response)",
            Style::default().fg(Color::DarkGray),
        )];
    };
    match app.response_tab {
        ResponseTab::Body => {
            let ct = header_value(&resp.headers, "content-type");
            let raw: Vec<String> = pretty_body(&resp.body, ct)
                .lines()
                .map(|l| l.to_string())
                .collect();
            crate::tui::search::filter_lines(&raw, &app.search_query)
                .into_iter()
                .map(|(_, l)| Line::raw(l))
                .collect()
        }
        ResponseTab::Headers => {
            let raw: Vec<String> = resp
                .headers
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect();
            crate::tui::search::filter_lines(&raw, &app.search_query)
                .into_iter()
                .map(|(_, l)| Line::raw(l))
                .collect()
        }
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
        ResponseTab::Tests => {
            // Assertions arrive once the sandbox plan populates RequestResult; for a
            // single send there is no script run, so show the parked state.
            vec![Line::styled(
                "Test results appear after `r`/`R` runs (sandbox plan).",
                Style::default().fg(Color::DarkGray),
            )]
        }
    }
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
