---
name: run
description: Use when the user wants to run Postman v2.1 collections with the golden CLI — "run the collection", "run the API tests", "run collections in CI", "run against staging", "run only the auth requests". Runs one/many collections (or a directory) headlessly, picks an environment, filters, repeats, and emits CI-ready reporters (pretty/junit/json/tap) with exit codes a pipeline can branch on. NOT for writing/debugging test scripts (use the test skill), a single request (send skill), or importing sources (import skill).
---

# golden run

Run collections headlessly — local or CI. What to run + how = read from repo (`collections/`, `.env*`, `Makefile`), never hard-coded.

## Resolve intent → command

| Intent | Command |
|--------|---------|
| Run everything | `golden run collections/` |
| One collection | `golden run collections/<name>.json` |
| Pick an environment | `golden run collections/ --env <name\|path>` |
| Only matching requests/folders | `golden run collections/ --filter '<glob>'` |
| Only certain HTTP verbs | `golden run collections/ -X GET` (repeatable, case-insensitive) — composes with `--filter` |
| Safe read-only sweep (no writes) | `golden run collections/ -X GET` — never fires POST/PUT/PATCH/DELETE; ideal against staging/prod |
| Repeat N times | `golden run collections/ --iterations <N>` |
| Data-driven (1 iteration per row) | `golden run collections/ --data <file.json\|csv>` — rows overlay vars + feed `pm.iterationData` |
| Stop at first failure | `golden run collections/ --bail` |
| Per-request timeout | `golden run collections/ --timeout <ms>` |
| Skip TLS verification | `golden run collections/ --insecure` |

`--collections <path>` (global, repeatable) or `GOLDEN_COLLECTIONS_PATHS` overrides discovery roots. Names match a collection's `info.name` or a file path.

## Reporters (CI output)

`--reporter <pretty|junit|json|tap>` — repeatable; `pretty` default. `--output <file>` writes to a file instead of stdout.

```bash
# pretty on stdout for humans + JUnit on disk for the test reporter
golden run collections/ --reporter pretty --reporter junit --output results.xml
```

## Exit codes — DO NOT swallow them

- `0` all passed · `1` an assertion failed · `2` network/exec error OR bad path / no collections found. `>2` = crash.
- In CI, let a non-zero exit fail the step — that's the gate.
- Under `bash -eo pipefail`, capture with `code=0; golden run … || code=$?` so errexit doesn't kill the step on exit 1 before you inspect it.

## Gotchas (verified)

- **Summary line:** `N requests, M failed | X assertions, Y failed | Zms`. Read N — a `--filter` that matches nothing runs **0 requests and still exits 0**. Green ≠ covered; check the count.
- **`golden init`** seeds `collections/<info.name-slug>.json` (e.g. "Fake APIs Collection" → `fake-apis-collection.json`), NOT `sample-collection.json`. Prefer the `collections/` directory form so the filename never matters.
- `--reporter json` (machine-readable) and `--reporter junit --output results.xml` (the file is written even when the run fails) are the two CI/agent outputs.

## Phases

1. **Locate.** cwd has `collections/` (or `.golden/`/`.retriever/`)? None → tell the user or pass `--collections`. Check `Makefile` for an existing run target the team uses.
2. **Resolve env.** Multiple `.env*`? Ask or use the one named. Confirm it points where intended (local vs staging) before running anything sensitive.
3. **Run.** Build the command above. Long/flaky external APIs → add `--timeout`. CI → add `--reporter junit --output results.xml`.
4. **Report.** Surface pass/fail counts + the exit code. Exit `2` = host/network problem, not a test failure — say so. Assertion failures → hand to the `test` skill to fix.
