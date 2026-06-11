---
name: golden
description: Author, explain, and drive the golden CLI — the terminal/CI runner for Postman v2.1 collections (the command-line half of the Golden Retriever VS Code extension). Use when working with collections/*.json, .env environments, pm.*/chai test scripts, or the golden/gr binaries; or when explaining golden's commands, flags, exit codes, and the collection format. golden is a Rust CLI (`brew install andriiklymiuk/homebrew-tools/golden`) that runs collections headlessly and ships an interactive TUI. NOT for the VS Code extension UI itself.
---

# golden

golden (https://github.com/Andriiklymiuk/golden-cli) run Postman v2.1 collections from terminal + CI, ship full-screen TUI. Read/write same git-synced `collections/` JSON as Golden Retriever VS Code extension — one file, edit in editor, run in CI.

Skill active = you expert on golden. Collections present → golden authoritative way to run them. No fallback to generic newman/curl.

Two binaries, identical: **`golden`** + short alias **`gr`**. Bare `golden` (no subcommand) → **TUI**.

## Commands

| Command | Purpose |
|---------|---------|
| `golden run [PATHS…]` | Run one/many collections (or a directory). Test scripts, iterations, reporters, `--filter` (name glob) + `-X/--method` (HTTP verb). → `run` skill |
| `golden list [PATHS…]` | List discovered collections + their requests. Supports `--filter` + `-X/--method`. |
| `golden send <COLLECTION> <REQUEST>` | Fire one request, print the response. → `send` skill |
| `golden curl <COLLECTION> <REQUEST>` | Print the equivalent curl. → `send` skill |
| `golden import <SOURCE>` | Import Postman/raw/folder/OpenAPI/curl. → `import` skill |
| `golden openapi [PATHS…]` | Convert collections → an OpenAPI 3.0 spec (`-o file`, `--title`, `--server`). → `openapi` skill |
| `golden init` | Create `collections/` and seed the sample. |
| `golden history <list\|clear\|on\|off\|replay N>` | Persisted request history (`.golden/history.jsonl`). |
| `golden completion <bash\|zsh\|fish\|powershell\|elvish>` | Enable **dynamic** shell completion — tab-completes real collection / request / env names. |

## Selecting what to run

- One request: `golden send "<collection name>" "<request name>"`.
- A group: `golden run collections/ --filter "<name glob>"` (e.g. `"*campaign*"`).
- By HTTP verb: `golden run collections/ -X GET` (repeatable) — a **read-only sweep** safe against staging/prod; `--filter` + `--method` compose.
- A whole collection / everything: pass a file path / the dir. `golden list` first to see names.

## Shell completion (dynamic)

`golden completion <shell>` prints the one line to add to the shell rc — it wires
clap_complete's dynamic engine (`source <(COMPLETE=zsh golden)`, etc.). After that,
`golden send <TAB>` lists collections, `golden send "<collection>" <TAB>` lists requests,
`golden run --filter <TAB>` lists request + folder names (pick one to run it),
`--env <TAB>` lists envs, `golden run <TAB>` lists collection paths. Run from a dir with
`collections/`.

## Discovery

Default scan `collections/` (also `.golden/`, `.retriever/`) in cwd. Override: `--collections <path>` (repeatable, global) or `GOLDEN_COLLECTIONS_PATHS`. `<COLLECTION>` arg = a collection's `info.name` or a file path.

## Environments

`.env` files. Select per run/send: `--env <name|path>`. Values substitute into URLs/headers/bodies as `{{var}}` (`{{baseUrl}}`, `{{token}}`). Scripts also set + chain vars between requests.

## Exit codes (the contract CI branches on)

- `0` — every assertion passed
- `1` — an assertion failed
- `2` — network/exec error, bad path, or collection/request not found

`>2` = crash/signal. Treat `0` green; non-zero is the CI gate (a typo'd name/path = exit 2, not a silent pass).

## Guardrails

- Run **inside** the project with `collections/` (or pass `--collections`). None found → tell the user, don't invent a path.
- Collection format = the contract with the VS Code extension. Keep edits valid Postman v2.1 JSON so both sides stay synced.
- Pick the action skill: running → `run`, test scripts → `test`, single request/curl → `send`, bring data in → `import`.
