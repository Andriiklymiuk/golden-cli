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
    // Surface the iteration count when running more than once, so a multi-iter
    // run is distinguishable from a single pass at a glance.
    let iters_suffix = if app.run.iterations > 1 {
        format!("  ×{}", app.run.iterations)
    } else {
        String::new()
    };
    let label = if app.run.running {
        format!("{}/{}{}", app.run.done, app.run.total, iters_suffix)
    } else {
        format!("done {}/{}{}", app.run.done, app.run.total, iters_suffix)
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

/// Draw the request-history overlay: a list of recent entries (newest first)
/// showing method, url, status and round-trip time. `j/k` navigate, `Enter`
/// replays the highlighted entry. Mirrors `draw_env_switch`/`draw_run`.
pub fn draw_history(frame: &mut Frame, app: &App, area: Rect) {
    let rect = centered(70, 60, area);
    frame.render_widget(Clear, rect);

    let entries = app.history_display();
    let items: Vec<ListItem> = if entries.is_empty() {
        vec![ListItem::new(Line::styled(
            "(no history)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        entries
            .iter()
            .map(|e| {
                let (status, color) = match e.status {
                    Some(s) if s < 400 => (s.to_string(), Color::Green),
                    Some(s) => (s.to_string(), Color::Red),
                    None => ("ERR".to_string(), Color::Red),
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{:<6}", e.method), Style::default().fg(Color::Cyan)),
                    Span::raw(format!("{} ", e.url)),
                    Span::styled(status, Style::default().fg(color)),
                    Span::styled(
                        format!("  {}ms  {}", e.time_ms, e.timestamp),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" History (j/k navigate · enter replay · esc close) "),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default();
    if !app.history.is_empty() {
        state.select(Some(app.history_selected));
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

/// Draw the move-to-folder picker overlay (destination folders + a synthetic
/// "(collection root)" entry). Driven by `App::move_pending`.
pub fn draw_move_folder(frame: &mut Frame, app: &App, area: Rect) {
    let rect = centered(45, 55, area);
    frame.render_widget(Clear, rect);
    let pending = match app.move_pending.as_ref() {
        Some(p) => p,
        None => return,
    };
    let items: Vec<ListItem> = pending
        .folders
        .iter()
        .map(|f| ListItem::new(Line::raw(f.clone())))
        .collect();
    let title = format!(" Move '{}' into… (j/k enter) ", pending.item_name);
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default();
    if !pending.folders.is_empty() {
        state.select(Some(pending.selected));
    }
    frame.render_stateful_widget(list, rect, &mut state);
}

/// Draw the collection variable manager overlay (a add · e edit · d delete).
pub fn draw_variables(frame: &mut Frame, app: &App, area: Rect) {
    let rect = centered(55, 60, area);
    frame.render_widget(Clear, rect);
    let coll_name = app
        .collections
        .get(app.var_ci)
        .map(|c| c.collection.info.name.clone())
        .unwrap_or_default();
    let vars = app.current_variables();
    let items: Vec<ListItem> = if vars.is_empty() {
        vec![ListItem::new(Line::styled(
            "(no variables — press a to add)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        vars.iter()
            .map(|v| {
                ListItem::new(Line::from(vec![
                    Span::styled(v.key.clone(), Style::default().fg(Color::Cyan)),
                    Span::raw(" = "),
                    Span::raw(v.value.clone()),
                ]))
            })
            .collect()
    };
    let title = format!(" Variables · {coll_name} (a add · e edit · d delete · esc) ");
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default();
    if !vars.is_empty() {
        state.select(Some(app.var_selected.min(vars.len() - 1)));
    }
    frame.render_stateful_widget(list, rect, &mut state);
}

/// Draw the search bar overlay.
pub fn draw_search(frame: &mut Frame, app: &App, area: Rect) {
    let rect = centered(60, 20, area);
    frame.render_widget(Clear, rect);
    let mut spans = vec![
        Span::styled("/", Style::default().fg(Color::Cyan)),
        Span::raw(app.search_query.clone()),
        Span::styled("\u{258f}", Style::default().fg(Color::Cyan)),
    ];
    // Live match count for the current response tab, e.g. "/query (3 matches)".
    if !app.search_query.is_empty() {
        let n = crate::tui::ui::response_pane::match_count(app);
        spans.push(Span::styled(
            format!(" ({n} match{})", if n == 1 { "" } else { "es" }),
            Style::default().fg(Color::DarkGray),
        ));
    }
    let para = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Search response (enter/esc) "),
    );
    frame.render_widget(para, rect);
}

/// Draw the help overlay listing all keybindings.
pub fn draw_help(frame: &mut Frame, area: Rect) {
    let rect = centered(60, 80, area);
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
        Line::raw("C           copy curl command for selected request (+ overlay)"),
        Line::raw("o           open last response in browser"),
        Line::raw("w           save last response to disk (prompts for path)"),
        Line::raw("i           import a source (path · optional --from) into collections/"),
        Line::raw("r           run selected (request runs tests; folder/collection runs all)"),
        Line::raw("R           run all collections"),
        Line::raw("+ / -       increase / decrease run iterations (default 1)"),
        Line::raw("F2          rename selected item        (tree)"),
        Line::raw("a / A       add request / folder        (tree)"),
        Line::raw("N           new collection"),
        Line::raw("v           manage collection variables (a add · e edit · d delete)"),
        Line::raw("ctrl-d      duplicate selected item"),
        Line::raw("d           delete selected item        (tree)"),
        Line::raw("m           move item to another collection (then pick a folder)"),
        Line::raw("] / [       reorder item down / up within its container"),
        Line::raw("x           switch environment"),
        Line::raw("H           request history (j/k navigate · enter replay)"),
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

/// Draw the curl overlay: a read-only, scroll-free popup showing the generated
/// curl command for the selected request (opened by the `C` gesture).
pub fn draw_curl(frame: &mut Frame, app: &App, area: Rect) {
    let rect = centered(70, 50, area);
    frame.render_widget(Clear, rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green))
        .title(" curl (copied to clipboard · esc close) ");

    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    frame.render_widget(
        Paragraph::new(app.curl_text.clone())
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: false }),
        inner,
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
        let backend = TestBackend::new(100, 48);
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
            iterations: 1,
            result: Some(run_result_with(vec![rr_ok("ping")])),
        };
        terminal
            .draw(|frame| draw_run(frame, &app, frame.area()))
            .unwrap();
    }

    #[test]
    fn draw_run_overlay_shows_iteration_multiplier_when_gt_1() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.mode = Mode::Run;
        app.run = RunState {
            running: true,
            total: 6,
            done: 2,
            iterations: 3,
            result: None,
        };
        terminal
            .draw(|frame| draw_run(frame, &app, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("\u{00d7}3"),
            "progress label should show the ×3 iteration multiplier, got: {content}"
        );
        assert!(
            content.contains("2/6"),
            "progress label should show live done/total, got: {content}"
        );
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
    fn draw_history_renders_entries_newest_first() {
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.mode = Mode::History;
        app.history = vec![
            golden_core::history::HistoryEntry {
                timestamp: "2026-06-09T00:00:00Z".into(),
                method: "GET".into(),
                url: "https://api.test/old".into(),
                request_headers: vec![],
                request_body: None,
                status: Some(200),
                time_ms: 5,
            },
            golden_core::history::HistoryEntry {
                timestamp: "2026-06-09T00:00:01Z".into(),
                method: "POST".into(),
                url: "https://api.test/new".into(),
                request_headers: vec![],
                request_body: None,
                status: Some(201),
                time_ms: 9,
            },
        ];
        app.history_selected = 0;
        terminal
            .draw(|frame| draw_history(frame, &app, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("History"),
            "overlay should be titled History"
        );
        assert!(
            content.contains("api.test/new"),
            "should list the newest url"
        );
        assert!(content.contains("POST"), "should show methods");
    }

    #[test]
    fn draw_history_renders_empty_without_panic() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.mode = Mode::History;
        terminal
            .draw(|frame| draw_history(frame, &app, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("no history"), "empty overlay should hint");
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
    fn draw_curl_overlay_renders_command_text() {
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = make_app(J);
        app.mode = Mode::Curl;
        app.curl_text = "curl -X GET 'http://example.com/api'".into();
        terminal
            .draw(|frame| draw_curl(frame, &app, frame.area()))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf
            .content()
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(
            content.contains("curl"),
            "curl overlay should show the command, got: {content}"
        );
        assert!(
            content.contains("clipboard"),
            "curl overlay title should mention the clipboard, got: {content}"
        );
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
