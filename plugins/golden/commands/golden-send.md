---
description: Fire a single request from a collection and print the response, or emit the equivalent curl. Pass the collection + request and any intent (e.g. "send the login request against .env.local", "show me the masked curl for the create-user request", "download the report from this request to report.pdf").
---

Run the golden **send** flow for the request in `$ARGUMENTS`.

- `$ARGUMENTS` = a collection + request name and what you want (send vs curl, an env, output file, mask). Empty → `golden list` first to find the request, then ask.
- Needs the collection on disk under `collections/` (or `--collections <path>`).

Follow the `send` skill (`plugins/golden/skills/send/SKILL.md`): resolve the collection/request name (use `golden list` if unknown), then either `golden send <COLLECTION> <REQUEST>` with the right `--env`/`--output`/`--max-size`/`--cookies`/`--open` flags, or `golden curl <COLLECTION> <REQUEST>` for a pasteable curl. Always add `--mask` to a curl before sharing it (redacts Authorization/Cookie/API-key headers). To assert on the response, hand off to the `test` skill; to run many requests, the `run` skill.
