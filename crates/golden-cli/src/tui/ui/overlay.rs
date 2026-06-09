//! Modal overlays: the live run panel (and future: help, env switcher, search bar).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Sparkline, Wrap,
};
use ratatui::Frame;

use crate::tui::app::App;

/// Return a centered `Rect` that is `pct_x`% wide and `pct_y`% tall within `area`.
pub fn centered(pct_x: u16, pct_y: u16, area: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(v[1])[1]
}

/// Draw the run overlay (progress gauge + per-request list + optional stats).
pub fn draw_run(frame: &mut Frame, app: &App, area: Rect) {
    let rect = centered(70, 70, area);
    frame.render_widget(Clear, rect);
    let block = Block::default().borders(Borders::ALL).title(" Run ");
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let has_stats = !app.run.stats().is_empty();
    let constraints = if has_stats {
        vec![
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(6),
        ]
    } else {
        vec![Constraint::Length(3), Constraint::Min(3)]
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    // Progress gauge.
    let label = if app.run.running {
        format!("{}/{}", app.run.done, app.run.total)
    } else {
        format!("done {}/{}", app.run.done, app.run.total)
    };
    frame.render_widget(
        Gauge::default()
            .block(Block::default().borders(Borders::ALL).title("Progress"))
            .gauge_style(Style::default().fg(Color::Green))
            .ratio(app.run.ratio())
            .label(label),
        rows[0],
    );

    // Per-request results.
    let items: Vec<ListItem> = app
        .run
        .request_lines()
        .into_iter()
        .map(|(name, ok, status)| {
            let (sym, color) = if ok {
                ("\u{2713}", Color::Green)
            } else {
                ("\u{2717}", Color::Red)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{sym} "), Style::default().fg(color)),
                Span::raw(format!("{name}  ")),
                Span::styled(status, Style::default().fg(Color::Gray)),
            ]))
        })
        .collect();
    frame.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title("Requests")),
        rows[1],
    );

    // Stats block (only shown when iterations > 1 produced stats).
    if has_stats {
        draw_stats(frame, app, rows[2]);
    }
}

