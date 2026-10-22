# The intranet lab

The lab tests itda's sign-in paths against self-hosted software behind a sign-in
rather than fixtures: Gitea, Redmine, WordPress, MediaWiki, a small legacy
intranet written for the lab, and three uglier cases built the harder ways (a
board inside an iframe, a table a script draws from an XHR, a session carried in
the URL). Every public-site number itda reports is measured; this is where the
numbers for "the old intranet behind a login" come from.

Nothing here is published anywhere. Images are pinned by digest in
`compose.yaml`, every port binds to 127.0.0.1, and the only account is
`lab` / `labpassword`.

| Service | Address | Version | What itda does there |
|---|---|---|---|
| Gitea | http://127.0.0.1:8081 | 1.24.7 | lists the issues of a private repository, opens one through the web form (`_csrf`) |
| Redmine | http://127.0.0.1:8082 | 6.0.11 | lists the issues of a private project, creates one through the web form (`authenticity_token`, session cookie rewritten per answer) |
| WordPress | http://127.0.0.1:8083 | 6.8.3 (MariaDB 11.4.13) | lists private posts and publishes one through the REST API with the session and a `rest-nonce` |
| MediaWiki | http://127.0.0.1:8084 | 1.43.11 | lists the pages of a private wiki and edits one through the API with a csrf token |
| legacy intranet | http://127.0.0.1:8085 | `intranet/server.py` on Python 3.12 | a login form with a CSRF token, a session cookie, a notices table with paging, a search form, a post form that needs the token |
| framed | http://127.0.0.1:8086 | `intranet/framed.py` on Python 3.12 | the notices table two iframes deep (`/` -> `/frame/board` -> `/frame/list`), each behind the same sign-in; a plain fetch of the home page sees only the shell |
| scripted | http://127.0.0.1:8087 | `intranet/scripted.py` on Python 3.12 | an empty `/board` shell a script fills from `GET /api/notices` (JSON), public to read; a post needs the session and the `X-CSRF-Token` the shell carries in `<meta name="csrf-token">` |
| urltoken | http://127.0.0.1:8088 | `intranet/urltoken.py` on Python 3.12 | no cookie at all: the sign-in redirects to `/notices?sid=<token>` and every link and form carries the `sid`; a request without a valid one goes to `/login` |

## Run it

Docker (with Compose v2) is all it needs.

```sh
examples/lab/lab.sh up       # starts every service, waits until each answers, seeds the account and content
examples/lab/lab.sh status   # one line per service
examples/lab/lab.sh down     # stops; the data stays for the next up
examples/lab/lab.sh reset    # stops and deletes the data, so the next up starts cold
```

`up` is safe to repeat: it seeds only what is missing. Measured on this
machine (Apple Silicon, Docker 28.5.1, images already pulled): 31 seconds from
cold (`reset`, then `up`), 14 seconds when the data already exists. The first
run also pulls the images, which took a few minutes here.

Then, with the lab up, from the repository root:

```sh
export ITDA_HOME=$(mktemp -d) ITDA_ALLOW_LOCAL=1
printf 'labpassword\n' | itda setup http://127.0.0.1:8085/notices --name intranet --user lab --no-record --client none --yes
itda call intranet.list-notices --page 2 --json
itda add examples/lab/specs/intranet.yaml --force     # the reviewed definition
itda login intranet --user lab                        # asks for the password
itda call intranet.post-notice --title "hello"        # waits for your approval
itda approve <id>
itda call intranet.post-notice --title "hello" --approval <id>
itda audit --verify
```

The same works for `gitea` (`/issues`), `redmine` (`/projects/lab/issues`),
`wordpress` (`/wp-admin/edit.php`) and `mediawiki`
(`/index.php/Special:AllPages`); `specs/` holds a reviewed definition for
each, which is what a person would keep after looking at the draft.

## The test suite

`apps/cli/tests/lab.rs` runs only with `ITDA_LAB=1`; without it every test
says it skipped, so CI never touches the lab.

```sh
examples/lab/lab.sh up
ITDA_LAB=1 cargo test -p itda-cli --test lab
```

Per service, in order: `itda setup <page> --user lab` (the page needs a
sign-in; itda signs in through the site's own form, drafts from the signed-in
page and keeps the session), a drafted read answers, the reviewed definition
is installed, `itda logout`, `itda login --user lab` with a wrong password
(refused, exit 10) and the right one, a read answers with records, a change
waits for an approval, the approval is given in a terminal (`expect` drives
`itda approve`, since it refuses piped input), the change lands once and a
second call with the same approval is refused, the read shows the change,
`itda audit --verify` passes and names the approval and the change, `itda
check` says `ok`.

The three uglier cases have their own tests. `framed`, `scripted` and `urltoken`
run from the reviewed definition to an approved change the same way. `urltoken`
also tests setup, signing in and keeping the `?sid=` session. `scripted` tests
the browser watch on Unix only; it needs `ITDA_TEST_BROWSER` set to a Chrome or
Chromium binary and skips the test without one. The one itda cannot do today,
`framed` from the shell, is written for the behaviour we want and marked
`#[ignore = "setup does not sign in from a shell frame that redirects to a login"]`.
`cargo test ... -- --ignored` shows it failing with the exact reason in the
comment above it.

