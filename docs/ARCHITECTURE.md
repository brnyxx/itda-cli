# Architecture

itda is a Rust workspace that builds one binary, `itda`. This page explains how
a call flows through it, what it keeps on disk, and why the crates are split
the way they are.

## Layers

Each crate depends only on crates in a lower layer. Cargo rejects cycles. CI
runs `cargo xtask check-layers` to check the direction.

| Layer | Crate | Responsibility | I/O |
|---|---|---|---|
| 0 | `itda-types` | Error contract and exit codes, URL checks with the Public Suffix List, redaction, file helpers | files |
| 1 | `itda-html` | Script-free HTML and embedded-JSON extraction; what a page shows about itself (`evidence`) for drafting; the routes a page's docs and scripts name (`docs`); the shape of a page or an answer without its values (`shape`) | none |
| 1 | `itda-store` | One SQLite connection on its own thread, with per-component migrations | SQLite |
| 2 | `itda-kb` | The knowledge base per site: pages and actions, how they relate, every try with its verdict, notes, file hashes. Shapes only, never a value. The gap rules and the pass rates. Nothing that sends a request reads it | SQLite (through `itda-store`) |
| 2 | `itda-spec` | Definition format, request building, response verification | none |
| 3 | `itda-policy` | Read/write classification, definition safety checks, single-use approvals | SQLite |
| 4 | `itda-browser` | Sign-in and recording; isolated read drivers, guarded browser egress, authenticated worker and bounded session pool | browser, loopback IPC |
| 5 | `itda-net` | Bounded HTTP and adaptive reads, contract-aware strategies, shared pacing and cooldowns, private-address guard | network, SQLite |
| 6 | `itda-vault` | Accounts and secrets: OS keyring, environment variables, or memory | keyring, SQLite |
| 7 | `itda-runtime` | Runs an action end to end; approvals for changes; audit log | files |
| 8 | `itda-prep` | Drafts definitions from OpenAPI documents, recordings and fetched pages, and holds an agent's draft to the evidence (`ground`). Does no I/O; the CLI fetches | none |
| 9 | `itda-setup` | Steps for `itda setup`, `login` and `check`: probe, draft, try every read, keep what works; installed sites; the preparation loop an agent runs (`prepare`), which ends in the same install. Questions use a trait and progress uses a callback, so the CLI and MCP server share them | network, files, browser |
| 10 | `itda-mcp` | MCP server over stdio; with a setup context, `itda_setup`, `itda_login`, `itda_check`, `itda_sites`, `itda_manage`, `itda_shape` and `itda_prepare` | stdio |
| 11 | `apps/cli` | The `itda` command; the only place that picks paths and settings | all |

`itda-spec` never does I/O. Response checks take a `ResponseLike` trait, so
testing and reusing the format needs no network stack.

## One call

`itda call sandbox.list-notices --q 점검` and the MCP tool
`sandbox.list-notices` take the same path:

1. **Resolve.** The CLI loads `specs/sandbox/spec.yaml`. `itda-policy`
   validates it and refuses unsafe definitions with named problems.
2. **Check inputs.** Every input must be declared; itda checks types and
   required inputs as it builds the request.
3. **Classify.** `operation_effect` decides read or change from the method,
   name, path, inputs and any GraphQL document.
4. **Approve (changes only).** The first call stores the exact request (site,
   action, account, inputs, definition hash, credential fingerprint) and returns
   `approval_required`. After `itda approve`, the next call with that id must
   match the digest; itda uses up the approval before sending.
5. **Credential.** The vault supplies the account's secret. itda attaches a
   header credential only when the destination has the credential's origin.
6. **Send.** `itda-net` resolves DNS and refuses private addresses unless
   allowed. It waits for the site's pacing slot, then sends with size and
   time limits. In direct-HTTP mode every request carries the
   `itda/<version>` user agent; in adaptive mode every request, a change
   included, carries the installed browser's, so the direct requests and the
   browser look the same to the site. When a read is answered with a challenge, or the answer lacks the
   fields the definition promises, itda tries it again inside the browser (a
   fetch, then for a `GET` a rendered page) and remembers which way worked.
   Changes never retry or follow redirects. When a change's answer is lost and
   its definition names a confirming read (`confirm`), itda runs that read once
   and reports `landed`, `not_found` or `unknown` next to
   `action_outcome_unknown`; it never resends.
7. **Classify the answer.** A challenge the browser did not get past fails that
   read, not the site. In direct-HTTP mode a block or challenge pauses the
   site until `itda resume`. A 401 on a signed-in request pauses that account
   either way. A rate limit sets a cooldown every transport respects; it is not
   a reason to try the browser.
8. **Verify and shape.** The declared response (JSON pointers, HTML fields)
   must match and the data must extract, or the call fails with
   `response_mismatch`; in adaptive mode a mismatch is first a reason to try
   the next way, and only an answer that passed is kept and its way remembered.
   An error the site itself reports (a GraphQL `errors` list) ends the call
   and is not retried. itda then projects the data through a model and
   removes the secret.
9. **Record.** The audit log gets a record; a change got one before step 6.
   The knowledge base gets one line: the verdict, the status, the time it
   took and the keys and types of the answer, never an input or a value.
10. **Return.** The CLI prints the data or JSON envelope. MCP marks it
    untrusted, cuts it to 24 KB and returns it as `structuredContent`.

## What is stored

