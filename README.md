<div align="center">

<img src="resources/icon.png" width="128" height="128" alt="golden">

# 🦮 golden

**Run and test your Postman v2.1 collections from the terminal — in CI, from an AI agent, in a script, or in a full-screen TUI.**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Homebrew](https://img.shields.io/badge/install-brew-orange.svg)](#install)

[![CI](https://github.com/Andriiklymiuk/golden-cli/actions/workflows/golden-ci.yml/badge.svg)](https://github.com/Andriiklymiuk/golden-cli/actions/workflows/golden-ci.yml)
[![Release](https://github.com/Andriiklymiuk/golden-cli/actions/workflows/release.yml/badge.svg)](https://github.com/Andriiklymiuk/golden-cli/releases)

[![Reliability Rating](https://sonarcloud.io/api/project_badges/measure?project=Andriiklymiuk_golden-cli&metric=reliability_rating)](https://sonarcloud.io/summary/new_code?id=Andriiklymiuk_golden-cli)
[![Bugs](https://sonarcloud.io/api/project_badges/measure?project=Andriiklymiuk_golden-cli&metric=bugs)](https://sonarcloud.io/summary/new_code?id=Andriiklymiuk_golden-cli)
[![Code Smells](https://sonarcloud.io/api/project_badges/measure?project=Andriiklymiuk_golden-cli&metric=code_smells)](https://sonarcloud.io/summary/new_code?id=Andriiklymiuk_golden-cli)

[![Security Rating](https://sonarcloud.io/api/project_badges/measure?project=Andriiklymiuk_golden-cli&metric=security_rating)](https://sonarcloud.io/summary/new_code?id=Andriiklymiuk_golden-cli)
[![Vulnerabilities](https://sonarcloud.io/api/project_badges/measure?project=Andriiklymiuk_golden-cli&metric=vulnerabilities)](https://sonarcloud.io/summary/new_code?id=Andriiklymiuk_golden-cli)

[![Maintainability Rating](https://sonarcloud.io/api/project_badges/measure?project=Andriiklymiuk_golden-cli&metric=sqale_rating)](https://sonarcloud.io/summary/new_code?id=Andriiklymiuk_golden-cli)
[![Lines of Code](https://sonarcloud.io/api/project_badges/measure?project=Andriiklymiuk_golden-cli&metric=ncloc)](https://sonarcloud.io/summary/new_code?id=Andriiklymiuk_golden-cli)
[![Technical Debt](https://sonarcloud.io/api/project_badges/measure?project=Andriiklymiuk_golden-cli&metric=sqale_index)](https://sonarcloud.io/summary/new_code?id=Andriiklymiuk_golden-cli)

[![Quality Gate Status](https://sonarcloud.io/api/project_badges/measure?project=Andriiklymiuk_golden-cli&metric=alert_status)](https://sonarcloud.io/summary/new_code?id=Andriiklymiuk_golden-cli)
[![Coverage](https://sonarcloud.io/api/project_badges/measure?project=Andriiklymiuk_golden-cli&metric=coverage)](https://sonarcloud.io/summary/new_code?id=Andriiklymiuk_golden-cli)
[![Duplicated Lines (%)](https://sonarcloud.io/api/project_badges/measure?project=Andriiklymiuk_golden-cli&metric=duplicated_lines_density)](https://sonarcloud.io/summary/new_code?id=Andriiklymiuk_golden-cli)

</div>

`golden` is a single static binary that runs the same API collections you already keep in your repo. Point it at a folder of Postman v2.1 JSON files and it fires the requests, runs your test scripts, substitutes `.env` variables, and reports pass/fail with machine-readable output (JSON, JUnit, TAP) and exit codes a pipeline — or an AI agent — can branch on. Run it with no arguments and you get an interactive TUI to browse, edit, and send requests without leaving the terminal.

It's the command-line sibling of [Golden Retriever](https://github.com/Andriiklymiuk/golden-retriever) (the VS Code extension). Both read and write the **same git-synced `collections/` format**, so you edit a request in the editor and run it in CI from the exact same file — no export step, no second source of truth.

```text
collections/*.json   ─►  golden run collections/
                           ├─ resolve the active .env
                           ├─ fire every request (REST + GraphQL)
                           ├─ run pm.* / chai test scripts
                           └─ report pretty | json | junit | tap
                                      ↓
                           pass/fail + an exit code CI / an agent branches on
```

## Why

Your API collection already lives in the repo next to the code. Until now you could only run it by hand in a GUI. `golden` makes that same file a first-class part of your workflow:

- **In CI** — `golden run collections/ --reporter junit --output results.xml` gates every PR against your real API. Non-zero exit fails the build on its own.
- **From an AI agent** — every command is one-shot and non-interactive, speaks `--reporter json`, and returns exit codes an agent can branch on. A bundled [Claude Code plugin](#claude-code-plugin) teaches an agent to drive it.
- **In the terminal** — a TUI to browse the tree, edit a request, send it, and read the response — no context switch to a GUI.
- **One-offs** — fire a single request (`golden send`), turn one into a `curl` you can paste anywhere (`golden curl`), or replay something from history.
- **Same files everywhere** — the VS Code extension, your teammates, CI, and agents all read the one git-tracked collection. Edit once, run anywhere.

Static musl Linux binaries (rustls, no OpenSSL), native macOS (Intel + Apple Silicon) and Windows builds — nothing to install at runtime.

## Install

```bash
# Homebrew (macOS + Linux)
brew install andriiklymiuk/homebrew-tools/golden

# shell installer (macOS + Linux)
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/Andriiklymiuk/golden-cli/releases/latest/download/golden-cli-installer.sh | sh

# Windows (PowerShell)
irm https://github.com/Andriiklymiuk/golden-cli/releases/latest/download/golden-cli-installer.ps1 | iex

# from source (Rust 1.82+)
cargo install --git https://github.com/Andriiklymiuk/golden-cli golden-cli
```

Every install ships two binaries: **`golden`** and the short alias **`gr`** — identical behaviour.

```bash
golden --version
```

## Quick start

```bash
golden init                 # create collections/ and seed a sample collection
golden list                 # show discovered collections + their requests
golden run collections/     # run everything, pretty output
golden                       # no args → launch the interactive TUI
```

`golden init` seeds `collections/<collection-name>.json` (the sample lands as `fake-apis-collection.json` — the filename is derived from the collection's `info.name`). By default `golden` discovers collections under `collections/` in the current directory; override the roots with `--collections <path>` (repeatable, global) or the `GOLDEN_COLLECTIONS_PATHS` environment variable.

## Made for AI agents & CI

golden is non-interactive by design (the TUI is opt-in — only a bare `golden` opens it). Everything an agent or pipeline needs is on stdout and in the exit code.

**Exit codes — the contract you branch on:**

| Code | Meaning                                                                  |
| ---- | ------------------------------------------------------------------------ |
| `0`  | every assertion passed                                                   |
| `1`  | an assertion failed (the request ran, the check didn't pass)             |
| `2`  | network / execution error, a bad path, or a collection/request not found |
| `>2` | crash / signal                                                           |

`0`, `1`, and `2` all mean "golden ran"; a typo'd name or path is `2`, so it fails CI instead of passing quietly.

**Machine-readable results** — `--reporter json` prints a structure an agent parses directly:

```bash
golden run collections/ --reporter json --output run.json --timeout 15000
code=$?     # branch on this
```

```jsonc
{
  "collections": [
    {
      "name": "Fake APIs Collection",
      "iterations": [
        {
          "index": 1,
          "requests": [
            {
              "name": "Get API Root",
              "method": "GET",
              "url": "https://swapi.info/api",
              "status": 200,
              "time_ms": 141,
              "assertions": [
                { "name": "Status code is 200", "passed": true, "error": null }
              ],
              "error": null
            }
          ]
        }
      ]
    }
  ],
  "totals": {
    "requests": 1,
    "failed_requests": 0,
    "assertions": 2,
    "failed_assertions": 0,
    "total_ms": 141
  }
}
```

An agent reads `.totals.failed_assertions` (or branches on the exit code) and drills into `.collections[].iterations[].requests[].assertions[].error` for the exact failure. JUnit (`--reporter junit`) and TAP (`--reporter tap`) are there for test reporters; `--output` is written **even when the run fails**.

**Drive it from Claude** — the [Claude Code plugin](#claude-code-plugin) ships skills (`run`, `test`, `send`, `import`) so an agent can run collections, gate CI, debug a failing assertion, and import specs the same way you would.

> **Gotcha for agents/CI:** a `--filter` that matches no requests runs **0 requests and still exits 0**. Green ≠ covered — read the `N requests` count in the summary (`1 requests, 0 failed | 2 assertions, 0 failed | 141ms`) or `.totals.requests`.

## The TUI

Run `golden` with no subcommand to open the full-screen terminal UI: a collection tree on the left, request/response panes on the right. Browse folders, edit a request inline, send it, and read the formatted response — all from the keyboard. It's the fastest way to poke at an API without leaving your shell.

## Commands

| Command                                           | What it does                                                                    |
| ------------------------------------------------- | ------------------------------------------------------------------------------- |
| `golden run [PATHS…]`                             | Run one/many collections (or a directory). Test scripts, iterations, reporters. |
| `golden list [PATHS…]`                            | List discovered collections and their requests.                                 |
| `golden send <COLLECTION> <REQUEST>`              | Fire a single request and print the response.                                   |
| `golden curl <COLLECTION> <REQUEST>`              | Print the equivalent `curl` command for a request.                              |
| `golden import <SOURCE>`                          | Import a Postman / raw / folder / OpenAPI / curl source.                        |
| `golden init`                                     | Create `collections/` and seed the sample collection.                           |
| `golden history <list\|clear\|on\|off\|replay N>` | Manage persisted request history (`.golden/history.jsonl`).                     |
| `golden completion <shell>`                       | Print a shell completion script.                                                |

`--collections <PATH>` (global, repeatable) and `GOLDEN_COLLECTIONS_PATHS` override discovery on every command. `<COLLECTION>` accepts a collection's `info.name` **or** a file path; `<REQUEST>` is the request name.

### `golden run` — run collections (CI-focused)

```bash
golden run collections/                                   # run everything, pretty output
golden run collections/fake-apis-collection.json          # one collection (filename = its info.name)
golden run collections/ --filter 'Star Wars*'             # only matching folders/requests (glob)
golden run collections/ --env .env.staging                # pick an environment
golden run collections/ --bail                            # stop at the first assertion failure
golden run collections/ --iterations 5 --timeout 15000    # repeat N times, per-request timeout (ms)
golden run collections/ --data users.json                 # data-driven: one iteration per row
golden run collections/ --reporter json --output run.json # machine-readable for an agent / dashboard
golden run collections/ --reporter pretty --reporter junit --output results.xml  # CI: human + JUnit
```

`--data` takes a JSON array of objects or a CSV (header + rows). Each row is one iteration: its keys overlay the variables (so `{{userId}}` resolves to the row's value, winning over env/collection) and are readable via `pm.iterationData.get('userId')`; `pm.info.iteration` / `pm.info.iterationCount` track progress.

Reporters: `pretty` (default), `json`, `junit`, `tap`; `--reporter` is repeatable. The summary line is `N requests, M failed | X assertions, Y failed | Zms`.

### `golden list` — discover what's there

```bash
golden list                                       # full tree: collections → folders → requests + URLs
golden list --filter 'Get All*'                   # only matching requests/folders
golden list collections/fake-apis-collection.json # restrict to one file
golden list --collections ./qa-collections         # a different discovery root
GOLDEN_COLLECTIONS_PATHS=./a:./b golden list       # multiple roots via env
golden list --filter 'SWAPI*'                      # scope to one API group before running it
```

Nothing found (empty dir or a bad path) prints `golden: no collections found` and exits `2`.

### `golden send` — fire one request

```bash
golden send "Fake APIs Collection" "Get API Root"                       # by collection name
golden send collections/fake-apis-collection.json "Get API Root"        # by collection file path
golden send "Fake APIs Collection" "Get Random User" --env .env.local   # with an environment
golden send "Fake APIs Collection" "Get Pokémon List" --timeout 5000    # bound a slow host
golden send "Fake APIs Collection" "Get All Products" --output out.json  # write the body to a file
golden send "Fake APIs Collection" "Get Company Details" --output d.bin --max-size 5242880 --force  # capped download, overwrite
golden send "Fake APIs Collection" "Get API Root" --cookies             # also print Set-Cookie headers
golden send "Fake APIs Collection" "Get Single Film" --open             # if HTML, open it in the browser
```

Duplicate request names within a collection resolve to the **first match** (the sample has "Get All Users" in two folders) — use a unique name. An unknown collection/request exits `2`.

### `golden curl` — turn a request into curl

```bash
golden curl "Fake APIs Collection" "Get API Root"            # print the equivalent curl
golden curl "Fake APIs Collection" "Get API Root" --mask     # redact Authorization/Cookie/API-key — safe to paste
golden curl "Fake APIs Collection" "Get Random User" -L -i   # follow redirects, include response headers
golden curl "Fake APIs Collection" "Search Products" -s -f   # silent, fail on HTTP errors
golden curl "Fake APIs Collection" "Get All Products" --compressed -w   # ask for gzip, print timing
golden curl "Fake APIs Collection" "Get Company Details" --download     # -O -J (save to file)
golden curl collections/fake-apis-collection.json "Get API Root"        # by collection file path
eval "$(golden curl "Fake APIs Collection" "Get API Root")"  # run the generated curl right now
```

### `golden import` — bring sources in

```bash
golden import postman_collection.json                    # auto-detect a Postman export
golden import openapi.yaml --from openapi --name "Billing API"  # an OpenAPI/Swagger spec
golden import "curl https://api.example.com/v1/ping"     # straight from a curl string
golden import ./requests/ --from folder                  # a directory of request JSON files
golden import request.json --from raw --name "Smoke"     # one request as JSON: {"method":"GET","url":"…"}
golden import other.json --strategy add                  # append into an existing collection
golden import other.json --strategy replace              # overwrite matching items
```

Imports land as `collections/<name>.json` (lower-cased from `--name`/`info.name`). `--strategy` defaults to `skip` (never clobbers).

### `golden init` — scaffold a collections folder

```bash
golden init                              # create collections/ + seed the sample (idempotent — skips if present)
golden init --collections ./api-tests    # seed into a different root
```

### `golden history` — replay what you ran

```bash
golden history on                # enable recording to .golden/history.jsonl
golden history list              # show recorded entries (newest last)
golden history replay 3          # re-run entry #3 (1-based)
golden history off               # stop recording
golden history clear             # wipe all entries
```

### `golden completion` — shell completions

```bash
golden completion zsh  > ~/.zsh/completions/_golden
golden completion bash > /etc/bash_completion.d/golden
golden completion fish > ~/.config/fish/completions/golden.fish
golden completion powershell | Out-String | Invoke-Expression
golden completion elvish > ~/.config/elvish/lib/golden.elv
```

## Use it in CI (GitHub Actions)

```yaml
- name: API tests
  run: |
    curl --proto '=https' --tlsv1.2 -LsSf \
      https://github.com/Andriiklymiuk/golden-cli/releases/latest/download/golden-cli-installer.sh | sh
    golden run collections/ --reporter junit --output results.xml --timeout 15000
```

golden exits non-zero on an assertion or network failure, so the step fails the build on its own. Feed `results.xml` to your favourite JUnit reporter. For flaky live APIs, `--timeout` makes dead hosts fail fast; assert on contract shape (status, field types) rather than exact values from live data.

## Claude Code plugin

golden ships a [Claude Code](https://claude.com/claude-code) plugin so an AI agent can drive the CLI the way you do — run collections, gate CI, debug failing assertions, fire one-off requests, and import specs.

```text
/plugin marketplace add Andriiklymiuk/golden-cli
/plugin install golden@golden
```

It adds a core **`golden`** skill (the expert on commands, the collection format, env vars, and exit codes) plus four action skills, each with a matching slash command. Just ask in plain words — the skill activates, or invoke the command directly:

| Skill · command             | Use it for                                                                | Ask Claude…                                                                                                                              |
| --------------------------- | ------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `create` · `/golden-create` | Auto-generate per-service collections across a workspace (REST + GraphQL) | _"scaffold collections for this corgi stack"_ · _"make a status check for every service"_ · _"build a full collection from our OpenAPI"_ |
| `run` · `/golden-run`       | Run collections in CI or locally; reporters + exit codes                  | _"run the collections with JUnit output for CI"_ · _"run just the SWAPI folder against staging"_                                         |
| `test` · `/golden-test`     | Write/debug `pm.*`/chai assertions; chain values between requests         | _"assert Get API Root returns 200 and save the token"_ · _"why is the Get All Users test failing?"_                                      |
| `send` · `/golden-send`     | Fire one request or emit a (maskable) curl                                | _"send Get Random User and show me the body"_ · _"give me a masked curl for Search Products"_                                            |
| `import` · `/golden-import` | Import Postman/OpenAPI/curl/folder sources                                | _"import this OpenAPI spec as Billing API"_ · _"import this curl into the existing collection"_                                          |

**Auto-generate collections for a workspace** — `/golden-create` (the `create` skill) detects a `corgi-compose.yml` workspace (or sibling repo folders, or the single service you're in), then writes a collection under each service's `collections/`. Pick **status-check** (one health GET + test per service — fast, no spec needed) or **full** (real requests + tests for REST _and_ GraphQL, discovered from an OpenAPI/GraphQL spec in the repo → a live spec endpoint → the framework's routes). It never duplicates an existing collection, keeps secrets as `{{vars}}`, and never auto-fires a mutating request (POST/PUT/PATCH/DELETE, GraphQL mutation) while generating — it writes them with tests for you to run deliberately. Then run them all:

```bash
golden run --collections api/collections --collections web/collections   # repeatable per service
# or: GOLDEN_COLLECTIONS_PATHS="api/collections:web/collections" golden run
```

The plugin lives under [`plugins/golden`](plugins/golden) (skills in `skills/`, commands in `commands/`), published via [`.claude-plugin/marketplace.json`](.claude-plugin/marketplace.json).

## Collections & environments

- **Collections** are Postman v2.1 JSON files. `golden` discovers them under `collections/` (and `.golden/`, `.retriever/`) — the same layout the [Golden Retriever](https://github.com/Andriiklymiuk/golden-retriever) VS Code extension reads and writes, so they stay in git alongside your code.
- **Environments** are `.env` files. Pick one per run/send with `--env <name|path>`; variables substitute into URLs, headers, and bodies (`{{baseUrl}}`, `{{token}}`, …).
- **Test scripts** run a `pm.*` / chai-style sandbox (assertions, variable chaining between requests) — the conformance suite diffs golden's behaviour against newman to keep it faithful.

## Build from source

```bash
git clone https://github.com/Andriiklymiuk/golden-cli
cd golden-cli
cargo build --workspace          # debug build
cargo test --workspace           # run the test suite
cargo run -p golden-cli -- list  # run the dev binary

# quality gates (what CI runs)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p xtask-conformance --release   # newman conformance oracle (needs Node + newman)
make checkSampleCollection                 # sample collection in sync
```

CI (`.github/workflows/golden-ci.yml`) runs the suite on Linux for every push (full macOS/Windows matrix on PRs), gates on clippy + rustfmt + the sample-sync check + the newman conformance oracle, dogfoods the release binary against `collections/`, and reports coverage to SonarCloud (when `SONAR_TOKEN` is set).

### Layout

| Path                                         | Purpose                                                                |
| -------------------------------------------- | ---------------------------------------------------------------------- |
| `crates/golden-cli`                          | The `golden`/`gr` binaries + TUI; embeds the sample collection.        |
| `crates/golden-core`                         | Library: model, HTTP runner, env/substitution, test sandbox, import.   |
| `xtask-conformance`                          | Conformance harness — diffs golden against the newman oracle.          |
| `collections/`                               | On-disk sample collection used by the dogfood job + integration tests. |
| `plugins/golden` · `.claude-plugin/`         | Claude Code plugin (skills + commands) and its marketplace manifest.   |
| `dist-workspace.toml` · `.github/workflows/` | cargo-dist release config + CI.                                        |

> **Note** — `crates/golden-cli/assets/sample-collection.json` (embedded in the binary) and `collections/sample-collection.json` (on disk) must stay byte-identical. Edit the asset, then run `make syncSampleCollection`; CI fails if they drift.

## Releasing

The CLI version in `crates/golden-cli/Cargo.toml` is the single source of truth.

```bash
make incrementVersionPatch    # or incrementVersionMinor / incrementVersionMajor
git commit -am "release: golden v$(make -s getVersion)"
git push
```

On push to `main`, `tag.yml` reads the version, pushes a `vX.Y.Z` tag, and dispatches `release.yml` — which builds all five targets with cargo-dist, publishes a GitHub Release (with the commit history since the last tag baked into the notes), and bumps the Homebrew formula on `andriiklymiuk/homebrew-tools`. The only required secret is `HOMEBREW_TAP_TOKEN` (a fine-grained PAT with Contents: Write on the tap repo).

### Targets built each release

| Target                       | Platform              |
| ---------------------------- | --------------------- |
| `aarch64-apple-darwin`       | macOS (Apple Silicon) |
| `x86_64-apple-darwin`        | macOS (Intel)         |
| `x86_64-unknown-linux-musl`  | Linux x86_64 (static) |
| `aarch64-unknown-linux-musl` | Linux ARM64 (static)  |
| `x86_64-pc-windows-msvc`     | Windows x64           |

## License

MIT © Andrii Klymiuk