Sessions are sealed under each test's data directory (`vault.sealed`), with
one master key per directory in the real system keyring, under `itda` /
`itda/vault/master-key-<hash>`. Each test removes its key at the end with
`security delete-generic-password` on macOS; a run that crashed leaves one
behind, which Keychain Access shows under that name.

## What each service taught the product

- Gitea, Redmine and the intranet answer a change with a redirect to the
  result: a change answered with 3xx is now reported as landed, with the
  address, instead of `request_failed`.
- Redmine keeps the CSRF token in a session cookie it rewrites on every
  answer: the cookies a preflight step sets now go along with the change.
- WordPress redirects to `wp-login.php?...&reauth=1`, an address itda will
  not follow because of the credential-named parameter: such a redirect is
  now answered as the redirect it is, so setup can name the sign-in page.
- MediaWiki answers a private wiki's pages with HTTP 200 and a "Login
  required" page that links to the sign-in: setup now follows that link when
  the page has that title and no records. Its API answers errors with HTTP
  200 too, so a signed-out read shows up as `response_mismatch` rather than
  `auth_required`; `itda check` still says the site changed.
- WordPress's REST API needs a nonce with the session, which `/wp-admin/admin-ajax.php?action=rest-nonce`
  hands out as text: a preflight step reads it with `css:body`. The lab's
  WordPress has plain permalinks, so the API is at `/?rest_route=...`.

## What itda does on each, measured

Run on this machine against the lab above, with the itda binary built from this
branch (frame following and query sessions, `b35a612`, are in it). These three
are the uglier ones, alongside the five earlier services.

- **framed (8086).** `itda setup http://127.0.0.1:8086/` drafts one read,
  `home`, of the shell's `<h1>` ("사내 인트라넷") and nothing else. Frame following
  reaches a shell's own frames. `itda draft page http://127.0.0.1:8086/` sees
  the iframe and says so ("`/`: shows /frame/board in a frame that was not
  read"). The frame answers `303` to `/frame/login`, a redirect itda will not
  follow because it is a credential path. Setup does not sign in from a shell
  whose frame wants a login, so it still never reaches the board two iframes
  down. Pointed at the frame instead,
  `itda setup http://127.0.0.1:8086/frame/list --user lab` sees the redirect to
  the sign-in and signs in through the form (2 cookies). It drafts the board the
  same way as the intranet: a list of ten records, a per-notice read, and a post
  with a `_csrf` preflight. The reviewed `specs/framed.yaml` targets the frame.
  A read returns the notices (25 across three pages, ten per page). A post waits
  for approval, then lands once (a 303 to the new notice, reported as landed).
  itda refuses a second call with the same approval; `itda audit --verify` and
  `itda check` pass. Signing in from the shell's own frame is left for a
  follow-up.
- **scripted (8087).** `itda setup http://127.0.0.1:8087/board --watch-only`
  opens a browser and watches the page load (2 calls, 1 read drafted). It drafts
  `list-notices` from the script's own `GET /api/notices` call; the read then
  returns records. No post happens while itda only watches, so the reviewed
  `specs/scripted.yaml` adds it by hand: `POST /api/notices` with the session
  cookie and the `X-CSRF-Token` read from `<meta name="csrf-token">` in a
  preflight step. With that, a post waits for approval and lands once (HTTP 201
  with the created notice). `itda audit --verify` and `itda check` pass. This
  case passes today, through the automatic browser watch.
- **urltoken (8088).** `itda setup http://127.0.0.1:8088/notices --user lab`
  sees the redirect to the sign-in and signs in through the form. The site
  redirects to `/notices?sid=<token>` and sets no cookie. itda reads `sid` from
  the redirect, drafts a `kind: browser` session with `value_from: query:sid`,
  and removes `sid` from the operations' inputs. The drafted `list-notices` then
  returns ten records, each `link` showing `?sid=[private]` (itda redacts the
  value). The reviewed `specs/urltoken.yaml` adds a per-notice read and names
  the write `post-notice`. A read returns the notices. A post waits for
  approval, then lands once (a 303 to the new notice at
  `/notices/{id}?sid=[private]`, reported as landed). itda refuses a second call
  with the same approval. The `sid` never appears in the audit log;
  `itda audit --verify` and `itda check` pass.

The test suite below encodes these outcomes. The framed, scripted and urltoken
cases run to an approved change, and it checks the browser watch and the
url-token setup too. The one the product cannot do today, signing in from a
shell whose frame redirects to a login, is written for the behaviour we want and
marked `#[ignore]` with that reason.

## What it does not cover

Single sign-on and one-time codes still need `itda login --browser`. Drafted
`GET` form reads (a search box) carry no response check, so the reviewed
definitions add one. The WordPress post list is drafted as a change because
its path says `edit.php`; the reviewed definition reads posts through the
REST API instead.
