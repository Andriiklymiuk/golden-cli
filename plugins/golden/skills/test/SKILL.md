---
name: test
description: Use when writing, debugging, or interpreting golden's test scripts — "add a test for this request", "why is this assertion failing", "assert the status is 200", "chain the token from login into the next request", "make the API tests gate CI". Covers the pm.*/chai sandbox golden runs (assertions, variable chaining) and turning failures into a green run. NOT for just running collections (use the run skill) or firing a one-off request (send skill).
---

# golden test

golden runs each request's test scripts in a `pm.*` / chai-style sandbox — same scripting model as Postman/newman; the conformance suite diffs golden against newman to stay faithful. Use this skill to author assertions, chain values between requests, and debug failures.

## The sandbox

Scripts live on the collection / folder / request (pre-request + test scripts). Globals mirror Postman:

- `pm.response` — `.code`, `.json()`, `.text()`, `.headers`, response time.
- `pm.test("name", () => { … })` — named assertion block; a throw inside fails it.
- `pm.expect(x)` / chai `expect` — `.to.equal`, `.to.have.status`, `.to.include`, …
- `pm.environment` / `pm.variables` — `get`/`set` to read env vars and **chain** values into later requests (save a token in login's test script, use `{{token}}` after).

```js
pm.test("login returns 200 + a token", () => {
  pm.expect(pm.response.code).to.equal(200);
  const body = pm.response.json();
  pm.expect(body.token).to.be.a("string");
  pm.environment.set("token", body.token);   // chained into later {{token}}
});
```

## Workflow

1. **Reproduce.** `golden run collections/<file>.json --filter '<request>'` runs just the failing request; read the pretty output for the failing assertion name + actual value.
2. **Read the script** on the request/folder/collection. Compare assertion to real response — `golden send <collection> <request>` (the `send` skill) prints the actual body/headers.
3. **Fix script or request.** Wrong expectation → fix the assertion. Missing chained var → set it in the upstream request's test script, reference `{{var}}`. Keep valid Postman v2.1 JSON so the VS Code extension stays synced.
4. **Re-run to green.** `golden run …`; confirm exit `0`. Distinguish a real assertion failure (exit `1` — fix it) from a network/exec error (exit `2` — host/timeout, not the test).

## Gate CI

```bash
golden run collections/ --reporter junit --output results.xml
```

Non-zero exit fails the build. For deterministic CI, assert on contract shape (status, fields, types) over exact values from live data; add `--timeout` so dead hosts fail fast.
