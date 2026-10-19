# Architecture

itda is a Rust workspace that builds one binary, `itda`. This page explains how
a call flows through it, what it keeps on disk, and why the crates are split
the way they are.

## Layers

Each crate depends only on crates in a lower layer. Cargo rejects cycles;
`cargo xtask check-layers` checks the direction, and CI runs it.

| Layer | Crate | Responsibility | I/O |
|---|---|---|---|
| 0 | `itda-types` | Error contract and exit codes, URL checks with the Public Suffix List, redaction, file helpers | files |
| 1 | `itda-html` | Script-free HTML and embedded-JSON extraction | none |
| 1 | `itda-store` | One SQLite connection on its own thread, with per-component migrations | SQLite |
| 2 | `itda-spec` | Definition format, request building, response verification | none |
| 3 | `itda-policy` | Read/write classification, definition safety checks, single-use approvals | SQLite |
| 4 | `itda-net` | Bounded HTTP, pacing and budgets, block detection and pauses, private-address guard | network, SQLite |
| 5 | `itda-vault` | Accounts and secrets: OS keyring, environment variables, or memory | keyring, SQLite |
| 7 | `itda-runtime` | Runs an action end to end; approvals for changes; audit log | files |
| 9 | `itda-mcp` | MCP server over stdio | stdio |
| 10 | `apps/cli` | The `itda` command; the only place that picks paths and settings | all |

Layers 6 (`browser`) and 8 (`prep`) are reserved for browser sign-in and for
drafting definitions; see the roadmap below.

`itda-spec` never does I/O: response checks take a `ResponseLike` trait, so the
format can be tested and reused without a network stack.

## One call

`itda call sandbox.list-notices --q 점검` and the MCP tool
`sandbox.list-notices` take the same path:

1. **Resolve.** The CLI loads `specs/sandbox/spec.yaml`. `itda-policy`
   validates it and refuses unsafe definitions with named problems.
2. **Check inputs.** Every input must be declared; types and required inputs
   are checked while the request is built.
3. **Classify.** `operation_effect` decides read or change from the method,
   name, path, inputs and any GraphQL document.
4. **Approve (changes only).** The first call stores the exact request
   (site, action, account, inputs, definition hash, credential fingerprint)
   and returns `approval_required`. After `itda approve`, the next call with
   that id must match the digest; the approval is used up before sending.
5. **Credential.** The vault supplies the account's secret. A header credential
   is attached only when the destination has the credential's origin.
6. **Send.** `itda-net` resolves DNS, refuses private addresses unless allowed,
   waits for the site's pacing slot, sends with the honest `itda/<version>`
   user agent and bounded size and time. Reads may retry once; changes never
   retry or follow redirects.
7. **Classify the answer.** A block, challenge or rate limit pauses the site;
   a 401 on a signed-in request pauses that account. Both stay paused until
   `itda resume`.
8. **Verify and shape.** The declared response (JSON pointers, HTML fields)
   must match, or the call fails with `response_mismatch`. The data is then
   extracted, projected through a model, and scrubbed of the secret.
9. **Record.** The audit log gets a record; a change got one before step 6.
10. **Return.** The CLI prints the data or the JSON envelope; MCP returns it as
    `structuredContent`, marked untrusted and cut to 24 KB.

## What is stored

Everything lives in the data directory (`ITDA_HOME`, or the platform's data
folder; `itda doctor` prints it).

| Path | Contents |
|---|---|
| `specs/<site>/spec.yaml` | Installed definitions |
| `itda.db` | Pacing and pauses (`request_gate`, `request_event`), approvals, account names and which secrets exist (`vault_account`, `vault_secret`). No secret values. |
| `audit.jsonl` | Hash-chained audit records: names, outcomes, input digests. No input values. |
| OS keyring, service `itda` | Secret values, one entry per site, account and name |

## Decisions

- **Definitions over browsing.** Work that browser agents repeat on every
  task (finding the button, reading the page) happens once, when the
  definition is written and reviewed. Calls are cheap, typed and checkable.
- **Changes need a person.** No code path sends a change without an approval
  the user granted for that exact request, and no MCP tool grants one.
- **Honest client.** itda identifies itself, never imitates a browser, and
  stops when a site says no. See [HONEST_CLIENT.md](HONEST_CLIENT.md).
- **Website text is data.** Results are marked untrusted for agents; the only
  instructions an agent gets come from itda itself.
- **Local first.** No server, account or telemetry. Secrets stay in the OS
  keyring.

## Roadmap

| Next | What it adds |
|---|---|
| Definition drafting (`prep`) | Draft `spec.yaml` from an OpenAPI document or a recorded browser session (HAR), then review and verify it |
| Browser sign-in (`browser`) | Sites without API keys: reuse a session the user signed in to, only where the definition says so |
| Approvals in clients | Approve through MCP elicitation or a desktop prompt instead of a terminal |
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
| `policy` | `src/policy.rs`, `src/session_recipe.rs` | Logic kept; pacing moved to `net`; approvals must be granted before use |
| `net` | `src/net.rs`, `src/defend.rs` | No browser imitation, no fallback to a browser when blocked; blocks pause the site |
