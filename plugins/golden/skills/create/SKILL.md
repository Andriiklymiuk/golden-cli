---
name: create
description: Use when the user wants to generate or scaffold golden collections for a project or multi-repo workspace — "create a collection", "generate API tests for my services", "scaffold collections for this corgi stack", "make a status check for every service", "build a collection from our OpenAPI/GraphQL". Detects a corgi-compose.yml workspace (or sibling repo folders, or the current single service) and writes a per-service collection under each service's collections/ — either quick health/status checks or full requests + tests covering REST (GET/POST/PUT/PATCH/DELETE) and GraphQL, discovered from an OpenAPI/GraphQL spec in the repo, a live spec endpoint, or the framework's routes. NOT for running collections (run skill), firing one request (send skill), or importing one already-known source (import skill).
---

# golden create

Auto-generate golden collections for a workspace. One collection per service under that service's `collections/`. REST + GraphQL, with tests. Two modes: quick **status check**, or **full** requests.

## Guardrails (non-negotiable)

- Generated collection holds **every method** — GET/POST/PUT/PATCH/DELETE + GraphQL queries **and** mutations — each with a test. That's the deliverable; the user runs it deliberately with `golden run`.
- **Generation-time live probing is read-only by default** (GET, GraphQL introspection/query). NEVER auto-fire a mutating request (POST/PUT/PATCH/DELETE, GraphQL mutation) while generating unless the user confirms the target is local/safe. Still write the request + its test.
- Secrets = `{{vars}}` only. Never bake tokens/keys. Base URLs/ports → collection variables.
- Don't overwrite or duplicate an existing collection. Existing one → **extend it** (add only the missing endpoints via `golden import --strategy add`), never recreate; keep its existing requests, tests, and edits intact.

## Phase 0 — Locate + enumerate services

- **`corgi-compose.yml` present → authoritative.** Read each `services.<name>`: `path`/`cloneFrom` (repo to scan), `port` + `portAlias`, `healthCheck` (HTTP path), `depends_on_services[].{scheme,suffix}`, `localhostNameInEnv`, `tunnel.hostname`. Each service = one target. Base URL = `{scheme|http}://{localhost}:{port}{suffix?}`.
- **No compose → fallback:** sibling subdirs that look like service repos (markers: `package.json`, `go.mod`, `Cargo.toml`, `pyproject.toml`, `pom.xml`, `composer.json`, `Gemfile`). Each = one target.
- **cwd itself is one service repo → single target** (write into `collections/` here).
- **Discover + inventory existing collections** per target the way the extension scans — top-level `*.json` in `collections/`, `.retriever/`, `.golden/`. For each, read it and build a **coverage set** keyed by **method + normalized path** (REST) and **operation type + root field** (GraphQL) — NOT by request name, which varies. A big service (e.g. `core`) usually already HAS a collection — add only what's missing.
- **Normalized path** (the dedup key): strip the host/base var, lower-case the method, drop the query string, and collapse concrete id segments (numbers, UUIDs, slugs after a known collection) to `:param`. So `GET /users/123`, `get /users/{{userId}}/`, and `GET {{coreUrl}}/users/42?x=1` are the **same** key.

## Phase 1 — Ask the mode (upfront)

Ask once:

- **(A) status check** — one health GET per service (`{{<service>Url}}{healthCheck|/}`); test asserts `pm.response.code` < 500 (or `=== 200` when the healthCheck path is known). Fast, always works, no spec needed.
- **(B) full** — discover real endpoints per service, REST + GraphQL, each with a test.

## Phase 2 — Build per service → `<service>/collections/<service>.json`

Ensure `<service>/collections/` exists (single service → `collections/` in cwd). Base URL → collection variable `{{<service>Url}}`. Group requests into folders inside the collection by area / tag / GraphQL-type.

**Existing collection → extend, never replace.** The common case for a big service (e.g. `core`) is that a collection already exists. Keep it: for each discovered endpoint compute its dedup key (method + normalized path; GraphQL op type + root field) and **skip it if that key is already in the Phase-0 coverage set** — even when the existing request has a different name. This makes re-running idempotent: no near-duplicates from `/users/123` vs `/users/{{id}}`, `GET` vs `get`, or a trailing slash. Add ONLY the genuinely new endpoints, grouped into folders by area/tag, leaving every existing request/test/edit untouched. Add via `golden import <missing>.json --from raw|postman --name "<existing collection name>" --strategy add` (merged items keep their own names) or by editing the collection's `item` tree directly. Re-run `golden list` to confirm the new requests landed and nothing was lost.

**Status mode:** one GET to the health path + the `<500` test. Done.

**Full mode — discovery, in order (stop at the first that yields endpoints):**

1. **Spec in the repo.** Search `path` for OpenAPI/Swagger (`openapi.{json,yaml,yml}`, `swagger.{json,yaml}`, `api/openapi*`, `docs/`) or GraphQL SDL (`*.graphql`, `*.gql`, `schema.graphql`). REST → `golden import <spec> --from openapi --name <service>`. GraphQL → build `mode:graphql` requests from the SDL.
2. **Live spec endpoint** (server reachable — GET the health path succeeds): try `/openapi.json`, `/swagger.json`, `/v3/api-docs`, `/swagger/v1/swagger.json` (REST), or POST a GraphQL **introspection** query to `/graphql` (read-only). Got it → import/build.
3. **Framework route scan** (no spec): detect the stack from the repo and read routes from source —
   - Node — Express/Fastify/Koa (`app.<method>('/…')`), NestJS (`@Get/@Post` + controller path), Apollo/nestjs-graphql resolvers.
   - Go — gin/echo/chi/mux (`r.GET("/…")`), gqlgen schema.
   - Python — FastAPI (`@app.get`), Flask (`@app.route`), Django `urls.py`, graphene.
   - Ruby/PHP — Rails routes, Laravel routes.
   Build a request per route (method + path). Found nothing → fall back to a status check for that service.

**Bodies + tests.** Write methods → scaffold a body from the spec schema / DTO, else a `{{var}}` placeholder. **GraphQL bodies MUST use `"mode": "graphql"` with `body.graphql.query` (string) + `body.graphql.variables` (object)** — never `mode: raw` with a JSON `{"query": …}` string; raw GraphQL bodies don't render in the extension's GraphQL editor. Tests: REST asserts status < 400 (or the documented code); GraphQL asserts `errors` absent and `data` present; content-type when known. Chain obvious vars (login token → `{{token}}`) when discoverable.

## Phase 3 — Report + run-all

- List each created/updated collection with its path.
- Discovery is **non-recursive**, so to run every service from the workspace root pass each root:
  `golden run --collections <svc1>/collections --collections <svc2>/collections …` — or `GOLDEN_COLLECTIONS_PATHS="<svc1>/collections:<svc2>/collections" golden run`.
- One service: `cd <svc> && golden run collections/`.
- Remind: base URLs are pre-filled as `{{<service>Url}}`; tokens/secrets go in a `.env`, picked with `--env`.