fn draw_stats(frame: &mut Frame, app: &App, area: Rect) {
    let stats = app.run.stats();
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Stats (avg ms)");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let data: Vec<u64> = stats.iter().map(|s| s.avg_ms.round() as u64).collect();
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);
    let summary: Vec<Span> = stats
        .iter()
        .map(|s| {
            Span::raw(format!(
                "{}: avg {:.0} min {} max {}   ",
                s.name, s.avg_ms, s.min_ms, s.max_ms
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(Line::from(summary)), split[0]);
    frame.render_widget(
        Sparkline::default()
            .data(
                data.iter()
                    .copied()
                    .map(ratatui::widgets::SparklineBar::from)
                    .collect::<Vec<_>>(),
            )
            .style(Style::default().fg(Color::Cyan)),
        split[1],
    );
}

/// Draw the env-switcher overlay listing discovered env profiles.
pub fn draw_env_switch(frame: &mut Frame, app: &App, area: Rect) {
    let rect = centered(40, 50, area);
    frame.render_widget(Clear, rect);
    let items: Vec<ListItem> = app
        .env_profiles
        .iter()
        .map(|(name, _)| ListItem::new(Line::raw(name.clone())))
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Switch env (enter) "),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default();
    if !app.env_profiles.is_empty() {
        state.select(Some(app.env_selected));
    }
    frame.render_stateful_widget(list, rect, &mut state);
}

/// Draw the move-target collection picker overlay.
pub fn draw_move_target(frame: &mut Frame, app: &App, area: Rect) {
    let rect = centered(40, 50, area);
    frame.render_widget(Clear, rect);
    let items: Vec<ListItem> = app
        .collections
        .iter()
        .map(|lc| ListItem::new(Line::raw(lc.collection.info.name.clone())))
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Move to collection (j/k enter) "),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default();
    if !app.collections.is_empty() {
        state.select(Some(app.move_target_selected));
    }
    frame.render_stateful_widget(list, rect, &mut state);
}

/// Draw the search bar overlay.
pub fn draw_search(frame: &mut Frame, app: &App, area: Rect) {
    let rect = centered(60, 20, area);
    frame.render_widget(Clear, rect);
    let para = Paragraph::new(Line::from(vec![
        Span::styled("/", Style::default().fg(Color::Cyan)),
        Span::raw(app.search_query.clone()),
        Span::styled("\u{258f}", Style::default().fg(Color::Cyan)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Search response (enter/esc) "),
    );
    frame.render_widget(para, rect);
}

/// Draw the help overlay listing all keybindings.
pub fn draw_help(frame: &mut Frame, area: Rect) {
    let rect = centered(50, 70, area);
    frame.render_widget(Clear, rect);
    let lines = vec![
        Line::styled(
            "golden \u{2014} keybindings",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::raw("enter       send request  (or expand/collapse a folder)"),
        Line::raw("s           send selected request"),
        Line::raw("↑↓ / j k    move selection · scroll response"),
        Line::raw("g / G       jump to top / bottom"),
        Line::raw("tab         switch pane (tree / request / response)"),
        Line::raw("e           edit the focused request field"),
        Line::raw("f           cycle request field (method/url/headers/body/scripts)"),
        Line::raw("t           cycle response tab (body/headers/cookies/tests)"),
        Line::raw("r           run selected (request runs tests; folder/collection runs all)"),
        Line::raw("R           run all collections"),
        Line::raw("F2          rename selected item        (tree)"),
        Line::raw("a / A       add request / folder        (tree)"),
        Line::raw("ctrl-d      duplicate selected item"),
        Line::raw("d           delete selected item        (tree)"),
        Line::raw("m           move item to another collection"),
        Line::raw("] / [       reorder item down / up within its container"),
        Line::raw("x           switch environment"),
        Line::raw("esc         cancel in-flight send / close run"),
        Line::raw("/           search response"),
        Line::raw("?           toggle this help · esc close overlay"),
        Line::raw("ctrl-c      quit (from anywhere) · q quit"),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Help ")),
        rect,
    );
}

/// Hint shown in the run overlay footer (rendered by caller in the status bar).
pub fn run_footer() -> Line<'static> {
    Line::from(Span::styled(
        " esc close ",
        Style::default().add_modifier(Modifier::DIM),
    ))
}

/// Draw the tree-CRUD name-prompt overlay.
///
/// Same visual style as `draw_edit` (60% wide, centered, yellow border), but
/// driven by `App::prompt` (a `PromptSession`) rather than `App::edit`.
pub fn draw_prompt(frame: &mut Frame, app: &App, area: Rect) {
    use crate::tui::app::PromptSession;

    let session: &PromptSession = match app.prompt.as_ref() {
        Some(s) => s,
        None => return,
    };

    let rect = centered(60, 25, area);
    frame.render_widget(Clear, rect);

    let title = format!(" {} ", session.title);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(title.as_str());

    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    let buf_text = format!("{}█", session.buffer);
    frame.render_widget(
        Paragraph::new(buf_text)
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: false }),
        rows[0],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" confirm  "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" cancel"),
        ]))
        .style(Style::default().fg(Color::DarkGray)),
        rows[1],
    );
}

