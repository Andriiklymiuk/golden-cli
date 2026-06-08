//! The interactive `golden` TUI — a lazygit/k9s-style terminal surface for
//! browsing Postman v2.1 collections, sending requests, running collections,
//! and inspecting responses. Launched when `golden`/`gr` runs with no subcommand.
//!
//! Read-only view + send/run (editing/CRUD is Plan E). Built across Plan D
//! Tasks 2-19. Reuses `crate::discovery` + `crate::load` for collection
//! discovery so the TUI and the headless `list`/`run` see the same files.

pub mod app;
pub mod edit;
pub mod event;
pub mod format;
pub mod loader;
pub mod run_state;
pub mod search;
pub mod tree;
pub mod ui;
pub mod worker;

use std::io;
use std::time::Duration;

use ratatui::crossterm::event::{poll as ct_poll, read as ct_read, Event, KeyEventKind};

use app::App;
use worker::{RunHandle, SendHandle, WatchHandle};

/// Launch the interactive TUI. `collections` are optional root overrides
/// (same semantics as the `--collections` flag). Returns a process exit code.
pub fn launch(collections: &[String]) -> i32 {
    let workspace = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let (loaded, errors) = loader::load_collections(&workspace, collections);

    // Resolve variable scopes from the first collection's vars + .env files.
    let collection_vars = loaded
        .first()
        .map(|c| c.collection.variable.clone())
        .unwrap_or_default();
    // Use workspace as both workspace and collections_root for the default case.
    let scopes = golden_core::env::resolve(&workspace, &workspace, &collection_vars);

    let mut app = App::new(workspace.clone(), loaded, scopes);
    if !errors.is_empty() {
        app.status = format!("{} file(s) failed to parse", errors.len());
    }

    // `ratatui::init()` enters alternate screen + raw mode and installs a panic
    // hook that calls `ratatui::restore()` so the terminal is always cleaned up.
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, &mut app, &workspace);
    // Restore the terminal regardless of whether the loop succeeded.
    ratatui::restore();

    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("golden TUI error: {e}");
            1
        }
    }
}

/// Drive the draw + event loop until the user quits.
///
/// `ratatui::restore()` is called by the caller after this returns (both Ok and
/// Err paths), so there is no risk of leaving the terminal in raw mode.
fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    workspace: &std::path::Path,
) -> io::Result<()> {
    let mut send_handle: Option<SendHandle> = None;
    let mut run_handle: Option<RunHandle> = None;

    // Spawn a background watcher on the collections dir; gracefully degrade if
    // it cannot be set up (e.g. non-existent path during tests).
    let watch: Option<WatchHandle> = worker::spawn_watch(&app.collections_dir).ok();
    let mut reload_pending = false;

    while !app.should_quit {
        terminal.draw(|frame| ui::draw(frame, app))?;

        // Drain any completed send result from the background send worker.
        event::drain_send(app, &mut send_handle);

        // Drain any completed run result from the background run worker.
        event::drain_run(app, &mut run_handle);

        // Coalesce a burst of watcher events into a single reload flag.
        if let Some(w) = &watch {
            while w.rx.try_recv().is_ok() {
                reload_pending = true;
            }
        }

        // Perform the debounced reload on the next tick after the last event.
        if reload_pending {
            reload_pending = false;
            let (collections, errors) = loader::load_collections(&app.collections_dir, &[]);
            worker::apply_reload(app, collections, &errors);
        }

        if ct_poll(Duration::from_millis(100))? {
            if let Event::Key(key) = ct_read()? {
                if key.kind == KeyEventKind::Press {
                    event::handle_key(app, key, &mut send_handle, &mut run_handle, workspace);
                }
            }
        }
    }
    Ok(())
}
