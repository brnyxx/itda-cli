<p align="center"><img src="assets/hero.png" alt="itda: a hand passes a small website card to an AI agent along one thread. Just type an address after itda and your AI can use the site." width="100%"></p>

<p align="center"><strong>Just type an address after itda and your AI can use the site.</strong></p>

<p align="center"><b>English</b> · <a href="README.ko.md">한국어</a></p>

<p align="center">
  <a href="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml"><img src="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/brnyxx/itda-cli/releases"><img src="https://img.shields.io/github/v/release/brnyxx/itda-cli?include_prereleases&amp;label=release" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License: Apache-2.0"></a>
  <a href="https://brnyxx.github.io/itda-cli/docs/">Docs</a>
</p>

Give itda one address. It learns what the site can do, tries it, and hands
your AI agent a short list of actions in plain words. Reads just work.
Nothing changes on the site until you say yes. It runs on your own computer,
and it works on the old intranet behind a login too.

<p align="center"><img src="assets/setup.gif" alt="A first run: itda asks for an address, news.ycombinator.com is pasted, it finds a list of 30 records, a form and a feed, tries both reads, saves the site and connects Claude Desktop after one Enter" width="860"></p>

<details>
<summary>The same run as text</summary>

```console
$ itda news.ycombinator.com
1/5  Looking at news.ycombinator.com...
    The page opens without signing in.
    Found: a list of 30 records, a form, a feed.
2/5  What agents will be able to do:
    read   home         List what Hacker News shows  (inputs: p)
    read   feed         Read the RSS feed
3/5  Trying every read once...
    ✓ home         30 records
    ✓ feed         ok
4/5  Saved as 'news-ycombinator'.
    Connect Claude Desktop? [Y/n]
5/5  Adding itda to Claude Desktop...
Added itda to Claude Desktop (~/Library/Application Support/Claude/claude_desktop_config.json).
Open Claude Desktop and itda will be there.

Done. Ask your agent for something on 'news-ycombinator', for example "use news-ycombinator to list what Hacker News shows".
```

</details>

Then ask your agent: *"use news-ycombinator to list what Hacker News shows"*.
A site behind a sign-in opens in your own browser instead; you sign in and use
it as usual while itda watches, and what you type never leaves your computer.

<p align="center"><img src="assets/flow.svg" alt="Record a website once in your browser, itda drafts commands from the recording, the agent calls them, and a change waits until a person approves it" width="860"></p>