Definitions and application state are in the data directory (`ITDA_HOME` or the
platform's data folder). `itda doctor` prints it. Windows browser workers use
the user's local application-data directory instead.

| Path | Contents |
|---|---|
| `specs/<site>/spec.yaml` | Installed definitions |
| `itda.db` | Pacing, pauses, transport strategies, approvals, account names and which secrets exist (`vault_account`, `vault_secret`). No secret values. |
| `itda.db`, `kb_*` tables | The knowledge base: pages and actions (`kb_node`) with their shapes (selectors and counts, field names, keys and types), how they relate (`kb_edge`), the last 200 tries of each action with stage, verdict, HTTP status and time (`kb_attempt`), notes with a full-text index (`kb_finding`), the hashes of the definition and the last verdict (`kb_artifact`), and the gaps the rules found (`kb_gap`). No answer, input, cookie, header value, record or form value. `itda remove` deletes a site's rows. |
| `audit.jsonl` | Hash-chained audit records: names, outcomes, input digests. No input values. |
| OS keyring, service `itda` | Secret values, one entry per site, account and name |
| `browser-runtime/<namespace>/` | The browser worker's secret, its lock files, the installed browser's user agent, and the temporary browser profiles while they are in use. One directory per itda build; a directory of a build that is gone is removed once nothing has touched it for a day and no worker holds its lock. On Windows this lives under the user's local application data. |

## Browser worker

A read that needs the browser goes to a worker process; direct HTTP needs no
worker, and the first read that needs one starts it. One worker belongs to one
data directory, itda executable, browser and private-address setting. It keeps
up to four browser sessions, one per site and account, closing the least
recently used when a fifth is needed, and exits after five idle minutes. Reads
for the same site and account queue behind each other inside the worker; the
turn-taking between processes and the pacing per site stay in SQLite, as for
direct HTTP. A changed credential gives the session a new generation, so the
old cookies are not reused; a changed query does not.

The CLI and the worker talk over the loopback address in itda's own message
format, not HTTP. Before any credential is sent, both sides prove they hold a
secret kept in a file only this user can read: each signs the other's fresh
nonce together with the generation, protocol version, direction and the hash of
the itda executable. Messages carry a bounded JSON header and the raw bytes.
A reset ends every queued generation; shutdown waits for the reads in flight
and gives the ownership lock back. Lock files are never deleted to take over.

Everything the browser loads goes through itda's own SOCKS proxy, which
resolves each name and connects only to addresses the private-address rule
allows. Frames and workers the page creates are paused and set up the same way
before they run. Other sites get neither the account's credential headers nor
its referrer. A resource `POST` passes only when the policy crate reads it as
a GraphQL query. A `Retry-After` from one resource cools that resource's site
down in the shared gate without failing the page that loaded it.

Each browser is started through a small supervisor process whose standard
input the worker holds. When that pipe closes, the supervisor kills the
browser, waits for it and removes its private profile, even before the browser
has written its DevTools port. Profiles the worker registered are also cleaned
up at the next start, once the browser shows (through its command line) that
the profile is its own. While a read's outcome is unknown the account's turn is
kept; once the worker is known to have died, the read gets one more try.

## Decisions

- **Definitions over browsing.** Browser agents repeat the same work on every
  task (finding the button, reading the page). Here it happens once, when the
  definition is written and reviewed. Calls are cheap, typed and checkable.
- **Changes need a person.** No code path sends a change without the user's
  approval for that exact request. No MCP tool grants approval.
- **Adaptive transport.** Bounded browser recovery is allowed for reads,
  with account isolation, shared pacing and no write replay. See
  [HONEST_CLIENT.md](HONEST_CLIENT.md).
- **Website text is data.** itda marks results untrusted for agents; an agent
  gets instructions only from itda itself.
- **Local first.** No server, account or telemetry. Secrets stay in the OS
  keyring; a browser session a read recovers in lives in a private profile on
  this computer and is removed when it closes.
- **The knowledge base informs; it does not decide.** It tells a person or an
  agent what was tried, what failed and what needs an input. No transport,
  header, retry or account choice reads it: the runtime gets a handle that
  can only write a try, and a test makes the same requests with it empty and
  full.

## Roadmap

| Next | What it adds |
|---|---|
| Desktop approvals | Approve through a desktop prompt instead of a terminal or MCP elicitation |
| Signed site packs | Share and install reviewed definitions with hash-locked, signed packs |

## Provenance

Parts of the foundation were ported from
[Hycli](https://github.com/Hybirdss/Hycli) (Apache-2.0). Each ported file says
so in its header; [NOTICE](https://github.com/brnyxx/itda-cli/blob/main/NOTICE)
lists them. The main changes from the original:

| Here | Hycli source | Change |
|---|---|---|
| `types/error.rs` | `src/apperr.rs` | Product names removed from remedies; SQLite conversion behind a feature |
| `types/url.rs` | URL helpers in `src/net.rs`, `src/runtime.rs` | Same-site checks use the Public Suffix List instead of a fixed list |
| `html` | parts of `src/html.rs` | Only what running and verifying need |
| `spec` | `src/spec.rs` | `Effect` moved here; `ResponseLike` decouples it from the network |
| `policy` | `src/policy.rs`, `src/session_recipe.rs` | Logic kept; pacing moved to `net`. Approvals must be granted before use |
| `net` | `src/net.rs`, `src/defend.rs` | Data-contract-validated recovery, persistent route learning and shared cooldowns; an exhausted adaptive read does not pause the site |
