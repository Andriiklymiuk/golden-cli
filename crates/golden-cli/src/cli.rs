//! Command-line interface definition via clap derive.

use clap::{Args, Parser, Subcommand, ValueEnum};
pub use clap_complete;
use clap_complete::ArgValueCompleter;

use crate::commands::completion::{
    complete_collections, complete_envs, complete_filter, complete_paths, complete_requests,
};

/// golden — run Postman v2.1 collections from the terminal and CI.
#[derive(Debug, Parser)]
#[command(
    name = "golden",
    version,
    disable_version_flag = true,
    about = "Run Postman v2.1 collections headlessly"
)]
pub struct Cli {
    /// Subcommand. When omitted, `golden` launches the interactive TUI.
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Print version.
    #[arg(short = 'v', short_alias = 'V', long = "version", action = clap::ArgAction::Version)]
    version: Option<bool>,

    /// Override collection roots to scan (repeatable). Also reads GOLDEN_COLLECTIONS_PATHS.
    #[arg(short = 'c', long, global = true, value_name = "PATH")]
    pub collections: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run one or many collections (or a directory of them).
    Run(RunArgs),
    /// List discovered collections and their requests.
    List(ListArgs),
    /// Fire a single request and print the response.
    Send(SendArgs),
    /// Generate a curl command for a single request.
    Curl(CurlArgs),
    /// View/manage persisted request history (.golden/history.jsonl).
    History(HistoryArgs),
    /// Create collections/ and seed the sample collection.
    Init,
    /// Import a Postman collection / raw request / folder / OpenAPI / curl.
    Import {
        /// Source: a .json file, a directory, or a curl command string.
        source: String,
        /// Name for the imported collection (defaults to file/info name).
        #[arg(short = 'n', long)]
        name: Option<String>,
        /// Merge strategy when the destination exists: add | replace | skip.
        #[arg(short = 's', long, default_value = "skip")]
        strategy: String,
        /// Input kind: auto | postman | raw | folder | openapi | curl.
        #[arg(short = 'f', long = "from", default_value = "auto")]
        from: String,
    },
    /// Generate an OpenAPI 3.0 spec from the discovered collections.
    Openapi(OpenapiArgs),
    /// Print how to enable dynamic shell completion (bash, zsh, fish, powershell, elvish).
    Completion {
        /// Shell to enable completion for.
        shell: clap_complete::Shell,
    },
    /// Upgrade golden to the latest release (Homebrew / installer-aware).
    #[command(alias = "update")]
    Upgrade,
    /// Check the workspace + golden setup; report problems and how to fix them.
    Doctor {
        /// Apply safe fixes (e.g. seed `collections/` when none are found).
        #[arg(short = 'f', long)]
        fix: bool,
    },
}

#[derive(Debug, Args)]
pub struct HistoryArgs {
    #[command(subcommand)]
    pub action: HistoryAction,
}

#[derive(Debug, Subcommand)]
pub enum HistoryAction {
    /// List recorded entries (newest last).
    List,
    /// Delete all recorded entries.
    Clear,
    /// Disable recording.
    Off,
    /// Enable recording.
    On,
    /// Re-run a recorded entry by 1-based index.
    Replay {
        /// 1-based index of the entry to replay.
        index: usize,
    },
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// Collection files or directories to run. Empty = all discovered.
    #[arg(value_name = "PATHS", add = ArgValueCompleter::new(complete_paths))]
    pub paths: Vec<String>,

    /// Select/override the .env (name or path).
    #[arg(short = 'e', long, value_name = "NAME|PATH", add = ArgValueCompleter::new(complete_envs))]
    pub env: Option<String>,

    /// Number of iterations.
    #[arg(short = 'n', long, default_value_t = 1, value_name = "N")]
    pub iterations: u32,

    /// Stop on the first assertion failure.
    #[arg(short = 'b', long)]
    pub bail: bool,

    /// Output format (repeatable). Default: pretty.
    #[arg(short = 'r', long, value_enum, value_name = "FORMAT")]
    pub reporter: Vec<ReporterKind>,

    /// Write reporter output to a file (else stdout).
    #[arg(short = 'o', long, value_name = "FILE")]
    pub output: Option<String>,

    /// Disable TLS verification for all hosts.
    #[arg(short = 'k', long)]
    pub insecure: bool,

    /// Per-request timeout in milliseconds.
    #[arg(short = 't', long, value_name = "MS")]
    pub timeout: Option<u64>,

