# Changelog

All notable changes to itda are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). Until 1.0, a minor version may
change the definition format or the CLI.

## [Unreleased]

### Added

- `itda record <url>` opens your own browser in a fresh window and records
  what the pages ask the server for while you use the site, then drafts a
  definition the same way `itda draft har` does. The recording stays in
  memory; `--save` keeps it as a HAR file. itda never clicks, types or
  navigates in the window. `ITDA_DEBUG_CDP=1` prints the browser messages.
- `itda draft page <url>...` drafts a site definition from the pages you
  use, fetched with the honest client: a table or a list of records becomes
  `html` fields, data a page embeds for its scripts becomes `ssr_json`, a
  search form becomes a read, a post form becomes a change with its token
  read through a preflight step, page links become a `page` input, and RSS or
  Atom feeds come along. Every read gets a response check, so `itda check`
  notices when a page changes. Lists are found by their shape, not only by
  known class names: any run of three or more siblings outside navigation
  that each hold a link at the same place, with the title and link read
  from that link's own path.
- `itda draft har` also handles a token sent as a header, such as
  `X-CSRF-Token`: a preflight step reads it from the page's
  `<meta name="csrf-token">` tag before each call.
- `itda add` also takes an https URL, or the name of a ready-made site
  (`itda add github`); the four definitions in `sites/` are built in.

## [0.3.0] - 2026-10-20

Install ready-made sites with one command, and work with old intranet forms
that need a fresh security token on every change.

### Added

- Preflight steps: an action can read a page just before it runs and pass
  a fresh value, such as a CSRF token, into its headers, query or form as
  `{{step.<step>.<name>}}`. Agents never see or set those values.
- The sandbox has a write form with a one-time CSRF token
  (`sandbox.write-notice`) to try preflight steps on.
- `itda draft har` turns a form token such as `_csrf` into a preflight step
  that reads it from the form page recorded before the post.
- `itda add <file>` installs a site definition (or `-` for stdin) and shows
  where it sends requests, which actions change the site and how it signs
  in; `itda remove <site>` takes it away.
- Ready-made site definitions in `sites/`: GitHub (public reads without an
  account, issues and comments with a token), npm, crates.io and PyPI.

### Changed

- A `model` now applies to each record when the answer is a JSON list,
  instead of returning an empty object.

## [0.2.0] - 2026-10-20

Sign in through your own browser, approve changes inside the agent app, and
check that sites still answer the way their definitions say.

### Added

- `itda accounts` lists signed-in accounts and the names of their secrets;
  `itda logout <site>` removes an account and its saved secrets.
- `itda check [<site>]` runs every read that needs no input and reports which
  ones no longer match the site, with exit code 11 when a site changed.
- Browser sign-in for sites without API keys: `itda login <site> --browser`
  opens your own browser in a fresh profile, keeps only that site's session
  cookies in the system keyring, and `kind: browser` actions send them only to
  the same site.
- Approve changes inside the MCP client: with `itda mcp --approve-in-client`
  (or `itda mcp install <client> --approve-in-client`), clients that show
  elicitation forms ask you to approve the exact request, and declining
  returns `approval_rejected`.

## [0.1.0] - 2026-10-19

The first release: reads and approved changes on plain HTTP sites with API
keys, from the command line or over MCP, and definitions drafted from OpenAPI documents or browser recordings.

### Added

- `itda` command: `ls`, `call`, `login`, `resume`, `doctor`, with a stable
  exit code contract for scripts and agents (`2` input, `10` sign in, `11`
  page changed, `12` blocked, `13` rate limited, `20`–`22` approvals and
  changes, `30` network).
- MCP server (`itda mcp`): one tool per action up to 30 actions, three stable
  catalog tools (`itda_search`, `itda_describe`, `itda_call`) above that.
  Results are marked untrusted and cut to 24 KB.
- `itda draft openapi`: draft a site definition from an OpenAPI 3 or
  Swagger 2.0 document, with response checks, header auth and a list of what
  could not be translated. `--install` puts it in the data directory.
- `itda draft har`: draft a site definition from a browser recording of you
  using the site. Only the shape of each call is kept; recorded tokens,
  cookies and values never reach the draft.
- `itda mcp install` for Claude Desktop, Cursor, VS Code and Codex, and the
  `claude mcp add` command for Claude Code. Existing settings are kept and
  backed up.
- Approvals for changes: a change returns an approval id, the user reviews
  the exact request with `itda approve` in a terminal, and the change runs
  once with exactly those inputs. `itda approvals` and `itda reject` list and
  refuse them.
- Hash-chained audit log of every call, approval and change, with
  `itda audit` and `itda audit --verify`. Input values and credentials are
  never written to it.
- Credential vault on the OS keyring, or `ITDA_SECRET__` environment variables
  for CI and containers. Agents only ever see account ids.
- Site definitions (`spec.yaml`): JSON and server-rendered HTML responses,
  declared response checks, models, and safety validation with named
  problems.
- Network layer: honest `itda/<version>` user agent, per-site pacing and
  budgets, pauses on blocks and expired sign-ins, private-address guard,
  credentials bound to their origin.
- Sandbox example site for trying everything locally.

[Unreleased]: https://github.com/brnyxx/itda-cli/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/brnyxx/itda-cli/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/brnyxx/itda-cli/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/brnyxx/itda-cli/releases/tag/v0.1.0
