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
itda 0.4.0
ok    data directory   /home/you/.local/share/itda
ok    secrets          system keyring; long secrets sealed in ~/.local/share/itda/vault.sealed with a key from the keyring
ok    audit log        0 records, chain intact
ok    changes          none with an unknown outcome
ok    sites            0 installed, 0 invalid
ok    agents           none connected (itda setup <url>, or itda mcp install <client>)
ok    local addresses  refused (ITDA_ALLOW_LOCAL=1 allows them)
```

## Set up a site you use

Type `itda`, paste the address of a site you use, and press Enter. itda looks
at the page; when the page is drawn by scripts and shows itda only a title,
it opens your browser on it for a few seconds, watches what the page loads by
itself and drafts from those calls (nothing is clicked or typed, and the
window closes on its own); when the page needs a sign-in, it asks for your
user name and password and signs in through the site's own form, as itself
(`--user <name>` from a script, the password on stdin), or asks to open your
browser and watch while you use the site. It then shows what agents will be
able to do, tries every read, and connects the agent apps it finds on your
computer after one more Enter. From then on, ask your agent for things on
that site. `itda setup <address>` does the same from a script (`--no-record`
never opens the browser, `--watch-only` watches but never asks you to
record); `itda setup --help` lists the flags.

A site that runs software itda knows (GitLab, Jira, Confluence, Redmine,
Discourse, MediaWiki, WordPress, Gitea or Forgejo) is recognised from its
page and confirmed with one request to that software's own API. itda then
says "This looks like GitLab. Using its API at <base>." and the agent gets
the software's real reads and changes from a ready-made template, with the
page's own lists and forms joining them. `--as <software>` forces a template
when the page does not say; `--no-template` skips them.

## From inside your agent

Once itda is connected (`itda mcp install <client>`, or the question at the
end of a setup), you can skip the terminal for the next sites. Tell your
agent "use news.ycombinator.com": it calls `itda_setup` with the address,
the site is looked at, drafted and tried the same way, and its actions show
up as tools in the same conversation. A page drawn by scripts opens briefly
in a browser window on your screen so itda can watch what it loads; nothing
in it is clicked or typed. When a read says to sign in, the agent
calls `itda_login` and your own browser opens for you to sign in; the agent
only ever sees the account id. `itda_check`, `itda_sites` and `itda_manage`
cover checking, listing, removing and resuming sites and reading approvals
and the audit log, with a form for you to confirm the ones that change
something. Nothing an agent can call approves a change.

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
itda approve <id>                           # shows what it does and the exact request, asks y/N
itda call sandbox.create-notice --title hi --approval <id>
itda audit                                  # everything above, hash-chained
itda describe sandbox                       # every action with its inputs and a command to copy
```

Without a keyring (CI, containers), skip `itda login` and set
`ITDA_SECRET_STORE=env ITDA_SECRET__SANDBOX__DEFAULT__API_KEY=sandbox-key`.

Requests to one site go one at a time, spaced about 1.2 seconds apart, as
they would be for a real website, so consecutive or parallel calls take a
moment. A site definition can ask for more room with `defaults.interval`.

## Commands

| Command | What it does |
|---|---|
| `itda setup <url> [--name id] [--record\|--no-record] [--as software\|--no-template] [--client name\|none] [--yes]` | One go: look at the page or record you using the site, name the actions in plain words (`ITDA_LANG=ko` for Korean), try every read, keep what works, connect an agent. Known software (GitLab, Jira, Confluence, Redmine, Discourse, MediaWiki, WordPress, Gitea) gets its API from a template; `--as` forces one, `--no-template` skips them |
| `itda draft page <url>… [--name id] [--base-url url] [--include text] [--install] [--force]` | Draft a site definition from the pages you use: tables, lists, embedded data, forms and feeds, plus the reads of the API description the site publishes |
| `itda record <url> [--name id] [--install] [--save file.har]` | Use a site in your own browser while itda records what its pages ask for, then draft a definition from that |
| `itda draft openapi <file> [--install]` · `itda draft har <file> [--install]` | Draft one from an API description or a saved browser recording |
| `itda add <file\|url\|name\|-> [--force]` | Install a site definition (a file, an https URL, stdin, or a ready-made site such as `github`) after showing where it sends requests and which actions change the site |
| `itda remove <site>` | Remove an installed site definition; its accounts stay until `itda logout` |
| `itda ls [site] [--json]` | Installed sites and their actions |
| `itda describe <site>[.<action>] [--json]` | Each action with its inputs and an example command to copy |
| `itda call <site>.<action> [--input value]… [--account id] [--approval id] [--json]` | Run one action |
| `itda login <site> [--user name] [--account id]` | Sign in through the site's own form (the definition names it): the password comes from a prompt, or from stdin; the session is kept in the keyring |
| `itda login <site> --secret <name> [--account id]` | Store a secret read from stdin, such as an API key, under that name |
| `itda login <site> --browser [--url page]` | Sign in in your own browser and keep that site's session |
| `itda accounts [<site>] [--json]` | Signed-in accounts and their secret names, never the values |
| `itda logout <site> [--account id]` | Remove an account and its saved secrets |
| `itda resume <site>` | Resume after a block or an expired sign-in |
| `itda approvals` · `approve <id>` · `reject <id>` | Review changes agents asked for; `reject` also withdraws one you granted |
| `itda audit [--limit n] [--verify] [--json]` | What agents did, and the changes a stopped process left without an outcome |
| `itda mcp [--mode direct\|catalog] [--approve-in-client] [--site name]` | Serve installed sites over MCP, with tools to set up, sign in, check and manage sites from the agent; `--site` serves one site only |
| `itda mcp install <client> [--dry-run]` | Add itda to an MCP client |
| `itda doctor` | Check the setup |
| `itda feedback [--no-open]` | Print and open a link to a new GitHub issue with the version, the platform and the doctor checks filled in; sends nothing itself |
| `itda check [<site>\|--all] [--account id] [--json]` | Run every read that needs no input and end each site with a verdict: `ok`, `signed_out`, `site_changed`, `blocked` or `unreachable`, with what to do; `--all` prints one verdict line per site. Run it when an action that used to work fails, before retrying or re-drafting |
| `itda refresh <site> [-y]` | When a check says `site_changed`: draft the site again from the sources it was set up from, show what would change (actions added, removed, changed) and write it after a yes, keeping the sign-in, the defaults, the name and every action you edited by hand. Then checks the site |
