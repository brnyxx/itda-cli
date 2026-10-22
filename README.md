<p align="center"><img src="assets/hero.png" alt="itda: a hand passes a small website card to an AI agent along one thread. Just type an address after itda and your AI can use the site." width="100%"></p>

<p align="center"><strong>Just type an address after itda and your AI can use the site.</strong></p>

<p align="center"><b>English</b> | <a href="README.ko.md">한국어</a> | <a href="README.ja.md">日本語</a> | <a href="README.zh-Hans.md">简体中文</a> | <a href="README.zh-Hant.md">繁體中文</a> | <a href="README.es.md">Español</a> | <a href="README.fr.md">Français</a> | <a href="README.de.md">Deutsch</a> | <a href="README.pt-BR.md">Português (Brasil)</a> | <a href="README.vi.md">Tiếng Việt</a> | <a href="README.id.md">Bahasa Indonesia</a> | <a href="README.ru.md">Русский</a></p>

<p align="center">
  <a href="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml"><img src="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/brnyxx/itda-cli/releases"><img src="https://img.shields.io/github/v/release/brnyxx/itda-cli?include_prereleases&amp;label=release" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License: Apache-2.0"></a>
  <a href="https://brnyxx.github.io/itda-cli/docs/">Docs</a>
</p>

Give itda one address. It learns what the site can do and tries it. It gives
your AI agent a short list of actions in plain words. Reads just work. Nothing
changes on the site until you say yes. It runs on your own computer, and it
works on the old intranet behind a login too.

It is for the sites your company runs and the services you have an account
on, not the web at large.

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

Ask your agent: *"use news-ycombinator to list what Hacker News shows"*. A site
behind a sign-in form asks for your user name and password instead. itda fills
in the site's own form as itself and keeps the session on your computer. If that
is not enough (single sign-on, a one-time code), your own browser opens. You
sign in and use the site as usual while itda watches.

<p align="center"><img src="assets/flow.svg" alt="Record a website once in your browser; itda drafts commands from it. The agent calls them; a change waits for a person's approval." width="860"></p>

