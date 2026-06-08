---
name: send
description: Use when the user wants to fire a single request from a collection or turn one into a curl — "send the login request", "hit this endpoint", "show me the curl for X", "download the report from this request", "open the HTML response". Sends one request and prints the response, or emits the equivalent curl command. NOT for running whole collections (use the run skill) or writing assertions (test skill).
---

# golden send / curl

Fire one request from a collection, or print its curl — fast, no full run.

## Send a request

```bash
golden send <COLLECTION> <REQUEST> [--env <name|path>]
```

`<COLLECTION>` = a collection's `info.name` or a file path; `<REQUEST>` = the request name within it. Useful flags:

| Flag | Effect |
|------|--------|
| `--env <name\|path>` | Apply an environment (`{{vars}}`). |
| `--output <file>` | Write the response body to a file instead of stdout. |
| `--max-size <bytes>` | Abort + remove the partial file if the download exceeds this. |
| `--force` | Overwrite the output file without confirming. |
| `--cookies` | Print `Set-Cookie` headers after the response. |
| `--open` | If the body is HTML, write it to a temp file and open it in the browser. |
| `--insecure` | Skip TLS verification. |
| `--timeout <ms>` | Per-request timeout. |

```bash
golden send "My API" "Login" --env .env.local
golden send "My API" "Download report" --output report.pdf --max-size 10485760
```

## Generate a curl

```bash
golden curl <COLLECTION> <REQUEST> [flags]
```

Emits the equivalent curl to paste anywhere. Flags map to curl: `--mask` (hide Authorization/Cookie/API-key values — use when sharing), `-L/--follow`, `-i/--include`, `-s/--silent`, `-k/--insecure`, `-f/--fail`, `--compressed`, `-w/--timing`, `--download` (`-O -J`).

```bash
golden curl "My API" "Login" --mask        # safe to paste in a ticket
golden curl "My API" "Login" -L -i          # follow redirects, include headers
```

## Notes (verified)

- No collection/request name? `golden list` (or the `golden` skill's discovery rules) shows what's available.
- **Duplicate request names within a collection → the first match wins, silently.** The sample has "Get All Users" in two folders; `send`/`curl` hit the first. Use a unique request name to be sure.
- `<COLLECTION>` takes a collection's `info.name` OR a file path; `<REQUEST>` is always the request name.
- Unknown collection or request → **exit 2** (so a typo fails CI rather than passing quietly).
- `--mask` before sharing any curl — redacts secret-bearing headers.
- Assert on the response → `test` skill. Run many requests → `run` skill.
