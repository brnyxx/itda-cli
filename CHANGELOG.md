# Changelog

All notable changes to itda are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). Until 1.0, a minor version may
change the definition format or the CLI.

## [Unreleased]

### Added

- `itda draft har`: draft a site definition from a browser recording of you
  using the site. Only the shape of each call is kept; recorded tokens,
  cookies and values never reach the draft.

## [0.1.0] - 2026-10-19

The first release: reads and approved changes on plain HTTP sites with API
keys, from the command line or over MCP, and definitions drafted from OpenAPI.

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

[Unreleased]: https://github.com/brnyxx/itda-cli/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/brnyxx/itda-cli/releases/tag/v0.1.0
