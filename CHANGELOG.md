# Changelog

All notable changes to itda are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). Until 1.0, a minor version may
change the definition format or the CLI.

## [Unreleased]

### Added

- Sites that run known software are recognised. `itda setup` reads the
  page's HTML for GitLab, Jira, Confluence, Redmine, Discourse, MediaWiki,
  WordPress and Gitea or Forgejo (a `generator` meta, well-known asset paths,
  the Atlassian context-path metas, body classes), confirms the guess with
  exactly one GET to that software's own version or info endpoint, and then
  drafts from a template under `sites/software/` instead of reading the page
  like any other: the reads an agent needs most, up to three changes marked
  as such, models and the software's own way of signing in, every operation
  with its documentation in `evidence`. The template's `base_url` is filled
  with the instance's API base, context path included (`/jira`, `/w`, a
  GitLab relative URL root, a Gitea `appSubUrl`, the WordPress API root the
  page links to); the site is named after the host; one line says so ("This
  looks like GitLab. Using its API at <base>."). The page draft still runs
  and its lists, forms and feeds join the template, the same method and path
  drafted once, and a page that needs a sign-in or scripts still gets the
  template when the software is confirmed. A fingerprint the API does not
  confirm is dropped quietly. `itda setup --as <software>` forces a
  template, `--no-template` skips them, `itda check` works on the result as
  on any site, and `itda_setup` over MCP takes `template` and names the
  template it used in its result. The site's `sources.json` names the
  template and its API base, so `itda refresh` fills the same template
  again, with the page read again when it still opens.
- `itda refresh <site>`: when `itda check` says `site_changed`, draft the
  site again from the sources it was set up from and merge the result with
  the installed definition. The site's name, title and address, `defaults`,
  `auth` and every action edited by hand are kept; actions the draft
  changed take the new version, ones it no longer drafts are dropped only
  when their read also failed, and new ones are tried once and added when
  they answer. The diff is printed in plain words (added, removed, changed,
  kept as edited) and written after a yes, or at once with `-y`; the site
  is checked right after and the verdict printed. Approvals are untouched.
  To make that possible, `itda setup`, `itda draft ... --install` and `itda
  record --install` now write `specs/<site>/sources.json` next to the
  definition: the pages, the API description, the GraphQL endpoints asked,
  the recording when one was kept (copied under the site as
  `recording.har` with its cookie, token and session values, private
  fields and sign-in answers blanked), the draft's options and a hash of
  every operation as drafted. `itda check` now points at `itda refresh <site>` when a site
  changed, and `itda check --all` prints one verdict line per site.
- `itda_manage` over MCP gains `refresh`: the person confirms the diff in a
  form (a form nobody answers counts as no), and the site's tools are
  reloaded and checked. `itda_check` without a site already checks every
  site; the docs now say so.
- Five MCP tools so an agent never needs a terminal to set a site up or
  look after it: `itda_setup` (look at a page as itda, draft, try every
  read once, keep what works; the site's actions become tools in the same
  session, announced with `notifications/tools/list_changed`, which the
  server now declares with `tools.listChanged`), `itda_login` (the person
  signs in in their own browser, after a form in clients that show one;
  the definition must name the sign-in cookie), `itda_check` (the verdict
  per site, as `itda check --json`), `itda_sites` (what is set up, with
  the last check's time and verdict, now kept in `specs/<site>/check.json`)
  and `itda_manage` (`remove` and `resume` after the person confirms in a
  form, `approvals`, `audit`). `itda_setup` never records and never keeps
  a credential; pages that need a sign-in or scripts are refused with the
  command to run instead. The server instructions, `AGENTS.md` and the
  skill say when to call them.
- A new `itda-setup` crate holds the steps behind `itda setup`, `itda login
  --browser`, `itda check` and the installed sites, with the questions
  answered through a trait and progress reported through a callback, so the
  command line and the MCP server run the same code. The CLI's words and
  questions are unchanged.
- `itda draft page` and `itda setup` draft a typed read for each list they
  find in a page's embedded data, instead of one read that returns the whole
  payload: an `ItemList` or a list of one typed thing (`NewsArticle`,
  `Product`...) in `ld+json`, and in `__NEXT_DATA__`, `__NEXT_F__`,
  `__APOLLO_STATE__`, `__PRELOADED_STATE__` and the like the list of objects
  with the most to read. Each read is named after what it holds
  (`list-news-articles`), points `extract_pointer` at the list and keeps up
  to eight plain fields of every record through a `model`, ids and titles
  first and credentials left out. `itda check` counts its records. The whole
  payload is drafted only when no list is in it.
- An Apollo cache (`__APOLLO_STATE__`) is also grouped by typename, so a
  definition can point at `/Question` or `/Product`.
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
- A draft that sees a GraphQL endpoint (a recorded call to it, or a page
  naming a same-origin `/graphql` path) asks it once, with the standard
  introspection query, what it offers, and drafts one read per root `Query`
  field whose arguments are scalars, enums or ids: the field name is the
  action, the arguments are typed inputs, and the selection is the field's
  scalars two levels deep, 24 at most. An optional argument that holds an
  object or a list is left out and listed; a field that requires one is
  listed instead. `Mutation` fields are listed in the notes and never
  drafted. A site with introspection off is drafted from
  its pages and calls alone. The query is sent as itda, read-only, with a
  size cap, by `itda draft page`, `itda draft har`, `itda record` and
  `itda setup`.
- An optional input the agent leaves out now leaves the `json_template`
  body too, instead of standing in as an empty string where a number or
  null belongs; GraphQL variables typed `Int` or `Boolean` need that.
- `itda draft page` and `itda setup` read the API description a site
  publishes: the one the page links to, else `/openapi.json` on the same
  origin when it answers with one (one GET, as itda, at most 4 MB). Its reads
  join the draft the way `itda draft
  openapi` makes them, drafted once when the page already had the same path;
  its changes become notes, and a read keyed by a credential input is left
  out and listed. A description on another address is noted, not merged,
  and one that does not parse leaves the page draft as it is. The
  header comment and each merged action's `evidence` name where it came from,
  and `itda setup` says what it read in one line.

### Changed

- `itda draft page` and `itda setup` on a page that refuses itda say what the
  gate saw instead of `error[internal]: <url> answered HTTP 403`: the host is
  behind Cloudflare, runs a bot manager or asks for a captcha
  (`blocked_by_site`, exit 12), or limits how often it is read, with the wait
  it asked for when it named one (`site_rate_limited`, exit 13), in English
  or Korean. A captcha served with HTTP 200 is reported the same way, and a
  page that sends itda on to a sign-in page says so (`auth_required`, exit
  10) and points at recording.
- A `Retry-After: 0`, or a date already past, no longer counts as a rate
  limit: a site that sends it with every page is read as itself.
- The landing page on phones: lighter glass, the setup dialog left out of the
  hero stack, full-width buttons, tighter rhythm, terminal lines wrapped on
  anything narrower than the two-column layout, and the sky drawn smaller
  with the cursor effect only while a finger is down.
- The landing page and both READMEs carry a prompt to hand to an agent: paste
  it into Claude Code, Cursor or Codex with a site's address and the agent
  installs itda, runs `itda setup <address> --client <app> -y` and starts
  using the site.

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
