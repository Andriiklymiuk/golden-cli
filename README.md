<div align="center">

<img src="resources/icon.png" width="128" height="128" alt="golden">

# 🦮 golden

**Run your Postman collections from the terminal — in CI, from an AI agent, or in a TUI.**

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
[![Reliability Rating](https://sonarcloud.io/api/project_badges/measure?project=Andriiklymiuk_golden-cli&metric=reliability_rating)](https://sonarcloud.io/summary/new_code?id=Andriiklymiuk_golden-cli)
[![Maintainability Rating](https://sonarcloud.io/api/project_badges/measure?project=Andriiklymiuk_golden-cli&metric=sqale_rating)](https://sonarcloud.io/summary/new_code?id=Andriiklymiuk_golden-cli)

</div>

A single static binary that runs the Postman v2.1 collections already in your repo — fires the requests, runs your `pm.*` test scripts, substitutes `.env` vars, and reports pass/fail with a CI/agent-friendly exit code. Run it bare for a full-screen TUI.

It's the command-line sibling of [Golden Retriever](https://github.com/Andriiklymiuk/golden-retriever) (the VS Code extension): **same git-synced `collections/` files**, edited in the editor, run in CI — no export, no second source of truth.

## Quick start

```bash
brew install andriiklymiuk/homebrew-tools/golden   # or see Install below

golden init                 # seed a sample collection under collections/
golden run collections/     # run everything; exit 0=pass 1=fail 2=error
golden doctor               # check the workspace + collections are healthy
golden                       # no args → interactive TUI
```

## Why

- **Gate every PR** — `golden run collections/ --reporter junit --output results.xml`. Non-zero exit fails the build.
- **Drive it from an AI agent** — non-interactive, speaks `--reporter json`, branchable exit codes, and ships a [Claude Code plugin](#drive-it-with-an-ai-agent).
- **One file everywhere** — the same collection runs in your editor, your terminal, CI, and an agent.
- **Nothing to install at runtime** — static musl Linux binaries (rustls, no OpenSSL), native macOS (Intel + Apple Silicon) and Windows.
- **Two binaries, identical:** `golden` and the short alias `gr`.

## Commands

| Command                                           | What it does                                                                         |
| ------------------------------------------------- | ------------------------------------------------------------------------------------ |
| `golden run [PATHS…]`                             | Run collections; reporters, `--filter`, `--env`, `--iterations`, `--data`, `--bail`. |
| `golden list [PATHS…]`                            | List discovered collections + requests.                                              |
| `golden send <COLL> <REQ>`                        | Fire one request, print the response.                                                |
| `golden curl <COLL> <REQ>`                        | Print the equivalent `curl` (`--mask` redacts secrets).                              |
| `golden import <SOURCE>`                          | Import Postman / OpenAPI 3.x / Swagger 2.0 / curl / folder.                          |
| `golden init`                                     | Create `collections/` and seed the sample.                                           |
| `golden history <list\|replay N\|on\|off\|clear>` | Persisted request history.                                                           |
| `golden doctor [--fix]`                           | Health-check the workspace; `--fix` seeds `collections/`.                            |
| `golden upgrade` (alias `update`)                 | Self-update via your install method (brew / installer).                              |
| `golden completion <shell>`                       | Shell completions (bash/zsh/fish/powershell/elvish).                                 |

`--collections <PATH>` (repeatable) and `GOLDEN_COLLECTIONS_PATHS` override discovery on any command. `<COLL>` is a collection's `info.name` or a file path.

## Recipes

```bash
golden run collections/ --env .env.staging            # pick an environment
golden run collections/ --filter 'Auth/*'             # only matching folders/requests
golden run collections/ --data users.csv              # data-driven: one iteration per row
golden run collections/ --reporter junit --output results.xml   # CI report
golden run collections/ --reporter json --output run.json       # machine-readable for an agent
golden send "My API" "Login" --env .env.local         # fire one request
golden curl "My API" "Login" --mask                   # safe-to-paste curl
golden import openapi.yaml --from openapi --name "Billing"      # scaffold from a spec
golden run --collections api/collections --collections web/collections   # many services at once
```

## In CI

```yaml
- name: API tests
  run: |
    curl --proto '=https' --tlsv1.2 -LsSf \
      https://github.com/Andriiklymiuk/golden-cli/releases/latest/download/golden-cli-installer.sh | sh
    golden run collections/ --reporter junit --output results.xml --timeout 15000
```

Exit code is the contract: **`0`** all passed · **`1`** an assertion failed · **`2`** network/exec error or bad path/name · **`>2`** crash. A typo'd name is `2` (fails the build), not a silent pass.

## Drive it with an AI agent

golden is non-interactive and speaks JSON, so an agent (or any script) can run it and branch on the result. A bundled [Claude Code](https://claude.com/claude-code) plugin teaches one to do it:

```text
/plugin marketplace add Andriiklymiuk/golden-cli
/plugin install golden@golden
```

| Skill · command             | Ask Claude…                                                                     |
| --------------------------- | ------------------------------------------------------------------------------- |
| `create` · `/golden-create` | _"scaffold collections for this corgi stack"_ · _"status check per service"_    |
| `run` · `/golden-run`       | _"run the collections with JUnit output for CI"_                                |
| `test` · `/golden-test`     | _"assert login returns 200 and save the token"_ · _"why is this test failing?"_ |
| `send` · `/golden-send`     | _"give me a masked curl for the create-user request"_                           |
| `import` · `/golden-import` | _"import this OpenAPI spec as Billing API"_                                     |

<details>
<summary>The <code>--reporter json</code> shape an agent parses</summary>

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

Read `.totals.failed_assertions` (or branch on the exit code); drill into `.collections[].iterations[].requests[].assertions[].error` for the exact failure. JUnit and TAP reporters are there too; `--output` is written even when the run fails. (Gotcha: a `--filter` matching nothing runs 0 requests and still exits 0 — check `.totals.requests`.)

</details>

## Collections & environments

- **Collections** are Postman v2.1 JSON under `collections/` (also `.golden/`, `.retriever/`) — the layout the [Golden Retriever](https://github.com/Andriiklymiuk/golden-retriever) extension reads/writes, so they live in git next to your code.
- **Environments** are `.env` files — `--env <name|path>`; vars substitute into URLs/headers/bodies (`{{baseUrl}}`, `{{token}}`), plus Postman dynamic vars (`{{$guid}}`, `{{$timestamp}}`, `{{$randomEmail}}`, …).
- **Test scripts** run a `pm.*` / chai sandbox (assertions, `pm.environment.set` chaining, `pm.iterationData`, `setNextRequest`, crypto, `atob`/`btoa`); a conformance suite diffs golden against newman.

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

Then `golden upgrade` keeps it current via whichever method you used.

<details>
<summary>Build, test & release (contributing)</summary>

```bash
git clone https://github.com/Andriiklymiuk/golden-cli && cd golden-cli
cargo build --workspace
cargo test --workspace

# what CI gates on
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p xtask-conformance --release   # newman conformance oracle (needs Node)
make checkSampleCollection                 # embedded sample == on-disk sample
```

**Layout:** `crates/golden-cli` (the `golden`/`gr` binaries + TUI) · `crates/golden-core` (model, HTTP runner, env, `pm.*` sandbox, import) · `xtask-conformance` (newman diff) · `plugins/golden` (Claude Code plugin).

**Release:** `make incrementVersionPatch` (bumps `crates/golden-cli/Cargo.toml` + the plugin) → commit → push. `tag.yml` tags `vX.Y.Z` and dispatches `release.yml`, which builds all five targets with cargo-dist, publishes a GitHub Release (commit history in the notes), and bumps the Homebrew formula. Only secret needed: `HOMEBREW_TAP_TOKEN`. Targets: macOS arm64/x64, Linux arm64/x64 (musl, static), Windows x64.

</details>

## License

MIT © Andrii Klymiuk
