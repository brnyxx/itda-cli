# itda

**Don't drive the browser. Compile the site.**

<img src="https://raw.githubusercontent.com/brnyxx/itda-cli/main/assets/hero.png" alt="itda drafts actions from a browser recording, reads freely, and waits for a person to approve a change" width="100%">

itda turns a website into a small set of verified, typed actions that an AI
agent can call from the command line or over MCP. It runs on your own machine,
reads by default, and never changes anything on a site until you approve that
exact change.

```console
$ itda call sandbox.list-notices --q 점검
{
  "items": [
    {"author": "운영팀", "date": "2026-10-01", "id": 1, "title": "10월 정기 점검 안내"}
  ],
  "total": 1
}
```

## Why

Agents that drive a browser re-read every page, spend thousands of tokens on
each screenshot or accessibility snapshot, follow whatever the page tells
them, and break silently when a layout changes. itda moves that work to an
install step. A site definition says which actions exist, what they send, what
a correct answer looks like and whether they change anything. After that,
every call is small, typed and checked.

- **Verified.** A signed-out page or a redesigned site is an error
  (`auth_required`, `response_mismatch`), never data.
- **Read-only by default.** Changes wait for a person to approve the exact
  request in a terminal. No agent tool can approve.
- **Honest.** itda says who it is, never imitates a browser, and stops when a
  site refuses it.
- **Accountable.** Every call, approval and change goes into a hash-chained
  audit log you can verify.
- **Local.** No server, account or telemetry. Secrets stay in your OS keyring.

## Where to go next

- [Quick start](quickstart.md): install and try it on the bundled sandbox.
- [Use it from an agent](agents.md): connect Claude, Cursor, VS Code or Codex.
- [Writing a site definition](SPEC.md): add your own site.
