# Transport policy

itda acts for a person, on websites that did not build it an API, with that
person's own account. It reads a site over plain HTTP and, when the site
answers a read with a challenge page, tries that read once more inside the
browser installed on the computer. Moving to the browser is the only thing a
challenge changes: it does not let itda send a change, carry a credential
elsewhere or skip a wait the site asked for.

## How a read recovers

Direct HTTP goes first. When the answer is a challenge page, or does not have
the fields the definition promises, itda runs the same read inside a hidden
window of the installed browser: a fetch from the page's own origin, then, for
a `GET`, the rendered page. An answer counts only when its response check
passes and the data extracts. An error the site itself reports, such as a
GraphQL `errors` list, ends the read instead of moving it on.

itda remembers which way worked for an hour, per site, account and action, so
the next call starts there instead of paying for the failed HTTP try again;
changing an input does not forget it. A way that failed is skipped for a few
minutes (five for direct HTTP, three for the browser). This cache holds no
secrets.

The browser is the one installed. itda downloads neither a browser nor a
captcha solver, and does not solve captchas. Every request in this mode, a
change and a sign-in form included, carries that browser's user agent, over
HTTP too, and the hidden window keeps the browser's own client hints. The site
sees one client whichever way a request goes, so a session or a challenge
clearance the browser earned also holds for the direct requests. itda does not
copy the browser's TLS fingerprint, and nothing here guarantees that a
challenge passes.

Each site and account gets its own browser session, isolated from every other.
A worker process starts on first need, keeps up to four sessions warm across
CLI calls and exits after five idle minutes; no daemon command is needed. Reads
for different sites or accounts run at the same time, under the shared pacing;
two reads for the same one take turns. Signing in again, signing out or
removing the site closes its session; changing a query, body or `Accept`
header does not.

Everything the page loads goes through itda's own proxy, so the address checks
of a direct request apply to every resource, frame and worker: private
addresses are refused, other sites get neither the account's credentials nor
the page's address as referrer, and only reads pass, plus a GraphQL query itda
has read and found to be one (no mutation, subscription, batch, or document it
cannot tell). A browser answer is capped at 16 MiB, and the read's own byte and
time limits still apply. After each read the page and its workers are stopped
and the proxy drained; the session stays warm.

A `429` or `Retry-After` from one resource the page loads cools down that
resource's site, across CLI calls, without throwing away the page's data when
the resource was optional. A read that cannot be satisfied during that wait
fails with `retry_after`, `upstream_origin` and `paused: false`.

If the worker dies mid-read, itda notices (its ownership lock, or a new worker
answering) and tries that read once more within the original deadline. A small
supervisor process closes a browser whose worker died, even before the browser
has reported its DevTools port. While itda does not know whether a read ended,
it keeps the account's turn rather than start a second read next to it.

`ITDA_TRANSPORT=http` keeps every request on direct HTTP with the `itda/<version>`
user agent; a block or challenge then pauses the site until `itda resume`, as
before. `ITDA_TRANSPORT=auto`, or leaving it unset, is the behaviour above.

## Boundaries

| Rule | Enforcement |
|---|---|
| Only a read may move to the browser on its own. A change goes once, over direct HTTP, after a single-use approval bound to its exact inputs, with no retry and no redirect. | `itda-net` read recovery; `itda-policy` classification and `Approvals`; `itda-runtime` write path |
| In direct-HTTP mode every request identifies itself as `itda/<version>`. In adaptive mode every request carries the installed browser's user agent instead, so the direct requests and the browser look the same to the site. A definition cannot set `sec-*` headers; the transport owns them. | `itda-net` default headers; `itda-policy` refuses `sec-*` headers in definitions |
| Credentials go only to the site and account they belong to. A redirect never carries them elsewhere, and a page in the browser cannot send them, or its address as referrer, to another origin. | `itda-policy` `validate_spec`; `itda-net` redirect checks; `itda-browser` request interception |
| Private addresses need explicit permission, for a direct request and for everything the browser loads. | `itda-types::url`; `itda-net` resolve; `itda-browser` proxy |
| itda processes on the same data directory share one limit for discovery and actions: one request at a time per site, about 1.2 seconds apart, whichever way the request goes. A definition may ask for more with `defaults.interval`. A read waits out a short `Retry-After` once; a longer one is reported with the time the site asked for. | `itda-net` `Gate` |
| The gate re-reads the request it is about to send, not only the declared inputs: the rendered address and body, and any GraphQL document in them. A declared read that turns out to change the site goes to the user for approval with the reason; one whose effect cannot be told is not sent (`effect_unknown`). A redirect or next-page address carrying a GraphQL mutation is not followed. Headers that rewrite the method or the address (`X-HTTP-Method-Override`, `X-Original-URL`, `X-Rewrite-URL`, ...) are refused. | `itda-policy` `rendered_request_effect`, `safe_read_url`, `overrides_method`; `itda-runtime` |
| A signed-in request answered with 401 pauses that account (`auth_required`); it does not move to the browser. | `itda-net` `Gate` |
| A read the browser did not get past fails with `blocked_by_site` and `paused: false`. Nothing is paused; only that way of fetching is skipped for a few minutes. In direct-HTTP mode (`ITDA_TRANSPORT=http`) a block or challenge pauses the whole site until the user resumes it. | `itda-net` |
| itda does not solve captchas and never downloads a browser; it uses the one installed. | `itda-browser` |
| itda records every call, approval and change in a hash-chained log on the user's device, and hands website text to agents as untrusted data. | `itda-runtime` `audit`; `itda-mcp` |
| The knowledge base does not choose how a request is sent: no transport, header, retry or account choice reads it. | `itda-kb` `Recorder` (write only); `itda-runtime` |
| The window itda opens for sign-in, recording or watching a page load is the person's own. itda opens the named page and watches, scrolling to the bottom once when watching a load by itself, but never clicking, typing or navigating. The window a read recovers in is hidden and never shown. | `itda-browser` `SignIn`, `Recorder`, `transport` |
| Sign-in through a site's own form belongs to the person: they type the user name and password. itda sends the form once, with the same user agent as its other requests, to its own action on the same site, with the page's fields. itda reports a refused sign-in without retrying or pausing anything. itda never keeps the password; an agent cannot start this path. | `itda-setup` `login::form` |

In `itda setup`, a read the site still refuses is dropped from the draft with a
line saying so, and nothing more is sent to the site in that run. Recovery
never changes an action's inputs, whether it counts as a read or a change, or
what it needs approving. An agent's preparation loop (`itda_prepare`) checks
only reads, through the same path a call takes, and counts each request,
recovery included; its advice never names a way around a refusal: a block
means pause and tell the person.

A definition may set a different `User-Agent` in `defaults.headers` when a site
asks for one; that choice is visible in the spec and in review. It cannot set
`sec-*` headers: the transport owns those.

## After a pause

A site pauses only in direct-HTTP mode, or for a sign-in the site no longer
accepts. Check the site in your browser, then run `itda resume <site>`: that
clears the site's pauses and the ways of fetching it remembered, and closes the
browser session for that site and account. Sign in again only when the
account's session was rejected (`auth_required`), not when an API refused one
request (`api_denied`). Adaptive mode clears an old challenge pause for the
site and account on its own; it never clears a sign-in pause or a rate-limit
cooldown.