    /// Include only requests/folders whose name matches this glob.
    #[arg(short = 'f', long, value_name = "GLOB", add = ArgValueCompleter::new(complete_filter))]
    pub filter: Option<String>,

    /// Include only requests with these HTTP methods (repeatable, case-insensitive).
    /// e.g. `-X GET` for a safe read-only sweep against staging/prod.
    #[arg(short = 'X', long = "method", value_name = "METHOD")]
    pub method: Vec<String>,

    /// Data file (JSON array of objects, or CSV) for a data-driven run: one row
    /// per iteration, overlaying the variables and feeding pm.iterationData.
    #[arg(short = 'd', long, value_name = "FILE")]
    pub data: Option<String>,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Restrict listing to these files/directories.
    #[arg(value_name = "PATHS", add = ArgValueCompleter::new(complete_paths))]
    pub paths: Vec<String>,

    /// Include only requests/folders whose name matches this glob.
    #[arg(short = 'f', long, value_name = "GLOB", add = ArgValueCompleter::new(complete_filter))]
    pub filter: Option<String>,

    /// Include only requests with these HTTP methods (repeatable, case-insensitive).
    #[arg(short = 'X', long = "method", value_name = "METHOD")]
    pub method: Vec<String>,
}

#[derive(Debug, Args)]
pub struct SendArgs {
    /// Collection name (info.name) or file path.
    #[arg(value_name = "COLLECTION", add = ArgValueCompleter::new(complete_collections))]
    pub collection: String,

    /// Request name to fire.
    #[arg(value_name = "REQUEST", add = ArgValueCompleter::new(complete_requests))]
    pub request: String,

    /// Select/override the .env (name or path).
    #[arg(short = 'e', long, value_name = "NAME|PATH", add = ArgValueCompleter::new(complete_envs))]
    pub env: Option<String>,

    /// Disable TLS verification for all hosts.
    #[arg(short = 'k', long)]
    pub insecure: bool,

    /// Per-request timeout in milliseconds.
    #[arg(short = 't', long, value_name = "MS")]
    pub timeout: Option<u64>,

    /// Write the response body to a file instead of stdout.
    #[arg(short = 'o', long, value_name = "FILE")]
    pub output: Option<std::path::PathBuf>,

    /// Max download size in bytes (aborts and removes partial file if exceeded).
    #[arg(short = 'm', long = "max-size", value_name = "BYTES")]
    pub max_size: Option<u64>,

    /// Overwrite the output file without confirmation.
    #[arg(short = 'f', long)]
    pub force: bool,

    /// After the response, print Set-Cookie headers.
    #[arg(long)]
    pub cookies: bool,

    /// If the response is HTML, write it to a temp file and open it in the browser.
    #[arg(long)]
    pub open: bool,
}

#[derive(Debug, Args)]
pub struct CurlArgs {
    /// Collection file (path or discovered name).
    #[arg(value_name = "COLLECTION", add = ArgValueCompleter::new(complete_collections))]
    pub collection: String,
    /// Request name within the collection.
    #[arg(value_name = "REQUEST", add = ArgValueCompleter::new(complete_requests))]
    pub request: String,
    /// Mask sensitive header values (Authorization/Cookie/X-API-Key/Bearer/Basic).
    #[arg(short = 'm', long)]
    pub mask: bool,
    /// Follow redirects (-L).
    #[arg(short = 'L', long = "follow")]
    pub follow: bool,
    /// Include response headers (-i).
    #[arg(short = 'i', long = "include")]
    pub include: bool,
    /// Silent (-s).
    #[arg(short = 's', long)]
    pub silent: bool,
    /// Insecure (-k).
    #[arg(short = 'k', long)]
    pub insecure: bool,
    /// Fail on HTTP errors (-f).
    #[arg(short = 'f', long)]
    pub fail: bool,
    /// Request compressed response (--compressed).
    #[arg(long)]
    pub compressed: bool,
    /// Print timing (-w).
    #[arg(short = 'w', long = "timing")]
    pub timing: bool,
    /// Download to file (-O -J).
    #[arg(short = 'O', long = "download")]
    pub download: bool,
}

#[derive(Debug, Args)]
pub struct OpenapiArgs {
    /// Collections to convert. Empty = all discovered.
    #[arg(value_name = "PATHS", add = ArgValueCompleter::new(complete_paths))]
    pub paths: Vec<String>,

    /// Write the spec to a file (else stdout).
    #[arg(short = 'o', long, value_name = "FILE")]
    pub output: Option<String>,

