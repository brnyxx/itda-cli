# Honest client policy

itda acts for a person, on websites that did not build it an API. That only
works long term if site operators can see who is calling and can say no. These
rules are fixed. Pull requests that weaken them are declined.

## What itda does

| Rule | Where it is enforced today |
|---|---|
| Every request identifies itself as `itda/<version>`. | `itda-net`: `USER_AGENT` |
| No browser imitation: no browser user agent, no `sec-ch-ua` or `sec-fetch-*` headers. | `itda-net` default headers; `itda-policy` refuses `sec-*` headers in derived credentials |
| A block, challenge or CAPTCHA pauses the whole site, for every account, until the user resumes it. | `itda-net` `Gate` (`blocked_by_site`) |
| A signed-in request answered with 401 pauses only that account (`auth_required`). | `itda-net` `Gate` |
| A rate limit or `Retry-After` sets a cooldown; requests in one scope are paced and budgeted. | `itda-net` `Gate` |
| Credentials go only to the origin or site they belong to; redirects never carry them elsewhere. | `itda-policy` `validate_spec`, `itda-net` redirect checks |
| Changes need a single-use approval bound to the exact inputs. Reads are the default. | `itda-policy` classification and `Approvals`, `itda-runtime` |
| A change is sent once: no automatic retries or redirects. | `itda-runtime` write path |
| Every call, approval and change is recorded in a hash-chained log on the user's device. | `itda-runtime` `audit` |

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
