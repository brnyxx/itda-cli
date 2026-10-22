# Changelog

All notable changes to itda are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). Until 1.0, a minor version may
change the definition format or the CLI.

## [Unreleased]

### Added

- `defaults.interval` in a site definition asks for more time between
  requests to that site (`2s`, `500ms`, up to 30 s). itda's own pace, one
  request at a time per site about 1.2 seconds apart, is the floor; a
  definition can only slow it down. A value that is not a time is refused
  by `itda add`.
- A site that answers a read with 429 or 503 and a `Retry-After` of ten
  seconds or less gets that wait, once, and the read is tried again. A longer
  wait stops the read with `site_rate_limited`, `itda check` reports the site
  as `blocked`, and the error, the check result (`retry_after_s`) and the MCP
  server instructions say how long the site asked for. A change is never sent
  twice.
- Requests queued in one process now take turns in order instead of polling
  the shared gate against each other.

### Changed

- The landing page has a living sky behind the whole page: liquid chrome
  with striped, colour-split reflections and a holographic sheen, a wire
  grid running to the horizon, and a field of points and threads that gather
  around the cursor, with a website on the left and an agent on the right
  (`site/sky.js`, three.js from cdnjs). It follows the scroll, draws one
  still frame under reduced motion, and leaves the plain glow without WebGL.
- The hero lost its badge and chips; the binary, platform and licence line
  moved to the footer.
- The landing page is laid out as a desktop. The hero stacks windows that
  can be dragged: the first run, the actions it found, the setup progress,
  an about box and a clock. Sections open with a numbered eyebrow and a
  centred headline, every box is a window with a title bar and a dither
  texture, and the risk comparison is two windows side by side, the
  browser-driving agent's lane next to itda's.
- The sky is now a living network: nodes spring home through a slow current,
  threads join neighbours, packets of light run node to node and along the
  handoff between the website and agent hubs. The cursor is a node too:
  nearby nodes gather and orbit it, its motion leaves a wake, threads run
  from it, and a click sends a ring through the field. Node count follows
  the viewport, with a lower cap on touch screens.
- The landing page's components now speak the same language as the sky:
  smoked-glass panels with a chrome hairline and faint scanlines, chrome
  headlines with a holographic accent, glossy pill buttons, Aqua-style
  window lights on the terminal, and chrome coins for the step numbers.
- `itda <address>` sets that site up, and `itda github` (or npm, crates-io,
  pypi) adds the ready-made site and connects the agent apps found on this
  computer; the README's one line is the one line you type. The tagline and
  the landing page say so.

### Added

- `AGENTS.md` and `skills/itda/SKILL.md` tell coding agents how to use itda:
  discover actions, run reads, treat website text as data, wait for `itda
  approve`, never retry a lost change, run `itda check` first when something
  stops working. The MCP server's instructions say the same, shorter.
- `itda feedback` prints a link to a new GitHub issue with the version, the
  platform and the doctor checks filled in (never the data directory path)
  and opens it; `--no-open` only prints. It sends nothing itself.
- `itda check` ends each site with a verdict and what to do: `ok`,
  `signed_out`, `site_changed`, `blocked` or `unreachable`. The exit code
  follows the worst verdict, and `--json` carries it as `verdict` and per
  site under `sites`.

### Changed

- `action_outcome_unknown` now says that itda never resends a change on its
  own, and the runtime tests prove it against a server that drops the
  connection after taking the request.
- A bare `itda` in a terminal says what it is and asks for one address, then
  runs `itda setup` on it, so the first run is: type `itda`, paste, Enter.
- `itda setup` finds the agent apps on this computer (Claude Desktop, Claude
  Code, Cursor, Codex, or a project with a `.vscode` folder) and connects them
  all after one Enter; the numbered list only appears when none is found.
- `itda setup` asks whether the site needs a sign-in only when the page it
  fetched has a password form; a page without one is used as it is.
- The installer ends with "Open a new terminal window and type: itda".
- At the first prompt, a ready-made site's name (`github`, `npm`, `crates-io`,
  `pypi`) installs that site and connects the agent apps found on this
  computer.
