# itda-cli

Turn a website into verified, typed actions that an AI agent can call from the
command line or over MCP, on your own machine.

> Status: early foundation, built crate by crate. The definition format, safety
> policy and network layer come first; the runtime, credential vault, MCP server
> and CLI follow.

## Principles

- **Local first.** Site definitions run on your device. Credentials never leave it.
- **Read-only by default.** An action is a read only when its method, name, path,
  inputs and GraphQL document all agree. Everything else is a change, and every
  change needs a single-use approval bound to its exact inputs.
- **Blocks are reported, not bypassed.** The client identifies itself, does not
  imitate a browser, and pauses a site that refuses or challenges it until you
  resume it. A signed-in request that is no longer accepted pauses that account.
- **Verified actions.** A definition declares what a correct response looks like,
  so "signed out" and "the site changed" are detected instead of returning a
  login page as data.

## Layout

Crates depend only on crates below them. `cargo xtask check-layers` enforces this.

| Layer | Crate | Purpose |
|---|---|---|
| 0 | `itda-types` | Error contract, URL checks (Public Suffix List), redaction |
| 1 | `itda-html` | Script-free HTML and embedded-JSON extraction |
| 2 | `itda-spec` | SiteSpec format, request building, response verification |
| 3 | `itda-policy` | Read/write classification, spec safety validation, approvals |
| 4 | `itda-net` | Bounded HTTP, pacing, block classification, private-address guard |

Planned: `vault`, `browser`, `runtime`, `prep`, `mcp`, `server`, `apps/cli`.
See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Develop

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo xtask check-layers
```

Rust 1.98 (edition 2024) is pinned in `rust-toolchain.toml`.

## Credits and license

Licensed under [Apache-2.0](LICENSE). Parts of this project are derived from
[Hycli](https://github.com/Hybirdss/Hycli) (Apache-2.0); see [NOTICE](NOTICE).
Each derived file says so in its header and describes what changed. This project
is not affiliated with Hycli and does not use its name or marks.
