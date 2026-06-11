---
description: Generate an OpenAPI 3.0 / Swagger spec from golden collections — whole directory or a slice, with an output file, title, and server URL. Pass intent in plain words (e.g. "openapi spec to openapi.json", "swagger doc titled Core API", "spec for the ecom collection"); empty = all collections to stdout.
---

Run the golden **openapi** flow for the request in `$ARGUMENTS`.

- `$ARGUMENTS` = plain-words description (output file, title, server, which collections). Empty → `golden openapi collections/` to stdout.
- Must run where the collections live (cwd has `collections/`, `.golden/`, or `.retriever/`), else pass `--collections <path>`. None found → tell the user to open the project first.

Follow the `openapi` skill (`plugins/golden/skills/openapi/SKILL.md`): pick the collections, choose `-o` (file vs stdout), `--title` (default = first collection name), and `--server` (default `{{baseUrl}}` — pass a real URL for a browsable Swagger doc). Build the `golden openapi` command, run it, and report where the spec was written + path/operation counts. If committing the spec, keep it OUTSIDE `collections/` so `golden run` doesn't treat it as a collection.
