---
description: Import an external source into golden's collections/ format — a Postman export, an OpenAPI/Swagger spec, a folder of raw requests, or a curl string. Pass the source + any intent (e.g. "import openapi.yaml as My API", "import this curl", "merge this Postman export into the existing collection").
---

Run the golden **import** flow for the request in `$ARGUMENTS`.

- `$ARGUMENTS` = the source (file path, directory, or curl string) plus name/merge intent. Empty → ask for the source.

Follow the `import` skill (`plugins/golden/skills/import/SKILL.md`): identify the source and set `--from` (`auto` unless detection guesses wrong: `postman`/`raw`/`folder`/`openapi`/`curl`), pick a `--strategy` (`skip` default / `add` / `replace`) so existing collections aren't clobbered, run `golden import <SOURCE> [--name …]`, then verify with `golden list` + `golden send`/`golden run` and commit the resulting Postman v2.1 JSON under `collections/`. Add assertions via the `test` skill if it should gate CI.
