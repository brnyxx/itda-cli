<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/wordmark-dark.svg">
    <img src="assets/wordmark-light.svg" alt="itda" width="240">
  </picture>
</p>

<p align="center"><strong>Don't drive the browser. Compile the site.</strong></p>

<p align="center">
  <a href="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml"><img src="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/brnyxx/itda-cli/releases"><img src="https://img.shields.io/github/v/release/brnyxx/itda-cli?include_prereleases&amp;label=release" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License: Apache-2.0"></a>
  <a href="https://brnyxx.github.io/itda-cli/docs/">Docs</a>
</p>

itda turns a website into a small set of verified, typed actions that an AI
agent can call from the command line or over MCP. It runs on your own
machine, reads by default, and never changes anything on a site until you
approve that exact change.

<p align="center"><img src="assets/hero.png" alt="itda drafts actions from a browser recording, reads freely, and waits for a person to approve a change" width="860"></p>

> Status: early. The runtime, CLI, MCP server, approvals, audit log and
> drafting from OpenAPI documents or browser recordings work today against
> plain HTTP sites with API keys. Browser sign-in and signed site packs are
> next. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Why

Agents that drive a browser re-read every page, spend thousands of tokens on
each screenshot or accessibility snapshot, click whatever the page tells them
to, and break silently when a layout changes. itda moves that work to an
install step:

| | Browser-driving agent | itda |
|---|---|---|
| What the agent sees | Whole pages, every step | One action's typed result, capped at 24 KB |
| A changed or signed-out page | Read as data | Detected from the declared response (`response_mismatch`, `auth_required`) |
| Changes on the site | Whatever the model decides | Single-use approval for exact inputs, granted by a person in a terminal |
| Text on the page saying "ignore previous instructions" | In the prompt | Marked untrusted data, never a tool |
| Credentials | Typed into the page by the model | In the OS keyring; the agent sees account ids only |
| Being blocked | Retry, rotate, imitate a browser | Stop, tell you, wait for you to resume |
| What happened | Chat history | Hash-chained audit log you can verify |

## Quick start

```sh
# macOS and Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
```

```powershell
# Windows
powershell -ExecutionPolicy Bypass -c "irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex"
```

Or download an archive from [Releases](https://github.com/brnyxx/itda-cli/releases),
or build from source with
`cargo install --git https://github.com/brnyxx/itda-cli itda-cli`.

Try it on the bundled sandbox, a fake intranet that only listens on
127.0.0.1: follow [examples/sandbox](examples/sandbox/README.md). A site
definition is one YAML file at `<data dir>/specs/<site>/spec.yaml`;
`itda doctor` prints where the data directory is, and
[docs/SPEC.md](docs/SPEC.md) explains how to write one.

```sh
itda ls                                  # installed sites and their actions
itda call <site>.<action> --input value  # run one; --json for agents
echo "$KEY" | itda login <site>          # store an API key in the OS keyring
itda login <site> --browser              # or sign in in your own browser, once
itda accounts / logout <site>            # see or remove saved sign-ins
itda approvals / approve <id> / reject <id>
itda audit [--verify]                    # what agents did
itda resume <site>                       # after a site refused or a key expired
```

## Use it from an agent

```sh
itda mcp install claude-desktop   # or cursor, vscode, codex
itda mcp install claude-code      # prints the `claude mcp add` command
# add --approve-in-client to approve changes in the client's own form
```

The installer merges into the client's config, keeps your other servers and
backs the old file up as `.bak`; `--dry-run` shows the change first. `itda mcp`
serves every installed site:

- Up to 30 actions appear as one tool each (`sandbox.list-notices`), with input
  schemas and read-only, destructive and idempotent hints.
- Above that, three stable tools (`itda_search`, `itda_describe`, `itda_call`)
  keep the tool list short and prompt caches valid. `--mode` overrides this.
- Results are the same JSON envelope as `itda call --json`, marked untrusted.
- A change returns `approval_required` with an id. There is no tool that
  approves; the agent has to ask you.

Agents that only have a shell can call `itda call … --json` and branch on the
exit code:

| Exit | Meaning |
|---|---|
| 0 | ok |
| 2 | bad input or unknown action |
| 10 | sign in again (`itda login`) |
| 11 | the page no longer matches its definition |
| 12 | the site refused; paused until `itda resume` |
| 13 | rate limited |
| 20 | needs approval (or changes are off) |
| 21 | approval used, rejected, expired or for other inputs |
| 22 | a change was sent but its outcome is unknown; check the site |
| 30 | network error |

## Principles

- **Local first.** Definitions run on your device. Credentials never leave it.
- **Read-only by default.** An action is a read only when its method, name,
  path, inputs and GraphQL document all agree. Everything else is a change.
- **People approve changes.** Each approval covers one request with exact
  inputs, account, definition and credential, works once and expires.
- **Blocks are reported, not bypassed.** itda identifies itself, never imitates
  a browser, and pauses a site that refuses it. See
  [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md).
- **Verified actions.** A definition declares what a correct response looks
  like, so "signed out" and "the site changed" are errors, not data.

## Layout

Crates depend only on crates below them; `cargo xtask check-layers` enforces it.

| Layer | Crate | Purpose |
|---|---|---|
| 0 | `itda-types` | Error contract, URL checks (Public Suffix List), redaction |
| 1 | `itda-html`, `itda-store` | Script-free HTML extraction; the local SQLite store |
| 2 | `itda-spec` | Definition format, request building, response verification |
| 3 | `itda-policy` | Read/write classification, definition safety checks, approvals |
| 4 | `itda-net` | Bounded HTTP, pacing, block detection, private-address guard |
| 5 | `itda-vault` | Accounts and secrets in the OS keyring or environment |
| 7 | `itda-runtime` | Runs actions: credentials, approvals, shaping, audit log |
| 9 | `itda-mcp` | MCP server over stdio |
| 10 | `apps/cli` | The `itda` command |

## Develop

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo xtask check-layers
```

Rust 1.98 (edition 2024) is pinned in `rust-toolchain.toml`. Tests only talk
to local servers they start themselves. Commit and PR rules are in
[CONTRIBUTING.md](CONTRIBUTING.md).

## Security

[docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) lists what itda defends against
and what it does not. Report vulnerabilities as described in
[SECURITY.md](SECURITY.md).

## Credits and license

Licensed under [Apache-2.0](LICENSE). Parts of this project are derived from
[Hycli](https://github.com/Hybirdss/Hycli) (Apache-2.0); see [NOTICE](NOTICE).
Each derived file says so in its header and describes what changed. This project
is not affiliated with Hycli and does not use its name or marks.