> Status: early, and used against real sites. The runtime, CLI, MCP server,
> approvals (in a terminal or inside the agent app), audit log, drafting from
> OpenAPI documents, browser recordings or fetched pages, API keys, browser
> sign-in and CSRF preflight steps work today. Signed site packs are next. See
> [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Why

Agents that drive a browser re-read every page, spend thousands of tokens on
each screenshot or accessibility snapshot, click whatever the page tells them
to, and break silently when a layout changes. itda moves that work to an
install step:

| | Browser-driving agent | itda |
|---|---|---|
| What the agent sees | Whole pages, every step | One action's typed result, capped at 24 KB |
| A changed or signed-out page | Read as data | Detected from the declared response (`response_mismatch`, `auth_required`) |
| Changes on the site | Whatever the model decides | Single-use approval for exact inputs, granted by a person in a terminal or the agent app |
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

Then type `itda` and the address of a site you use. It finds the agent apps on
your computer and connects them after one Enter (`ITDA_LANG=ko` makes it speak
Korean). A bare `itda` asks for the address; `itda setup <address>` takes flags
for scripts.

Sites that run known software are recognised. When the page says it is a
GitLab, Jira, Confluence, Redmine, Discourse, MediaWiki, WordPress or Gitea
(also Forgejo), and one request to that software's own API confirms it, the
agent gets that software's real API from a ready-made template
([`sites/software/`](sites/software/)), signed in the way that software does
it, and what the page itself shows joins it. `itda setup --as jira` forces a
template; `--no-template` reads the page like any other.

A page drawn by scripts shows itda only a title, so itda opens your browser
on it for a few seconds, watches what the page loads by itself and drafts
from those calls; nothing is clicked or typed, and the window closes on its
own. Only a page that still shows nothing, or needs a sign-in, asks you to
use the site in your browser while itda watches.

Once an agent is connected, you can also just ask it: "set up
news.ycombinator.com for me". It calls `itda_setup`, the site is looked at,
drafted and tried the same way, and its actions appear as tools in the same
conversation (a scripted page opens briefly in a browser window on your
screen); when a read says to sign in, `itda_login` opens your own browser
and the agent only sees the account id.

Or skip the terminal and hand this to Claude Code, Cursor or Codex, with the
address of a site you use:

```text
Install itda and set up the-site-you-use.example for me.
1. Install it: curl -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
   (on Windows: irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex)
2. Run: itda setup https://the-site-you-use.example --client claude-code -y
   (use cursor, codex or claude-desktop if that is what you are)
3. Read https://github.com/brnyxx/itda-cli/blob/main/AGENTS.md, then use the site through itda.
   Reads just work. Anything that changes the site waits for my approval.
   If a read says to sign in, tell me to run: itda login <site> --browser
```

The pieces are commands of their own. For pages you can open without
signing in, give itda their addresses: tables, lists and repeated cards
become records, a page that is one thing (a package, an article, a product)
becomes a read of its headline, body and labelled values, a list whose links
share one shape gets a read of one record by its id,
search forms become reads, post forms become changes, feeds come along, a
GraphQL endpoint the page names is asked once what it offers, one read per
query field, and when the site publishes an API description, its reads join
the draft:

```sh
itda draft page https://example.com/board --install   # shows what it found on the page
itda call example.get-board --page 2 --json
itda check example                                     # does the page still match?
```

For a site behind a sign-in, record yourself using it. itda opens your own
browser in a fresh window and watches what the pages ask the server for
while you sign in, search and post a form; it never clicks or types:

```sh
itda record https://intranet.example/board --name board --install   # press Enter when done
                                                    # then say yes to keep the session or token it used
itda call board.list-notices --json                 # reads run right away
itda call board.create-notice --title "Server maintenance"   # changes wait for your approval
itda mcp install claude-desktop                     # or let your agent call them
```

The recording stays in memory and only the shape of each call reaches the
draft: no header, cookie or value you typed. A HAR file saved from the
browser's developer tools works the same way with `itda draft har`.

Public APIs need no recording. These ready-made sites from
[sites/](sites/README.md) are built in and need no account:

<p><a href="sites/github.yaml"><img src="assets/brands/github.svg" width="36" height="36" alt="GitHub"></a> <a href="sites/npm.yaml"><img src="assets/brands/npm.svg" width="36" height="36" alt="npm"></a> <a href="sites/crates-io.yaml"><img src="assets/brands/crates-io.svg" width="36" height="36" alt="crates.io"></a> <a href="sites/pypi.yaml"><img src="assets/brands/pypi.svg" width="36" height="36" alt="PyPI"></a></p>

```sh
itda add github      # or npm, crates-io, pypi
itda call github.repo --owner rust-lang --repo rust
```

Or try the bundled sandbox, a fake intranet that only listens on
127.0.0.1: follow [examples/sandbox](examples/sandbox/README.md). A site
definition is one YAML file; `itda add spec.yaml` installs it after showing
where it sends requests and which actions change the site, `itda doctor`
prints where the data directory is, and
[docs/SPEC.md](docs/SPEC.md) explains how to write one.

```sh
itda setup <url>                         # page or recording -> named actions -> tried -> agent, with questions
itda draft page <url>... [--install]     # draft a site from the pages you use
itda record <url> [--install]            # or from using it in your browser, behind a sign-in
itda draft har <file> / openapi <file>   # or from a saved recording or an API description
itda add spec.yaml / remove <site>       # install or remove a site (also a URL or github)
itda ls                                  # installed sites and their actions
itda describe <site>                     # their inputs and a command to copy each
itda call <site>.<action> --input value  # run one; --json for agents
echo "$KEY" | itda login <site>          # store an API key in the OS keyring
itda login <site> --browser              # or sign in in your own browser, once (sealed under the data directory)
itda accounts / logout <site>            # see or remove saved sign-ins
itda approvals / approve <id> / reject <id>
itda audit [--verify]                    # what agents did
itda check [<site>|--all]                # does every read still match the site?
itda refresh <site>                      # a site changed: draft it again, keep what you decided
itda resume <site>                       # after a site refused or a key expired
itda feedback                            # report a problem: opens a new issue, filled in
```

## Use it from an agent

```sh
itda mcp install claude-desktop   # or cursor, vscode, codex
itda mcp install claude-code      # prints the `claude mcp add` command
# add --approve-in-client to approve changes in the client's own form
# itda mcp --site <name> serves one site only, for a project's own configuration
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
| 14 | the site refused this one request as not allowed; signing in again will not help |
| 20 | needs approval, or the user has not answered yet (or changes are off) |
| 21 | approval used, rejected, expired or withdrawn because the inputs changed |
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
| 9 | `itda-setup` | The setup, sign-in and check steps, for the CLI and the MCP server alike |
| 10 | `itda-mcp` | MCP server over stdio |
| 11 | `apps/cli` | The `itda` command |

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
