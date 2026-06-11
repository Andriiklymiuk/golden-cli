---
name: openapi
description: Use when the user wants an OpenAPI / Swagger spec out of their golden collections — "generate openapi from the collections", "make a swagger doc", "export an OpenAPI spec", "I need a spec for the API client generator". Converts discovered Postman v2.1 collections into an OpenAPI 3.0 JSON document (paths, methods, path + query parameters, request-body examples, tags per collection). NOT for importing a spec (that's the import skill) or running collections (run skill).
---

# golden openapi

Convert the git-synced `collections/` into an **OpenAPI 3.0** spec — the reverse of
`golden import --from openapi`. Useful for Swagger UI, `editor.swagger.io`, or feeding an
API-client generator. The collections stay the source of truth; the spec is derived.

```bash
golden openapi [PATHS…] [-o FILE] [--title TITLE] [--server URL]
```

## Resolve intent → command

| Intent | Command |
|--------|---------|
| Spec for all collections → stdout | `golden openapi collections/` |
| Write to a file | `golden openapi collections/ -o openapi.json` |
| Set the API title | `golden openapi collections/ --title "My API"` |
| Record a concrete server | `golden openapi collections/ --server https://api.example.com` |
| One collection only | `golden openapi "collections/<name>.json" -o openapi.json` |

- `PATHS` empty → all discovered collections (same discovery as `run`/`list`:
  `collections/`, `.golden/`, `.retriever/`, or `--collections <path>` / `GOLDEN_COLLECTIONS_PATHS`).
- Default `--server` is `{{baseUrl}}` (matches the collection variable). Pass a real URL
  for a doc that's directly browsable in Swagger UI.

## What maps to what

- Each **request** → a path operation. `{{var}}` and `:param` URL segments become OpenAPI
  `{param}` path parameters; `?a=b` query keys become `query` parameters.
- A **raw JSON body** → `requestBody` with the body as the example; a **GraphQL body** →
  `{ "query": … }` example.
- Each request is **tagged by its collection name**, so Swagger UI groups by collection.
- Methods, paths, and params are real (read from the collection); responses are a generic
  `200` + `default` — golden has no response schema to infer, so flesh those out if needed.

## Guardrails

- Run where the collections live (cwd has `collections/`, else `--collections <path>`).
  None found → tell the user, don't invent paths.
- The spec is **generated** — regenerate after collections change rather than hand-editing
  it. Keep it next to (not inside) `collections/` if committing, so `golden run` doesn't
  try to execute the spec as a collection.
- For a repo whose collections are themselves generated (e.g. from routes), this gives a
  spec that tracks the real API for free — re-run both together.
