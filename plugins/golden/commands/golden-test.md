---
description: Write, debug, or interpret golden's pm.*/chai test scripts — add assertions, chain a value (like a token) between requests, explain a failing assertion, or make the API tests gate CI. Pass the request/collection and what you want (e.g. "assert login returns 200 and save the token", "why is the users test failing").
---

Run the golden **test** flow for the request in `$ARGUMENTS`.

- `$ARGUMENTS` = what to assert or which failure to debug (a request name + intent). Empty → ask which request/collection.
- Needs the collection on disk under `collections/` (or `--collections <path>`).

Follow the `test` skill (`plugins/golden/skills/test/SKILL.md`): reproduce the failure with `golden run … --filter '<request>'` and read the failing assertion (Phase 1), compare the script against the real response via `golden send` (Phase 2), fix the assertion or chain the missing variable keeping the file valid Postman v2.1 JSON (Phase 3), and re-run to exit `0` — distinguishing a real assertion failure (exit 1) from a network/exec error (exit 2) (Phase 4). To gate CI: `golden run collections/ --reporter junit --output results.xml`.