> Status: early, and used against real sites. The runtime, CLI, MCP server,
> approvals (in a terminal or inside the agent app), audit log, drafting from
> OpenAPI documents, browser recordings or fetched pages, API keys, browser
> sign-in and CSRF preflight steps work today. Signed site packs are next. See
> [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Why

Agents that drive a browser re-read every page and spend thousands of tokens on
each screenshot or accessibility snapshot. They click whatever the page tells
them to and break silently when a layout changes. itda moves that work to an
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

## What it can and cannot do

Every line here is true of the code today. `itda setup` says which case it met;
`itda draft page` or `itda record` say what they left out.

**Works today**

- Pages drawn on the server: tables, lists and repeated cards become records,
  and a page that is one thing becomes a read of its fields. Search forms become
  reads, post forms become changes, and feeds come along.
- Pages that embed their data for their scripts (`__NEXT_DATA__`,
  `ld+json`, an Apollo or preloaded state): one read per list in it.
- Pages drawn by scripts that call an API answering JSON, a React or Vue portal
  say: `itda setup` opens your browser on the page and watches what it loads by
  itself. It drafts from those calls. It does not click or type.
- Forms that want a CSRF token: a preflight step reads a fresh one from the
  form page right before each approved post.
- Pages that are a shell around frames: the content sits in an `<iframe>` on
  the same site, often behind its own sign-in. itda reads each frame as a page
  of the site and signs in first when the frame asks for it.
- Cookie sessions: a sign-in through the site's own form with your user name
  and password, or your own browser for single sign-on and one-time codes. A
  session carried in the address (`?sid=...`) works the same way.
- An OpenAPI or Swagger description the site publishes, and a GraphQL
  endpoint that answers introspection: one read per operation or query field.
- Sites that run GitLab, Jira, Confluence, Redmine, Discourse, MediaWiki,
  WordPress, Gitea or Forgejo: their own API, from a ready-made template.
- Anything you recorded yourself using, in your browser or as a HAR file.

**Works after you record it**

- A page that shows a plain fetch nothing and loads nothing by itself. It
  fetches only after you click, search or sign in. `itda record` watches you use
  it and drafts from the calls it saw.

**Does not work**

- WebSocket. itda speaks plain HTTP, and a recording keeps no socket traffic.
- APIs that sign every request (a signature or nonce computed per call). itda
  keeps a fixed token or API key in the keyring. It does not reproduce a value
  that changes with every call, so the site refuses the request.
- Pages assembled from HTML fragments (htmx and the like). Not by design: itda
  may still read a fragment containing a whole table, but not what the script
  does with it.
- Content drawn on a canvas or into an image. itda reads text in HTML, JSON
  and XML; it does not look at pixels.
- Sites behind a challenge or a bot manager (Cloudflare, Akamai, a captcha).
  itda identifies itself and never imitates a browser. It pauses the site until
  you resume it. See [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md).
- Anything your account cannot see. itda has no account of its own; it uses
  yours.

## When the site changes

`itda check <site>` runs every read needing no input and never sends a change.
It ends with one verdict: `ok`, `signed_out`, `site_changed`, `blocked` or
`unreachable`. The exit code follows it (11 for `site_changed`), so a scheduled
job can warn you before an agent runs into it. Over MCP, `itda_check` returns
the same.

When the verdict is `site_changed`, `itda refresh <site>` drafts the site again
from what it was set up from: its pages, the API description, the GraphQL
endpoint, or the kept shape of the recording, every value blanked. It shows what
would change per action and writes nothing until you say yes. itda keeps actions
you edited by hand and updates actions the new draft changed. It drops an action
the sources no longer draft only if its read also failed, and tries each new
action once, adding it if it answers. itda withdraws approvals waiting for the
site and checks the site again right after. A definition installed with
`itda add` has no sources and cannot be refreshed; edit it by hand.

That is the trade-off. A browser agent adapts on every run, and is slow and
nondeterministic for it. itda is one request and the same answer every time. It
breaks loudly when the site moves.

## Next to browser agents

Playwright MCP, browser-use and computer use drive a browser on every task. itda
does not replace them: a page you will use once, or a site from the list above
that itda does not cover, is theirs. itda is for the site an agent will call
again and again. Record it once in a browser, review the actions, then run them
over HTTP with a typed answer, a single-use approval for every change and a log.
Use both: a browser agent to explore, itda for the calls that run every day.

## Quick start

```sh
# macOS and Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
```

```powershell
# Windows
powershell -ExecutionPolicy Bypass -c "irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex"
```

Download an archive from
[Releases](https://github.com/brnyxx/itda-cli/releases), or build from source
with `cargo install --git https://github.com/brnyxx/itda-cli itda-cli`.

Claude Desktop, with no terminal: download the bundle for
[macOS (Apple silicon)](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-aarch64-apple-darwin.mcpb),
[macOS (Intel)](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-apple-darwin.mcpb) or
[Windows](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-pc-windows-msvc.mcpb) and open the file. Claude
Desktop installs itda with its tools, and "use the-site-you-use.example" sets
a site up from inside the conversation. Approving a change still happens in a
terminal with `itda approve`, so for changes install the command too.

Type `itda` and the address of a site you use. It finds agent apps on your
computer and connects them after one Enter. It speaks twelve languages: `LANG`
picks one and `ITDA_LANG` overrides it. A bare `itda` asks for the address;
`itda setup <address>` takes flags for scripts.

itda recognises sites that run known software. When the page says it is a
GitLab, Jira, Confluence, Redmine, Discourse, MediaWiki, WordPress or Gitea
(also Forgejo), and one request to that software's own API confirms it, the
agent gets that software's real API from a ready-made template
([`sites/software/`](sites/software/)), signed in the way that software does it.
What the page itself shows joins it. `itda setup --as jira` forces a template;
`--no-template` reads the page like any other.

A page drawn by scripts shows itda only a title, so itda opens your browser on
it for a few seconds. It watches what the page loads by itself and drafts from
those calls. Nothing is clicked or typed, and the window closes on its own. Only
a page that still shows nothing, or needs a sign-in, asks you to use the site in
your browser while itda watches.

Once an agent is connected, you can also just ask it: "set up
news.ycombinator.com for me". It calls `itda_setup`. itda looks at the site,
drafts and tries it the same way. Its actions appear as tools in the same
conversation. A scripted page opens briefly in a browser window on your screen.
When a read says to sign in, `itda_login` opens your own browser; the agent only
sees the account id.

Skip the terminal and hand this to Claude Code, Cursor or Codex, with the
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

The pieces are commands of their own. For pages you can open without signing in,
give itda their addresses. Tables, lists and repeated cards become records. A
page that is one thing (a package, an article, a product) becomes a read of its
headline, body and labelled values. A list whose links share one shape gets a
read of one record by its id. Search forms become reads; post forms become
changes. Feeds come along. A GraphQL endpoint the page names is asked once what
it offers, one read per query field. When the site publishes an API description,
its reads join the draft:

```sh
itda draft page https://example.com/board --install   # shows what it found on the page
itda call example.get-board --page 2 --json
itda check example                                     # does the page still match?
```

For a site behind a sign-in form, give `itda setup` the page and your user name.
It sends the site's own form once, as itself, with the password you type. It
keeps the session in the system keyring and drafts from the page as you see it
signed in. The definition remembers the form, so `itda login board` signs in
again when the session expires:

```sh
itda setup https://intranet.example/board --name board --user me   # asks for the password
itda call board.list-notices --json                 # reads run right away
itda call board.create-notice --title "Server maintenance"   # changes wait for your approval
itda login board --user me                          # when the session expires
```

For single sign-on or a one-time code, record yourself using the site instead.
itda opens your own browser in a fresh window. It watches what the pages ask the
server for while you sign in, search and post a form; it never clicks or types:

```sh
itda record https://intranet.example/board --name board --install   # press Enter when done
                                                    # then say yes to keep the session or token it used
itda mcp install claude-desktop                     # or let your agent call them
```

`examples/lab` runs Gitea, Redmine, WordPress, MediaWiki, a small legacy
intranet and three uglier cases behind their sign-ins: a board inside an iframe,
a table a script draws from an XHR, and a session carried in the URL. A test
suite walks each from setup to an approved change.

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

Try the bundled sandbox, a fake intranet that only listens on 127.0.0.1: follow
[examples/sandbox](examples/sandbox/README.md). A site definition is one YAML
file. `itda add spec.yaml` installs it after showing where it sends requests and
which actions change the site. `itda doctor` prints the data directory's
location. [docs/SPEC.md](docs/SPEC.md) explains how to write a definition.

```sh
itda setup <url>                         # page or recording -> named actions -> tried -> agent, with questions
itda draft page <url>... [--install]     # draft a site from the pages you use
itda record <url> [--install]            # or from using it in your browser, behind a sign-in
itda draft har <file> / openapi <file>   # or from a saved recording or an API description
itda add spec.yaml / remove <site>       # install or remove a site (also a URL or github)
itda ls                                  # installed sites and their actions
itda describe <site>                     # their inputs and a command to copy each
itda call <site>.<action> --input value  # run one; --json for agents
itda login <site> --user me              # sign in through the site's own form (password from stdin or a prompt)
echo "$KEY" | itda login <site> --secret api-key   # or store an API key in the OS keyring
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

The installer merges into the client's config and keeps your other servers. It
backs up the old file as `.bak`. `--dry-run` shows the change first. `itda mcp`
serves every installed site:

- Up to 30 actions appear as one tool each (`sandbox.list-notices`), with input
  schemas and read-only, destructive and idempotent hints.
- Above that, three stable tools (`itda_search`, `itda_describe`, `itda_call`)
  keep the tool list short and prompt caches valid. `--mode` overrides this.
- Results are the same JSON envelope as `itda call --json`, marked untrusted.
- A change returns `approval_required` with an id. No tool approves; the
  agent has to ask you.

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
| 22 | itda sent a change but its outcome is unknown; check the site |
| 30 | network error |

## Principles

- **Local first.** Definitions run on your device. Credentials never leave it.
- **Read-only by default.** An action is a read only when its method, name,
  path, inputs and GraphQL document all agree. Everything else is a change.
- **People approve changes.** Each approval covers one request with exact
  inputs, account, definition and credential. It works once and expires.
- **Blocks are reported, not bypassed.** itda identifies itself and never
  imitates a browser. It pauses a site that refuses it. See
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

`rust-toolchain.toml` pins Rust 1.98 (edition 2024). Tests only talk to local
servers they start themselves. Commit and PR rules are in
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
