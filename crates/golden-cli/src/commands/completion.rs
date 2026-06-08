//! `golden completion <shell>` — emit a shell completion script to stdout.

use clap::CommandFactory;
use clap_complete::Shell;

use crate::cli::Cli;

/// Generate and write the completion script for `shell` to stdout.
pub fn execute(shell: Shell) -> i32 {
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "golden", &mut std::io::stdout());
    0
}