    /// API title (defaults to the first collection's name).
    #[arg(long, value_name = "TITLE")]
    pub title: Option<String>,

    /// Server URL recorded in the spec (defaults to `{{baseUrl}}`).
    #[arg(long, value_name = "URL")]
    pub server: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ReporterKind {
    Pretty,
    Junit,
    Json,
    Tap,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_run_with_all_flags() {
        let cli = Cli::try_parse_from([
            "golden",
            "run",
            "collections/",
            "api/",
            "--env",
            ".env.staging",
            "--iterations",
            "3",
            "--bail",
            "--reporter",
            "pretty",
            "--reporter",
            "junit",
            "--output",
            "out.xml",
            "--insecure",
            "--timeout",
            "5000",
            "--filter",
            "auth/*",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Run(args)) => {
                assert_eq!(
                    args.paths,
                    vec!["collections/".to_string(), "api/".to_string()]
                );
                assert_eq!(args.env.as_deref(), Some(".env.staging"));
                assert_eq!(args.iterations, 3);
                assert!(args.bail);
                assert_eq!(
                    args.reporter,
                    vec![ReporterKind::Pretty, ReporterKind::Junit]
                );
                assert_eq!(args.output.as_deref(), Some("out.xml"));
                assert!(args.insecure);
                assert_eq!(args.timeout, Some(5000));
                assert_eq!(args.filter.as_deref(), Some("auth/*"));
            }
            _ => panic!("expected run"),
        }
    }

    #[test]
    fn run_defaults_iterations_to_1_and_reporter_to_pretty() {
        let cli = Cli::try_parse_from(["golden", "run"]).unwrap();
        match cli.command {
            Some(Command::Run(args)) => {
                assert_eq!(args.iterations, 1);
                assert!(args.paths.is_empty());
                // default reporter applied at dispatch time, not in clap; vec is empty here
                assert!(args.reporter.is_empty());
            }
            _ => panic!("expected run"),
        }
    }

    #[test]
    fn parses_list_and_send() {
        let list = Cli::try_parse_from(["golden", "list"]).unwrap();
        assert!(matches!(list.command, Some(Command::List(_))));

        let send = Cli::try_parse_from(["golden", "send", "Sample", "login"]).unwrap();
        match send.command {
            Some(Command::Send(args)) => {
                assert_eq!(args.collection, "Sample");
                assert_eq!(args.request, "login");
            }
            _ => panic!("expected send"),
        }
    }

    #[test]
    fn bare_invocation_has_no_subcommand_for_tui() {
        // `golden` with no subcommand -> command is None -> launches the TUI.
        let cli = Cli::try_parse_from(["golden"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn parses_init_subcommand() {
        let cli = Cli::try_parse_from(["golden", "init"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Init)));
    }

    #[test]
    fn parses_import_subcommand_with_all_flags() {
        let cli = Cli::try_parse_from([
            "golden",
            "import",
            "spec.json",
            "--name",
            "MyAPI",
            "--strategy",
            "add",
            "--from",
            "openapi",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Import {
                source,
                name,
                strategy,
                from,
            }) => {
                assert_eq!(source, "spec.json");
                assert_eq!(name.as_deref(), Some("MyAPI"));
                assert_eq!(strategy, "add");
                assert_eq!(from, "openapi");
            }
            _ => panic!("expected import"),
        }
    }

    #[test]
    fn version_flag_works_with_short_v_and_capital_v() {
        for flag in ["-v", "-V", "--version"] {
            let err = Cli::try_parse_from(["golden", flag]).unwrap_err();
            assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
        }
    }

    #[test]
    fn parses_run_with_short_flags() {
        let cli = Cli::try_parse_from([
            "golden",
            "run",
            "-e",
            ".env.staging",
            "-n",
            "3",
            "-b",
            "-r",
            "junit",
            "-o",
            "out.xml",
            "-k",
            "-t",
            "5000",
            "-f",
            "auth/*",
            "-d",
            "rows.csv",
            "-c",
            "collections/",
        ])
        .unwrap();
        assert_eq!(cli.collections, vec!["collections/".to_string()]);
        match cli.command {
            Some(Command::Run(args)) => {
                assert_eq!(args.env.as_deref(), Some(".env.staging"));
                assert_eq!(args.iterations, 3);
                assert!(args.bail);
                assert_eq!(args.reporter, vec![ReporterKind::Junit]);
                assert_eq!(args.output.as_deref(), Some("out.xml"));
                assert!(args.insecure);
                assert_eq!(args.timeout, Some(5000));
                assert_eq!(args.filter.as_deref(), Some("auth/*"));
                assert_eq!(args.data.as_deref(), Some("rows.csv"));
            }
            _ => panic!("expected run"),
        }
    }

    #[test]
    fn parses_send_with_short_flags() {
        let cli = Cli::try_parse_from([
            "golden",
            "send",
            "Sample",
            "login",
            "-e",
            ".env",
            "-k",
            "-t",
            "1000",
            "-o",
            "/tmp/out.bin",
            "-m",
            "1048576",
            "-f",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Send(args)) => {
                assert_eq!(args.env.as_deref(), Some(".env"));
                assert!(args.insecure);
                assert_eq!(args.timeout, Some(1000));
                assert_eq!(args.max_size, Some(1048576));
                assert!(args.force);
            }
            _ => panic!("expected send"),
        }
    }

    #[test]
    fn parses_import_with_short_flags() {
        let cli = Cli::try_parse_from([
            "golden",
            "import",
            "spec.json",
            "-n",
            "MyAPI",
            "-s",
            "add",
            "-f",
            "openapi",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Import {
                name,
                strategy,
                from,
                ..
            }) => {
                assert_eq!(name.as_deref(), Some("MyAPI"));
                assert_eq!(strategy, "add");
                assert_eq!(from, "openapi");
            }
            _ => panic!("expected import"),
        }
    }

    #[test]
    fn rejects_unknown_reporter() {
        let err = Cli::try_parse_from(["golden", "run", "--reporter", "yaml"]);
        assert!(err.is_err());
    }

    #[test]
    fn parses_history_list() {
        let cli = Cli::try_parse_from(["golden", "history", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::History(HistoryArgs {
                action: HistoryAction::List
            }))
        ));
    }

    #[test]
    fn parses_history_clear() {
        let cli = Cli::try_parse_from(["golden", "history", "clear"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::History(HistoryArgs {
                action: HistoryAction::Clear
            }))
        ));
    }

    #[test]
    fn parses_history_on_off() {
        let on = Cli::try_parse_from(["golden", "history", "on"]).unwrap();
        assert!(matches!(
            on.command,
            Some(Command::History(HistoryArgs {
                action: HistoryAction::On
            }))
        ));
        let off = Cli::try_parse_from(["golden", "history", "off"]).unwrap();
        assert!(matches!(
            off.command,
            Some(Command::History(HistoryArgs {
                action: HistoryAction::Off
            }))
        ));
    }

    #[test]
    fn parses_history_replay_with_index() {
        let cli = Cli::try_parse_from(["golden", "history", "replay", "3"]).unwrap();
        match cli.command {
            Some(Command::History(HistoryArgs {
                action: HistoryAction::Replay { index },
            })) => {
                assert_eq!(index, 3);
            }
            _ => panic!("expected history replay"),
        }
    }

    #[test]
    fn parses_send_output_flags() {
        let cli = Cli::try_parse_from([
            "golden",
            "send",
            "Sample",
            "login",
            "--output",
            "/tmp/out.bin",
            "--max-size",
            "1048576",
            "--force",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Send(args)) => {
                assert_eq!(
                    args.output.as_ref().unwrap().to_str().unwrap(),
                    "/tmp/out.bin"
                );
                assert_eq!(args.max_size, Some(1048576));
                assert!(args.force);
            }
            _ => panic!("expected send"),
        }
    }

    #[test]
    fn parses_send_cookies_and_open_flags() {
        let cli = Cli::try_parse_from(["golden", "send", "Sample", "login", "--cookies", "--open"])
            .unwrap();
        match cli.command {
            Some(Command::Send(args)) => {
                assert!(args.cookies);
                assert!(args.open);
            }
            _ => panic!("expected send"),
        }
    }

    #[test]
    fn parses_completion_bash() {
        let cli = Cli::try_parse_from(["golden", "completion", "bash"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Completion {
                shell: clap_complete::Shell::Bash
            })
        ));
    }

    #[test]
    fn parses_completion_zsh() {
        let cli = Cli::try_parse_from(["golden", "completion", "zsh"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Completion {
                shell: clap_complete::Shell::Zsh
            })
        ));
    }

    #[test]
    fn parses_completion_fish() {
        let cli = Cli::try_parse_from(["golden", "completion", "fish"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Completion {
                shell: clap_complete::Shell::Fish
            })
        ));
    }
}