/// Draw the single-field inline editor overlay.
///
/// Renders a bordered popup (60% wide, centered vertically near the top) that shows
/// the field title, the current buffer content + a cursor character, and a hint line.
pub fn draw_edit(frame: &mut Frame, app: &App, area: Rect) {
    use crate::tui::edit::EditSession;

    let session: &EditSession = match app.edit.as_ref() {
        Some(s) => s,
        None => return,
    };

    let rect = centered(60, 25, area);
    frame.render_widget(Clear, rect);

    let title = format!(" {} ", session.field.title());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(title.as_str());

    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    // Split inner area: buffer input line + hint line.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    // Buffer with a trailing cursor block.
    let buf_text = format!("{}█", session.buffer);
    frame.render_widget(
        Paragraph::new(buf_text)
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: false }),
        rows[0],
    );

    // Hint line.
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" save  "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" cancel"),
        ]))
        .style(Style::default().fg(Color::DarkGray)),
        rows[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::{App, Mode};
    use crate::tui::loader::LoadedCollection;
    use crate::tui::run_state::RunState;
    use golden_core::env::VarScopes;
    use golden_core::model::Collection;
    use golden_core::result::{
        Assertion, CollectionResult, Iteration, RequestResult, RunResult, Totals,
    };
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn draw_help_overlay_renders_keybindings() {
        // Use a tall-enough terminal so all keybinding lines fit inside the overlay.
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.mode = Mode::Help;
        terminal
            .draw(|frame| draw_help(frame, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("Help"),
            "help overlay should have a Help title"
        );
        assert!(
            content.contains("golden"),
            "help overlay should mention golden"
        );
        assert!(content.contains("quit"), "help overlay should mention quit");
    }

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
        { "name": "ping", "request": { "method": "GET", "url": "http://x/ping" } }
      ]
    }"#;

    fn rr_ok(name: &str) -> RequestResult {
        RequestResult {
            name: name.into(),
            method: "GET".into(),
            url: "http://x".into(),
            status: Some(200),
            time_ms: 10,
            assertions: vec![Assertion {
                name: "s".into(),
                passed: true,
                error: None,
            }],
            error: None,
        }
    }

    fn run_result_with(reqs: Vec<RequestResult>) -> RunResult {
        RunResult {
            collections: vec![CollectionResult {
                name: "C".into(),
                iterations: vec![Iteration {
                    index: 1,
                    requests: reqs,
                }],
                stats: vec![],
            }],
            totals: Totals::default(),
        }
    }

    #[test]
    fn draw_run_overlay_renders_without_panic() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.mode = Mode::Run;
        app.run = RunState {
            running: false,
            total: 1,
            done: 1,
            result: Some(run_result_with(vec![rr_ok("ping")])),
        };
        terminal
            .draw(|frame| draw_run(frame, &app, frame.area()))
            .unwrap();
    }

    #[test]
    fn centered_returns_inner_rect() {
        let area = Rect::new(0, 0, 100, 100);
        let r = centered(70, 70, area);
        // Should be roughly in the middle
        assert!(r.x > 0);
        assert!(r.y > 0);
        assert!(r.width < 100);
        assert!(r.height < 100);
    }

    #[test]
    fn draw_env_switch_renders_without_panic() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.mode = Mode::EnvSwitch;
        app.env_profiles = vec![
            ("default".to_string(), "/tmp/.env".into()),
            ("staging".to_string(), "/tmp/.env.staging".into()),
        ];
        app.env_selected = 0;
        terminal
            .draw(|frame| draw_env_switch(frame, &app, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("default"),
            "env switcher should list env names"
        );
        assert!(
            content.contains("staging"),
            "env switcher should list staging"
        );
    }

    #[test]
    fn draw_search_renders_without_panic() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.mode = Mode::Search;
        app.search_query = "alice".to_string();
        terminal
            .draw(|frame| draw_search(frame, &app, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("alice"),
            "search overlay should show query"
        );
    }

    #[test]
    fn draw_edit_overlay_renders_field_title_and_buffer() {
        use crate::tui::edit::{EditField, EditSession};
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.mode = Mode::Edit;
        app.edit = Some(EditSession::new(
            EditField::Url,
            vec![0],
            "http://example.com/api".into(),
        ));
        terminal
            .draw(|frame| draw_edit(frame, &app, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("URL"),
            "edit overlay should show field title 'URL', got content with no 'URL'"
        );
        assert!(
            content.contains("http"),
            "edit overlay should show buffer content"
        );
        assert!(
            content.contains("Enter"),
            "edit overlay should show Enter hint"
        );
        assert!(content.contains("Esc"), "edit overlay should show Esc hint");
    }

    #[test]
    fn draw_edit_overlay_does_nothing_when_no_session() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = make_app(J);
        // edit is None — draw_edit should be a no-op (no panic)
        terminal
            .draw(|frame| draw_edit(frame, &app, frame.area()))
            .unwrap();
        // Just checking it doesn't panic; all cells should be empty / default.
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            !content.contains("Enter"),
            "no edit session should render no overlay"
        );
    }
}
