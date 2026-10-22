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
| 1 | `itda-html` | Script-free HTML and embedded-JSON extraction; what a page shows about itself (`evidence`) for drafting | none |
| 1 | `itda-store` | One SQLite connection on its own thread, with per-component migrations | SQLite |
| 2 | `itda-spec` | Definition format, request building, response verification | none |
| 3 | `itda-policy` | Read/write classification, definition safety checks, single-use approvals | SQLite |
| 4 | `itda-net` | Bounded HTTP, pacing and budgets, block detection and pauses, private-address guard | network, SQLite |
| 5 | `itda-vault` | Accounts and secrets: OS keyring, environment variables, or memory | keyring, SQLite |
| 6 | `itda-browser` | Opens the user's browser in a fresh profile. Reads its cookies over DevTools for sign-in; watches page requests for `itda record` and the watched load of `itda setup` | browser |
| 7 | `itda-runtime` | Runs an action end to end; approvals for changes; audit log | files |
| 8 | `itda-prep` | Drafts definitions from OpenAPI documents, recordings and fetched pages. Does no I/O; the CLI fetches | none |
| 9 | `itda-setup` | Steps for `itda setup`, `login` and `check`: probe, draft, try every read, keep what works; installed sites. Questions use a trait and progress uses a callback, so the CLI and MCP server share them | network, files, browser |
| 10 | `itda-mcp` | MCP server over stdio; with a setup context, `itda_setup`, `itda_login`, `itda_check`, `itda_sites` and `itda_manage` | stdio |
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
   allowed. It waits for the site's pacing slot, then sends with the honest
   `itda/<version>` user agent and size and time limits. Reads may retry once;
   changes never retry or follow redirects.
7. **Classify the answer.** A block, challenge or rate limit pauses the site;
   a 401 on a signed-in request pauses that account. Both stay paused until
   `itda resume`.
8. **Verify and shape.** The declared response (JSON pointers, HTML fields) must
   match, or the call fails with `response_mismatch`. itda then extracts the
   data. It projects it through a model and removes the secret.
9. **Record.** The audit log gets a record; a change got one before step 6.
10. **Return.** The CLI prints the data or JSON envelope. MCP marks it
    untrusted, cuts it to 24 KB and returns it as `structuredContent`.

## What is stored

Everything is in the data directory (`ITDA_HOME` or the platform's data folder).
`itda doctor` prints it.

| Path | Contents |
|---|---|
| `specs/<site>/spec.yaml` | Installed definitions |
| `itda.db` | Pacing and pauses (`request_gate`, `request_event`), approvals, account names and which secrets exist (`vault_account`, `vault_secret`). No secret values. |
| `audit.jsonl` | Hash-chained audit records: names, outcomes, input digests. No input values. |
| OS keyring, service `itda` | Secret values, one entry per site, account and name |

## Decisions

- **Definitions over browsing.** Browser agents repeat the same work on every
  task (finding the button, reading the page). Here it happens once, when the
  definition is written and reviewed. Calls are cheap, typed and checkable.
- **Changes need a person.** No code path sends a change without the user's
  approval for that exact request. No MCP tool grants approval.
- **Honest client.** itda identifies itself and never imitates a browser. It
  stops when a site says no. See [HONEST_CLIENT.md](HONEST_CLIENT.md).
- **Website text is data.** itda marks results untrusted for agents; an agent
  gets instructions only from itda itself.
- **Local first.** No server, account or telemetry. Secrets stay in the OS
  keyring.

## Roadmap

| Next | What it adds |
|---|---|
| Desktop approvals | Approve through a desktop prompt instead of a terminal or MCP elicitation |
| Signed site packs | Share and install reviewed definitions with hash-locked, signed packs |
| Outcome check after a lost answer | A change names the read that shows it landed; after `action_outcome_unknown` itda runs that read before asking you to approve again |

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
| `net` | `src/net.rs`, `src/defend.rs` | No browser imitation or browser fallback when blocked. Blocks pause the site |
