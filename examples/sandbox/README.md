# Sandbox intranet

A small notice board that behaves like an older internal system: a
server-rendered HTML table, a JSON API behind an API key, a write form that
only takes a one-time CSRF token from its own page, and a page that refuses
automated access. Use it to try itda without touching a real website.
It only listens on 127.0.0.1.

```sh
cargo run -p itda-sandbox            # http://127.0.0.1:8787
```

In another terminal, install its definition into a scratch data directory:

```sh
export ITDA_HOME=$(mktemp -d) ITDA_ALLOW_LOCAL=1
cargo build -p itda-cli && alias itda=target/debug/itda
itda add examples/sandbox/spec.yaml

itda ls
itda call sandbox.list-notices --q 보안      # JSON API
itda call sandbox.notice-board --json       # HTML table extracted into records
itda call sandbox.me                        # exit 10: no API key yet
echo sandbox-key | itda login sandbox       # stores the key in the system keyring
itda call sandbox.me                        # signed in
itda call sandbox.create-notice --title hi  # exit 20: waits for approval <id>
itda approvals                              # what is waiting
itda approve <id>                           # shows the exact request, asks y/N
itda call sandbox.create-notice --title hi --approval <id>   # runs once
itda call sandbox.write-notice --title hi   # a form post: approve it the same way;
                                            # itda reads a fresh CSRF token first
itda audit                                  # everything above, hash-chained
itda call sandbox.reports                   # exit 12: the site refused, so it is paused
itda call sandbox.list-notices              # still paused, nothing is sent
itda resume sandbox
```

Without a system keyring (CI, containers), skip `itda login` and use
`ITDA_SECRET_STORE=env ITDA_SECRET__SANDBOX__DEFAULT__API_KEY=sandbox-key`.

Requests to the same site are spaced about 1.2 seconds apart, as they would
be for a real website, so consecutive calls take a moment.