- After adding itda to Claude Desktop on macOS, itda offers to restart the app
  when it is open, or says that opening it is enough; for Claude Code it runs
  `claude mcp add` itself when `claude` is on the PATH.
- `itda doctor` lists the agent apps whose settings already start itda.

## [0.4.0] - 2026-10-22

One address is enough: `itda setup` looks at a page, or watches you use the
site in your own browser, names what agents will be able to do in plain words,
tries every read, and connects an agent. Any website, not only the ones with an
API description.

### Added

- `itda setup <url>` sets a site up in one go: it looks at the page, or
  opens your browser and records when the page refuses, needs a sign-in or
  is drawn by scripts; shows what agents will be able to do in plain words;
  tries every read once and keeps the ones that work; saves the site; and
  connects an agent. It asks before each step; `--record`/`--no-record`,
  `--client` and `--yes` answer for scripts. `ITDA_LANG=ko` (or a Korean
  `LANG`) makes it, and the drafted descriptions, speak Korean.
- Drafted actions carry a description a person would say (`List what
  Notices shows`, `공지사항 목록 읽기`), taken from the page's title or the
  call's name, in English or Korean.
- `itda record <url>` opens your own browser in a fresh window and records
  what the pages ask the server for while you use the site, then drafts a
  definition the same way `itda draft har` does. The recording stays in
  memory; `--save` keeps it as a HAR file. itda never clicks, types or
  navigates in the window. `ITDA_DEBUG_CDP=1` prints the browser messages.
- `itda record --install` offers to keep the token and the browser session
  the recording signed in with, each after a yes in the terminal, so a site
  that sends a bearer token from its scripts works without pasting it.
- A recorded GraphQL call becomes one action per operation (`exploreProjects`
  is `explore-projects`): queries are reads, mutations are changes, and the
  variables are the typed inputs, rendered through a `json_template`.
- `itda record` and `itda draft har` also draft the pages the recording
  holds, as `itda draft page` does, so a site that renders its lists and
  forms on the server gets actions even when its pages call no API.
- `itda draft page <url>...` drafts a site definition from the pages you
  use, fetched with the honest client: a table or a list of records becomes
  `html` fields, data a page embeds for its scripts becomes `ssr_json`, a
  search form becomes a read, a post form becomes a change with its token
  read through a preflight step, page links become a `page` input, and RSS or
  Atom feeds come along. Every read gets a response check, so `itda check`
  notices when a page changes. Lists are found by their shape, not only by
  known class names: any run of three or more siblings outside navigation
  that each hold a link at the same place, with the title and link read
  from that link's own path. When a page embeds its data for its scripts,
  the draft points `extract_pointer` at the list inside the payload and
  checks the keys every record shares.
- `itda draft har` also handles a token sent as a header, such as
  `X-CSRF-Token`: a preflight step reads it from the page's
  `<meta name="csrf-token">` tag before each call.
- `itda add` also takes an https URL, or the name of a ready-made site
  (`itda add github`); the four definitions in `sites/` are built in.

### Changed

- `itda setup` names the agent app it connects ("Adding itda to Claude
  Desktop..."), shows the settings file it wrote with `~` for your home
  folder, says so in Korean too, lines the action names up only as wide as
  they need, and writes its example request mid-sentence.
- The README and the landing page open with a recording of `itda setup` as
  it ran on Hacker News (`assets/setup.gif`, Korean in `assets/setup.ko.gif`,
  rebuilt from `assets/src/setup.*.tape` with vhs).
- A new key visual, a hand passing a website card to an agent
  (`assets/handoff.png`), on the README banners and the link preview
  (`assets/hero.png`, `assets/hero.ko.png`, `assets/social.png`, rendered from
  `assets/src/banner.html`), and the docs introduction says the same thing the
  README does.

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

[Unreleased]: https://github.com/brnyxx/itda-cli/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/brnyxx/itda-cli/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/brnyxx/itda-cli/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/brnyxx/itda-cli/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/brnyxx/itda-cli/releases/tag/v0.1.0
