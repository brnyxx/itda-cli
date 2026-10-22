# Contributing

## Commits

One commit is one change a reviewer can understand on its own: a module, a
feature, a fix. Every commit builds and passes the checks below.

```
<type>: <summary>

- <change>
- <change>
```

- **Subject:** `<type>: <summary>`, English, imperative mood, lower case after
  the colon, no trailing period, 72 characters at most.
- **Body:** a list of the changes, one item per line. Say what changed and,
  when it is not obvious, why. Leave out what the diff already shows.
- **Types:**

  | Type | Use for |
  |---|---|
  | `feat` | New behaviour, a new module or crate |
  | `fix` | A bug fix |
  | `refactor` | A change that keeps behaviour the same |
  | `docs` | Documentation only |
  | `test` | Tests only |
  | `build` | Workspace, dependencies, toolchain, `xtask` |
  | `ci` | GitHub Actions |
  | `chore` | Anything else that ships nothing (formatting, renames) |

Example:

```
feat: add same-site checks based on the public suffix list

- registrable_domain uses psl instead of a hand-written suffix table
- a.github.io and b.github.io are now different sites
- tests cover multi-label suffixes and IP hosts
```

## Pull requests

- One crate, feature or fix per pull request. Aim for under about 500 changed
  lines, not counting `Cargo.lock` and test fixtures. Split anything larger.
- For dependent changes, open a stack of pull requests. Each targets the
  previous request's branch; they merge in order.
- Branch names follow the commit type: `feat/net-gate`, `fix/redirect-origin`.
- The description says what a reader sees before and after the change, then how.

## Site definitions and software templates

The easiest contribution is a site. Two kinds live in the repository:

- `sites/<name>.yaml` is one public site, built into the binary so
  `itda add <name>` works without a checkout. Add the file and its line to
  `BUNDLED` in `crates/setup/src/sites.rs` and to the table in
  `sites/README.md`. A test installs every bundled site and checks that its
  `site.name` matches the file name.
- `sites/software/<name>.yaml` is one kind of software (GitLab, Jira, a wiki),
  with a placeholder `base_url` that `itda setup` fills in. Add the file and
  its entry to `SOFTWARE` in `crates/setup/src/software.rs`, with the words
  and the one confirming request that recognise it. A test fills every
  template and runs the install checks on it.

A good definition reads public data or uses the site's documented API. Every
read has a `response` check and a `model`. It keeps only the few changes an
agent needs and links each action's docs in `evidence`. It never works around a
block or imitates a browser; see [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md).
Before opening the pull request, run `itda add sites/<name>.yaml` and
`itda check <name>` against the real site and paste the check's summary into the
description.

## Issues and questions

- A bug: the **Bug report** template. `itda feedback` fills in the version,
  the platform and the doctor checks.
- A site that stopped answering: the **Site stopped working** template, with
  `itda check <site>` output.
- A site or software you want set up well: the **Site request** template.
- A question about whether itda fits what you are doing, or how to set a site
  up: [Discussions](https://github.com/brnyxx/itda-cli/discussions).
- A vulnerability: privately, as [SECURITY.md](SECURITY.md) says. Never in an
  issue.

## Translations

`README.ko.md` and the Korean strings in `site/i18n.js` follow `README.md` and
`site/index.html`. A pull request that changes the English text updates the
Korean text too, or says in its description that it does not. New elements on
the landing page get a `data-i18n` key with a Korean entry.
`python3 assets/src/flow.py` builds the animated diagrams and holds both
languages.

## Checks

```sh
cargo fmt --all --check
cargo xtask check-layers
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

CI runs the same checks on Linux, macOS and Windows.

## Code derived from Hycli

A file that contains code derived from Hycli says so in its header comment. The
header names the original file and describes what changed. Keep that header when
you edit the file.

## Versions

itda is before 1.0, so the version says this:

- **Minor** (`0.5.0` → `0.6.0`): new behaviour, and any change a person or an
  agent has to adapt to (a renamed command or tool, a new exit code, a field
  that is no longer accepted, a definition that now needs a different key).
  Every change that touches `docs/SPEC.md` is a minor bump.
- **Patch** (`0.5.0` → `0.5.1`): fixes only. Nothing new to learn.
- **1.0.0** comes when the site definition format, the exit codes and the MCP
  tools have gone one minor release without a change people had to adapt to.

Each change that users will notice gets a line in `CHANGELOG.md` under
`Unreleased`, in one of `Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`,
`Security`. Each heading appears at most once. `cargo xtask check-changelog`
(run by CI) checks that. It also checks that the newest release heading,
`site/index.html` and `docs/quickstart.md` carry the workspace version.

## Releasing

1. Move the `Unreleased` notes in CHANGELOG.md under the new version with the
   date. Bump `version` in the root Cargo.toml and update the version shown in
   `site/index.html` and `docs/quickstart.md`. `cargo xtask check-changelog`
   passes when the four agree.
2. On GitHub, open Actions → Release → Run workflow, and enter the tag, such as
   `v0.2.0`. The workflow builds every platform. It creates the tag and
   publishes the release with installers. Leave the default `dry-run` to build
   without publishing.
