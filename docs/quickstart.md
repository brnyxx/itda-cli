# Quick start

## Install

macOS and Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
```

Windows (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex"
```

Or download an archive from
[Releases](https://github.com/brnyxx/itda-cli/releases), or build from source
with `cargo install --git https://github.com/brnyxx/itda-cli itda-cli`.

Check the setup:

```console
$ itda doctor
itda 0.3.0
ok    data directory   /home/you/.local/share/itda
ok    secrets          system keyring
ok    audit log        0 records, chain intact
ok    sites            0 installed, 0 invalid
ok    local addresses  refused (ITDA_ALLOW_LOCAL=1 allows them)
```

## Try the sandbox

The repository includes a small fake intranet that only listens on
127.0.0.1: a notice board with an HTML table, a JSON API behind an API key, and
a page that refuses automated access.

```sh
git clone https://github.com/brnyxx/itda-cli && cd itda-cli
cargo run -p itda-sandbox            # http://127.0.0.1:8787
```

In another terminal, use a scratch data directory and install its definition:

```sh
export ITDA_HOME=$(mktemp -d) ITDA_ALLOW_LOCAL=1
mkdir -p "$ITDA_HOME/specs/sandbox"
cp examples/sandbox/spec.yaml "$ITDA_HOME/specs/sandbox/"
itda ls
```

Read, sign in, and see what happens when a site says no:

```sh
itda call sandbox.list-notices --q 보안      # JSON API
itda call sandbox.notice-board --json       # HTML table extracted into records
itda call sandbox.me                        # exit 10: no API key yet
echo sandbox-key | itda login sandbox       # stores the key in the OS keyring
itda call sandbox.me                        # signed in
itda call sandbox.reports                   # exit 12: refused, so the site is paused
itda resume sandbox
```

Make a change:

```sh
itda call sandbox.create-notice --title hi  # exit 20: waits for approval <id>
itda approve <id>                           # shows the exact request, asks y/N
itda call sandbox.create-notice --title hi --approval <id>
itda audit                                  # everything above, hash-chained
```

Without a keyring (CI, containers), skip `itda login` and set
`ITDA_SECRET_STORE=env ITDA_SECRET__SANDBOX__DEFAULT__API_KEY=sandbox-key`.

Requests to one site are spaced about 1.2 seconds apart, as they would be for a
real website, so consecutive calls take a moment.

## Commands

| Command | What it does |
|---|---|
| `itda add <file\|url\|name\|-> [--force]` | Install a site definition (a file, an https URL, stdin, or a ready-made site such as `github`) after showing where it sends requests and which actions change the site |
| `itda remove <site>` | Remove an installed site definition; its accounts stay until `itda logout` |
| `itda ls [site] [--json]` | Installed sites and their actions |
| `itda call <site>.<action> [--input value]… [--account id] [--approval id] [--json]` | Run one action |
| `itda login <site> [--account id] [--secret name]` | Store a secret read from stdin |
| `itda login <site> --browser [--url page]` | Sign in in your own browser and keep that site's session |
| `itda accounts [<site>] [--json]` | Signed-in accounts and their secret names, never the values |
| `itda logout <site> [--account id]` | Remove an account and its saved secrets |
| `itda resume <site>` | Resume after a block or an expired sign-in |
| `itda approvals` · `approve <id>` · `reject <id>` | Review changes agents asked for |
| `itda audit [--limit n] [--verify] [--json]` | What agents did |
| `itda mcp [--mode direct\|catalog]` | Serve installed sites over MCP |
| `itda mcp install <client> [--dry-run]` | Add itda to an MCP client |
| `itda doctor` | Check the setup |
| `itda check [<site>] [--account id] [--json]` | Run every read that needs no input and report which no longer match the site |
