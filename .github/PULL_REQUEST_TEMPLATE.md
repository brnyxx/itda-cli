<!-- One crate, feature or fix per pull request. See CONTRIBUTING.md. -->

## Before and after

What a reader sees before and after this change.

## How

The shape of the change, in a few lines. Leave out what the diff shows.

## Checks

- [ ] `cargo fmt --all --check`
- [ ] `cargo xtask check-layers`
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] `cargo test --workspace --locked`
- [ ] English and Korean text say the same thing (README, `site/i18n.js`, `docs/`), or the description says why not
- [ ] `CHANGELOG.md` has a line under `Unreleased` when users will notice the change
