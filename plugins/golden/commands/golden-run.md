---
description: Run Postman v2.1 collections with the golden CLI — whole directory or a slice, with an environment, filter, iterations, and CI reporters (pretty/junit/json/tap). Pass what to run in plain words (e.g. "all collections", "just the auth collection against staging", "everything with junit output for CI"); empty = run collections/.
---

Run the golden **run** flow for the request in `$ARGUMENTS`.

- `$ARGUMENTS` = plain-words description of what to run (all, one collection, a filter, an environment, CI output). Empty → `golden run collections/`.
- Must run where the collections live (cwd has `collections/`, `.golden/`, or `.retriever/`), else pass `--collections <path>`. None found → tell the user to open the project first.

Follow the `run` skill (`plugins/golden/skills/run/SKILL.md`) end to end: locate collections + check the `Makefile` for an existing run target (Phase 1), resolve the environment and confirm it points where intended (Phase 2), build the `golden run` command with the right reporters/filter/timeout (Phase 3), then report pass/fail counts + the exit code, never swallowing it (Phase 4). For CI, add `--reporter junit --output results.xml` and let a non-zero exit fail the step.
