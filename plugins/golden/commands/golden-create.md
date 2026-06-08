---
description: Auto-generate golden collections for a project or multi-repo workspace — one per service under each service's collections/. Detects a corgi-compose.yml (or sibling repo folders, or the current single service), then either makes a quick health/status check per service or full requests + tests (REST + GraphQL) from a spec, a live spec endpoint, or the framework's routes. Pass intent (e.g. "status checks for every service", "full collection from our OpenAPI", "scaffold tests for this corgi stack"); empty = detect + ask.
---

Run the golden **create** flow for the request in `$ARGUMENTS`.

- `$ARGUMENTS` = intent: which mode (status vs full), or which services. Empty → detect the workspace and ask the mode.
- Run at the workspace root (next to `corgi-compose.yml`) for a multi-repo stack, or inside a single service repo.

Follow the `create` skill (`plugins/golden/skills/create/SKILL.md`) end to end: locate + enumerate services from `corgi-compose.yml` (or sibling repo folders, or the current service) and discover existing collections so nothing is duplicated (Phase 0); ask the mode upfront — quick status-check vs full requests+tests (Phase 1); per service, write `<service>/collections/<service>.json` with base URL as `{{<service>Url}}`, discovering endpoints via spec-in-repo → live spec endpoint → framework route scan, covering REST + GraphQL with tests (Phase 2); then report each collection and the multi-root run command (Phase 3).

Honor every guardrail: the collection includes all methods + GraphQL with tests, but generation-time live probing is **read-only by default** — never auto-fire a mutating request (POST/PUT/PATCH/DELETE, GraphQL mutation) without confirming the target is local/safe; secrets are `{{vars}}`, never baked; never overwrite an existing collection without asking.
