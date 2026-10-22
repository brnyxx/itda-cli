# The intranet lab

Self-hosted software behind a sign-in, so itda's sign-in paths are tested
against real software rather than fixtures: Gitea, Redmine, WordPress,
MediaWiki and a small legacy intranet written for the lab. Every public-site
number itda reports is measured; this is where the numbers for "the old
intranet behind a login" come from.

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

## What it does not cover

Single sign-on and one-time codes still need `itda login --browser`. Drafted
`GET` form reads (a search box) carry no response check, so the reviewed
definitions add one. The WordPress post list is drafted as a change because
its path says `edit.php`; the reviewed definition reads posts through the
REST API instead.
