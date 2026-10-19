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
- When one change depends on another, open them as a stack: each pull request
  targets the branch of the one before it, and they merge in order.
- Branch names follow the commit type: `feat/net-gate`, `fix/redirect-origin`.
- The description says what a reader sees before and after the change, then how.

## Checks

```sh
cargo fmt --all --check
cargo xtask check-layers
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

CI runs the same checks on Linux, macOS and Windows.

## Code derived from Hycli

A file that contains code derived from Hycli says so in its header comment,
names the original file and describes what changed. Keep that header when you
edit the file.

## Releasing

1. Move the `Unreleased` notes in CHANGELOG.md under the new version and bump
   `version` in the root Cargo.toml.
2. On GitHub, open Actions → Release → Run workflow, and enter the tag, such as
   `v0.2.0`. The workflow builds every platform, creates the tag and publishes
   the release with the installers. Leave the default `dry-run` to build
   without publishing.
