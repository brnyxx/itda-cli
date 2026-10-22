# Honest client policy

itda acts for a person, on websites that did not build it an API. That only
works long term if site operators can see who is calling and can say no. These
rules are fixed. Pull requests that weaken them are declined.

## What itda does

| Rule | Where it is enforced today |
|---|---|
| Every request identifies itself as `itda/<version>`. | `itda-net`: `USER_AGENT` |
| No browser imitation: no browser user agent, no `sec-ch-ua` or `sec-fetch-*` headers. Besides `User-Agent`, a request carries a plain `Accept` (HTML, or JSON for API paths), `Accept-Language: *` (no preference), a same-origin `Referer` for API calls, signed-in requests and changes, an `Origin` for changes, and the headers its definition declares. | `itda-net` default headers; `itda-policy` refuses `sec-*` headers in definitions |
| A block, challenge or CAPTCHA pauses the whole site, for every account, until the user resumes it. | `itda-net` `Gate` (`blocked_by_site`) |
| A signed-in request answered with 401 pauses only that account (`auth_required`). | `itda-net` `Gate` |
| One request at a time per site, about 1.2 seconds apart (a definition may ask for more with `defaults.interval`), shared by every itda process on the same data directory, during discovery as well as for actions. | `itda-net` `Gate` |
| A rate limit or `Retry-After` sets a cooldown. A read waits out a short one (ten seconds or less) once and tries again; a longer one is reported with the time the site asked for. Budgets cap requests per scope. | `itda-net` `Gate`, `Network::fetch` |
| Credentials go only to the origin or site they belong to; redirects never carry them elsewhere. | `itda-policy` `validate_spec`, `itda-net` redirect checks |
| Changes need a single-use approval bound to the exact inputs. Reads are the default. | `itda-policy` classification and `Approvals`, `itda-runtime` |
| A change is sent once: no automatic retries or redirects. | `itda-runtime` write path |
| Every call, approval and change is recorded in a hash-chained log on the user's device. | `itda-runtime` `audit` |
| The browser window itda opens for sign-in, recording or watching a page load is the person's own: itda opens the page they named, watches, scrolls once to the bottom when it watches a load by itself, and never clicks, types or navigates. | `itda-browser` `SignIn`, `Recorder` |
| A sign-in through a site's own form is the person's: they type the user name and password, itda sends the form once, as `itda/<version>`, to the form's own action on the same site, with the fields the page gave. A refused sign-in is reported, never retried, and nothing is paused. The password is never kept; an agent cannot start this path. | `itda-setup` `login::form` |

One step is treated differently: while `itda setup` tries each read of a new
site once, with the person watching, a read the site blocks or challenges is
dropped from the draft with a line saying the site does not give itda that
page, and nothing more is sent to the site in that run. The site is not left
paused, because nothing was retried: the remaining reads wait for the next
`itda check`, and from then on a block or challenge pauses the site as above.

## What itda will not do

- Solve or route around CAPTCHAs, challenges or bot managers.
- Retry a blocked request through a browser, a headless browser or another
  network path.
- Spoof TLS fingerprints, client hints or another client's identity.
- Resume a paused site on its own.

A SiteSpec may set a different `User-Agent` in `defaults.headers` when a site
asks for one. That choice is visible in the spec and in review.

## For site operators

If you see `itda/<version>` in your logs, a person is using itda on their own
machine with their own account. Blocking or challenging those requests stops
them for that person until they choose to resume, and they are told your site
refused the request. A contact page for operators will be linked from the user
agent once the project has a stable home.
