---
name: import
description: Use when the user wants to bring an external source into golden's collections — "import this Postman collection", "turn this OpenAPI spec into requests", "import from this curl", "import a folder of raw requests". Imports Postman/raw/folder/OpenAPI/curl into the git-synced collections/ format, with a merge strategy when the destination exists. NOT for running (run skill), single requests (send skill), or writing tests (test skill).
---

# golden import

Bring an external API definition into the `collections/` format the whole toolchain (CLI, TUI, VS Code extension) shares.

```bash
golden import <SOURCE> [--name <NAME>] [--from <KIND>] [--strategy <STRATEGY>]
```

- `<SOURCE>` — a `.json`/spec file, a directory, or a curl command string.
- `--from` — input kind: `auto` (default, detect), `postman`, `raw`, `folder`, `openapi`, `curl`.
- `--name` — name for the imported collection (defaults to the file/`info.name`).
- `--strategy` — merge when the destination exists: `skip` (default), `add`, `replace`.

## Examples

```bash
golden import postman_collection.json                   # auto-detect
golden import openapi.yaml --from openapi --name "My API"
golden import ./requests/ --from folder                 # a directory of raw requests
golden import "curl https://api.example.com/v1/ping"    # straight from a curl string
```

## Workflow

1. **Identify the source.** Postman export, OpenAPI/Swagger spec, folder of raw requests, or a curl line. Use `--from auto` unless it guesses wrong, then set it explicitly.
2. **Choose a strategy.** New collection → `skip` is fine. Updating an existing one → `add` to append, `replace` to overwrite matching items. Default `skip` never clobbers.
3. **Import**, then `golden list` to confirm requests landed, and `golden send`/`golden run` to verify they fire.
4. **Commit.** Result = Postman v2.1 JSON under `collections/` — commit so the VS Code extension + CI pick it up. Add assertions with the `test` skill if it should gate CI.
