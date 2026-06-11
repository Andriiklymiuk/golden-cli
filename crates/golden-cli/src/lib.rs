//! golden-cli: headless CLI over golden-core. Discover, run, report, set exit code.
//!
//! Two binaries (`golden` and `gr`) share this one entrypoint via `src/bin/`.

pub mod cli;
pub mod commands;
pub mod discovery;
pub mod exit;
pub mod filter;
pub mod load;
pub mod reporter;
pub mod tui;

use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use cli::{Cli, Command};

/// Parse args, dispatch the subcommand (or launch the TUI when none is given),
/// and return the process exit code.
pub fn run() -> ExitCode {
    // Dynamic shell completion: when `COMPLETE=<shell>` is set, emit candidates and exit.
    // No-op for a normal invocation; enabled per shell by `golden completion <shell>`.
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

    let cli = Cli::parse();
    let code = match &cli.command {
        // Bare `golden` (or `gr`) with no subcommand -> interactive TUI.
        None => tui::launch(&cli.collections),
        Some(Command::Run(args)) => commands::run::execute(args, &cli.collections),
        Some(Command::List(args)) => commands::list::execute(args, &cli.collections),
        Some(Command::Send(args)) => commands::send::execute(args, &cli.collections),
        Some(Command::Curl(args)) => commands::curl::execute(args),
        Some(Command::History(args)) => commands::history::execute(&args.action),
        Some(Command::Init) => commands::init::execute(),
        Some(Command::Import {
            source,
            name,
            strategy,
            from,
        }) => commands::import::execute(source, name.as_deref(), strategy, from),
        Some(Command::Openapi(args)) => commands::openapi::execute(args, &cli.collections),
        Some(Command::Completion { shell }) => commands::completion::execute(*shell),
        Some(Command::Upgrade) => commands::upgrade::execute(),
        Some(Command::Doctor { fix }) => commands::doctor::execute(*fix, &cli.collections),
    };
    ExitCode::from(code as u8)
}
