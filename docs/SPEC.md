# Writing a site definition

A site definition (`spec.yaml`) tells itda a website's actions, what each sends,
what a correct answer looks like and whether it changes anything. itda reads
`<data dir>/specs/<site>/spec.yaml`. Beside it, `check.json` stores the last
`itda check` time and verdict; `sources.json` stores a drafted definition's
sources (see [Keep it working](#keep-it-working)). `itda add spec.yaml` copies
it there after a short review. `itda doctor` prints the data directory;
`ITDA_HOME` moves it.

## Start from an OpenAPI document

When the site publishes an OpenAPI 3 or Swagger 2.0 description, let itda
draft the definition:

```sh
itda draft openapi openapi.json > spec.yaml            # print the draft
itda draft openapi openapi.json --include orders       # only matching paths, ids or tags
itda draft openapi openapi.json --base-url https://api.example.com --install
```

The draft keeps each operation's source in `evidence` and maps `GET` to reads,
all else to changes. It adds response checks from required fields in the success
schema and maps header API keys and bearer tokens to `auth`. itda lists what it
cannot translate (OAuth, cookie keys, header parameters, multipart bodies,
deprecated operations) on stderr without guessing. Review the draft before
agents use it: shorten descriptions, remove actions agents should not call, and
tighten the response checks.

## Start from the pages you use

For a site with no API description, and pages you can open without signing
in, give itda the addresses of the pages an agent should read:

```sh
itda draft page https://example.com/board > spec.yaml      # print the draft
itda draft page https://example.com/board https://example.com/board/42
itda draft page https://example.com/board --name board --install
```

itda fetches each page under the same rules as every other read: once over
HTTP and, when the page answers with a challenge, once more inside the browser
installed on the computer (see [HONEST_CLIENT.md](HONEST_CLIENT.md)). Private
addresses need `ITDA_ALLOW_LOCAL=1`. If the page still refuses, the draft stops
with what the gate saw, as `itda call` would report it: `blocked_by_site` (exit
12) naming Cloudflare, a bot manager or a captcha, or `site_rate_limited` (exit
13) with the wait the site asked for; nothing is paused. A page that sends itda
on to a sign-in page stops with `auth_required` (exit 10) and points at `itda
record`. itda does not solve a captcha; the site's official API, or its owner,
is the way in. Each page that answers becomes a read
with a response check, so `itda check` notices when the page changes:

| What the page has | What the draft gets |
|---|---|
| A table with a header row | `html.items` per row, one field per column named after its header, plus `link` |
| A list of similar elements with a title and a link (cards, posts, `tr.athing`) | `html.items` with `title`, `link` and, when present, `id`, `summary`, `meta`, `published_at`, plus fields most records share at one location under a site-named class (`score`, `author`, `release__version` becomes `version`). Build names such as `heading__fzQskI` are left out. |
| Three or more consecutive siblings of the same shape, outside navigation, sidebars, headers and footers, leaving out ad slots (an element whose class, id or label marks it as an ad: `AdHolder`, `ad-*`, `sponsored`, `aria-label="Sponsored"`). Each has a title in the same place: a link (`ol.stories > li.story`, `#articles > dt`, a `dl` per section), a heading, text named as a title (`span.result__title`), or the record itself if it is a link with content (`a.release`). | The same, reading title and link from the title's own path in the page and date-like text as `published_at`. A heading or a link spanning the whole record needs one more field. Bare links, menus and paragraphs never qualify. If the page names its records, itda uses that name to describe the read (`List the projects on Projects`) |
| A list of record links sharing one shape: one path with a different segment per record (`/item/{id}`), or one path with one varying query key (`view.php?id=`) | A one-record read (`get-project`, `get-post`) with a required `id` input (`int` for numeric ids). Query keys every link sends become inputs with defaults. No response check yet: run it once, then declare what the page shows. |
| One thing (an article, a package, a product, a bug): an `h1` outside the site's header and menus, beside a body of text, a date, or two or more labelled values. It is outside any record list, or the list takes less than a quarter of the main block. | `html.page` fields across the page, each matching once: required `headline`; `body` (the longest named body block, `div.project-description`, `#readme`, `[itemprop=articleBody]`, else the block with most text); fields beside the headline under site-named classes (`project-header__summary` becomes `summary`); `description` from page meta; `published_at`; `author`. Each `dt`/`dd` pair or `th`/`td` row adds a field named after its label. A page that is one thing with a named list of five or more records on it (a package with its releases) gets both reads |
| A list in data embedded for scripts: an `ItemList` or a list of one type (`NewsArticle`, `Product`...) in `ld+json`. In `__NEXT_DATA__`, `__NEXT_F__`, `__APOLLO_STATE__` (grouped by typename), `__PRELOADED_STATE__` and similar data, choose the object list with the most to read (a title, a link, a date), then the longest among ties. | One read per list, named for its contents (`list-news-articles`), with `ssr_json`, `extract_pointer` to the list and `required_pointers` on its first record. Its `model` keeps up to eight plain fields per record, ids and titles first, excluding credentials. |
| Data embedded for its scripts with no list in it | `ssr_json` with `required_pointers` on the whole payload, and a note saying what to add |
| Only a title | `required_html_fields` on its `h1` or `title`, and a note saying what to add |
| Links to `?page=2` on the same path, or a link marked `rel="next"` with a number in its query | An optional query input named after that key (`int`, no default, so the read asks for the address it was drafted from until an agent or a page handle asks for another; counting starts at 1 for `page` and the like, at 0 for `offset` and `start`). A key the page's own address carries keeps its value. The reads that return records also get `pagination` on that key (`style: page`, or `style: offset` for a key that starts at 0) |
| A `GET` form | A read with the form's fields as `query` inputs |
| A `POST` form | A change with the fields as `body.form`, using `auth: session`; a hidden token such as `_csrf` becomes a `preflight` step read from the form page |
| A `POST` form with `enctype=multipart/form-data` and a file control | A change with the fields as `body.multipart.fields` and the file control as `body.multipart.file`, with an input of its own for the path of the file to send |
| Links the page labels as attachments: a `download` attribute, the words download, attachment, 첨부 or 다운로드 in the link's text, title, label or class, or an icon named for one | A file read (`download-file`) with `response: {format: file}`. Links of one shape become one read with an input (`/files/{file_id}`, or a varying query key); `media_types` lists the kinds the link names say, or the usual document types when they say nothing. A link to a document the page does not call an attachment stays a page |
| A sign-in form | Left out; `itda setup --user` and `itda login <site>` sign in through it, see `auth` below |
| `<link rel="alternate">` to RSS or Atom | A read with `format: xml` |
| A link to an OpenAPI document, in the page's `Link` header or a `<link>` or `<a>` (`rel="service-desc"`, an API catalog with `rel="api-catalog"`, `rel="describedby"`, or an `href` ending in `openapi.json`, `openapi.yaml` or `swagger.json`), or, when nothing is linked, `/openapi.json` on the same origin answering with one | Its reads join the draft as `itda draft openapi` makes them, under the page's address. Its changes appear as notes. See below. |

When a site also publishes an API description, itda reads it: the one the page
points at, else `/openapi.json` on the same origin when it answers with an
OpenAPI 3 or Swagger 2.0 document. It makes one GET as itda, at most 4 MB. Every
request to a site waits its turn, so no other path is tried. Where the page
points comes from the HTTP `Link` header the first page was answered with
(RFC 8288) and from the `<link>` and `<a>` elements of the pages, which costs
no request. The best wins: `service-desc` (RFC 8631) or a description file
name, then `api-catalog` (RFC 9727), then `describedby`; on a tie the header
comes first. A `service-desc` document is read even when it does not parse,
so the draft can say so. An API catalog is a linkset (RFC 9264,
`application/linkset+json`): itda follows its `service-desc` links, up to
three, and takes the first OpenAPI document; a catalog that only points at
further catalogs is not followed. A `describedby` document counts only when it
is an OpenAPI document; otherwise `/openapi.json` is still tried. Its reads become
actions as `itda draft openapi` makes them. Each path is drafted once, whether
the page or description named it. The changes it lists are notes rather than
actions (one each up to ten, then a count); `itda draft openapi <url>` drafts
those for review. itda omits and lists reads with credential inputs, such as an
`apikey` query parameter. Install-time checks would refuse them. itda only notes
descriptions with another server address; it does not merge them. A description
that does not parse leaves the page draft unchanged. The header comment and
every merged action's `evidence` name the description's address.

Field and input names face the same checks as a hand-written definition: itda
omits and lists names that look like credentials. A hidden form field other than
a token keeps its value as a default: the form always sends it. itda copies
nothing else from the page. A script-drawn page shows little to a plain fetch.
`itda draft page` reports this and points to recording the site; `itda setup`
first watches the page load in your browser (below).

A fetched page is a shell when its main content is one or more same-origin
`<iframe src>` (or an older `<frameset>`'s `<frame src>`) and it has no records,
data, form or feed of its own. itda then fetches each frame's page as a read, on
the same pacing and with the same session, and drafts from the frame pages as
pages of the site. Each frame read keeps the frame's own address as its
operation path, since that is what a request to the server looks like. A form
inside a frame posts to the frame's action like any other. itda follows at most
four frames, at most two deep. itda lists a frame on another origin in the notes
and never fetches it. The same goes for a same-origin frame that answered with
nothing. `itda draft page` and `itda setup` say what they followed ("Followed 1
frame: /frame/board.").

When a same-origin frame answers with a redirect to a sign-in page (the same
detection the top-level page uses), itda treats it as if the top-level page
had: following the redirect to read the login form is a read, so with
`itda setup --user` it signs in through that form and reads the frame again
with the session, then drafts from it. `itda draft page`, which does not sign
in, notes that the frame leads to a sign-in rather than only that it was not
read.

## Start from the software a site runs

Most intranets run software everyone knows, and that software has a stable API
worth more than a reading of one page. `itda setup` recognises eight of them
from page HTML (a `generator` meta, a well-known asset path, an Atlassian
context-path meta, a body class). It confirms the guess with exactly one GET to
the software's own endpoint, as itda, through the same gate as other requests.
No fingerprint, no request. A confirmed match says so in one line, "This looks
like GitLab. Using its API at <base>.", and drafts from the software's template
under
[`sites/software/`](https://github.com/brnyxx/itda-cli/tree/main/sites/software):

| Software | Recognised by | Confirmed with | API base | Signs in with |
|---|---|---|---|---|
| GitLab | `og:site_name` GitLab, `gl-*` body classes, `/assets/webpack/` scripts (their prefix is the relative URL root) | `GET /api/v4/projects?per_page=1` | `<root>/api/v4` | `PRIVATE-TOKEN` |
| Jira (Data Center, Cloud) | `application-name` JIRA, `ajs-context-path` | `GET /rest/api/2/serverInfo`, whose `baseUrl` becomes the base | `<baseUrl>` | the `Authorization` header you store: `Bearer <PAT>` or `Basic <email:token>` |
| Confluence | `confluence-base-url` / `confluence-context-path` metas | `GET /rest/api/space?limit=1` | `<context>` | the same |
| Redmine | `controller-*` and `action-*` body classes with `/stylesheets/application.css` (its prefix is the sub-directory) | `GET /projects.json?limit=1` | `<root>` | `X-Redmine-API-Key` |
| Discourse | `generator` Discourse | `GET /site/basic-info.json` | origin | reads only (posting needs two headers a definition cannot send yet) |
| MediaWiki | `generator` MediaWiki or the `mediawiki` body class; `EditURI` names `api.php` | `GET api.php?action=query&meta=siteinfo` | the script path (`/w` on Wikipedia) | the browser session, for editing |
| WordPress | `<link rel="https://api.w.org/">`, else `generator` WordPress | `GET <wp-json>/` listing `wp/v2` | the linked API root | an application password, as basic auth |
| Gitea, Forgejo | their default `keywords` meta; `appSubUrl` in `window.config` | `GET /api/v1/version` (a `+gitea-` version is Forgejo) | `<sub>/api/v1` | `Authorization: token` |

Each template holds eight to fourteen reads an agent needs most (lists, search,
one item), up to three marked changes, record models and the software's sign-in
method. Every operation has its documentation in `evidence`. itda names the site
after the host (`issues.apache.org` becomes `issues-apache`; `--name`
overrides). The template's `store:<software>/...` secrets follow that name. itda
tries every read once, as on any site, and removes reads the instance does not
answer (a private project list, a disabled module). The page draft still runs.
Its lists, forms and feeds join the template under the page's address if it
differs from the API's. Each method and path is drafted once. A page that is not
enough on its own, because it needs a sign-in or is drawn by scripts, still gets
the template when the software is confirmed. If the page says WordPress but
`/wp-json/` does not answer like WordPress, itda says nothing. It drafts the
page like any other.

`itda setup --as <software>` forces a template, using the fingerprint's base if
the page agrees, else the API under the page's origin. The confirm request still
runs; the line says if it got no answer. `--no-template` reads the page like any
other. `itda check` and `itda refresh` work on the result as on any site. A
refresh fills the same template at the same API base and merges the page's reads
again. Over MCP, `itda_setup` does the same. It takes `template` (a name or
`none`) and names the template used in its result. `itda add` does not install
the templates: setup fills their placeholder `base_url`.

## Sites with a ready-made definition

A page on `pypi.org`, `github.com`, `www.npmjs.com` (or `npmjs.com`) or
`crates.io` gets the site's `itda add` definition (`pypi`, `github`, `npm`,
`crates-io`). It reads the site's own API instead of its pages. `itda setup`
says so in one line, "This is PyPI. Using its JSON API.", and does not fetch the
page. It saves the definition under its own name and tries its reads once as on
any site. `--no-template` reads the page like any other; `--as <software>` uses
that template instead. Over MCP, `itda_setup` takes the same path, with
`template: none` to skip it, and puts the line in its notes. A site set up this
way has no sources to redraft from, so `itda refresh` does not apply.
`itda add <site> --force` installs the current definition.

## When setup tries the reads

`itda setup` tries every read once while you watch, over HTTP and then, when
the site answers with a challenge, once more in the installed browser. A read
the site still blocks or challenges (a 403 from a bot manager, a captcha, a 200
page titled as a challenge) is a finding of the setup, not a refusal to
remember: that read is left out with one line, "<path>: the site does not give
itda this page", and nothing more is sent to the site in that run. The reads after it are kept and
shown as not tried, so the next `itda check` tries them. The site is not paused,
since nothing was retried. itda installs reads that already answered.
`itda check`, `itda call` and the MCP tools on an installed site try a
challenged read in the browser as well; only in direct-HTTP mode
(`ITDA_TRANSPORT=http`) does a block pause the site until you resume it.

## Start from a browser recording

Most internal and older sites publish no API description, but their pages
still call one. Record yourself using the site and let itda draft from the
recording:

```sh
itda record https://intranet.example.com/board                  # print the draft
itda record https://intranet.example.com/board --name board --install
itda record https://intranet.example.com/board --save session.har   # keep the recording too
```

itda opens Chrome, Edge or Chromium (or `ITDA_BROWSER`) in a fresh, empty
profile at that page and watches, through the browser's own DevTools protocol,
what every page asks the server for. Use the site the way an agent should: sign
in, search, open an item, submit a form. Press Enter in the terminal or close
the window when you are done. itda never clicks, types or navigates in that
window; it opens only the page you named. The recording stays in memory unless
`--save` writes it to a file. That file holds your cookies and tokens; delete it
afterwards. With `--install`, itda offers to keep the recording's sign-in token
for a drafted header `auth` entry and session for a `browser` entry. Each needs
a yes in the terminal. They go to the system keyring under `<site>/default` and
never appear on screen. You need `itda login` only when a token expires.

`itda setup` runs a shorter version automatically when a page shows itda only a
title and has scripts that could fetch data: a script file, preloaded module, or
inline script that imports a module or mentions `fetch`, `XMLHttpRequest`,
`graphql` or `api`. Without these, the page goes straight to the question below.
It opens the same kind of window on the page, waits for two seconds of network
silence (ten at most), scrolls to the bottom once and waits again. It closes the
window and drafts from what the page loaded on its own, like `itda draft har`.
It tries the page's own address first, so what the page itself shows is never
lost to a busier API elsewhere. It reports one line
(`Watched the page load: 12 calls, 3 reads drafted.`, counting reads beyond a
title check). It asks you to record only if it drafted no such read. It never
clicks or types in the window or keeps sign-in data, since nobody signed in. A
page whose scripts lead to sign-in stops with `auth_required`, as a redirect
does. `--no-record` skips the window as well, `--watch-only` watches but never
asks you to record. itda uses the template for a site with recognised software
(above) and does not watch it.

A browser developer-tools recording works the same way. Open the Network tab,
use the site, save with "Save all as HAR" (Chrome, Edge) or "Save All As HAR"
(Firefox), and draft from the file:

```sh
itda draft har session.har > spec.yaml                  # print the draft
itda draft har session.har --base-url https://intranet.example.com/api
itda draft har session.har --name board --install
```

A GraphQL page sends every call to one path; each recorded operation becomes an
action named after it (`exploreProjects` is `explore-projects`), with queries as
reads and mutations as changes. The document and operation name are fixed
inputs; agents fill in variables, which a `json_template` renders into the same
JSON the page sent. itda lists variables holding objects or lists for you to add
by hand.

A GraphQL endpoint can also say what else it offers. When a draft finds an
endpoint in a recorded call or a page naming a same-origin `/graphql` path, itda
sends the standard introspection query once, as a read with a size cap. It drafts one read per root `Query` field with scalar, enum or id
arguments: the field name is the action (`countries`), arguments are typed
inputs (required if the schema says so), and the selection contains up to 24
scalar fields, two levels deep. itda omits and lists optional object or list
arguments, so it can still read the field without them. It lists fields
requiring an object, list or credential instead of drafting them. An argument
named `query` becomes the input `<field>_query`, since `query` holds the
document. itda lists `Mutation` fields in notes, never drafting them as changes:
a schema says what a change takes, not what it does. itda drafts sites with
introspection off from their pages and calls alone, with no error. This is the
one request `itda draft har` makes on its own.

In both cases, itda also drafts the pages you saw, as `itda draft page` does.
Sites rendering lists and forms on the server get actions even when their pages
call no API. A recording holds real tokens, cookies and personal data, so the
draft keeps only the shape of each call: method, path, input names and types,
and the response fields every sample returned. Numbers and ids in paths become
placeholders (`/notices/42` becomes `/notices/{notice_id}`). itda marks inputs
every sample sent as required. Token headers become `auth` entries whose values
you add with `itda login`. Calls with a sign-in cookie (by name, such as
`JSESSIONID` or `SID`) but no token get a `kind: browser` entry.
`itda login <site> --browser` signs them in. A form token such as `_csrf`
becomes a `preflight` step. It reads the token from the form page recorded
before the post, so each change sends a fresh one. A recorded
`multipart/form-data` post whose parts the browser kept (their names, and the
file part's file name, never the file) becomes a change with `body.multipart`;
a recording without the part names lists the call instead. An answer the site
sent as `Content-Disposition: attachment` becomes a file read of the media type
it carried. A header such as
`X-CSRF-Token` gets the same treatment using a `<meta name="csrf-token">` tag on
an earlier page, as Rails, Laravel and many single-page apps do. A session the
site carries in the URL rather than a cookie also gets a `kind: browser` entry,
with `value_from: query:<key>`. itda drafts a query key as the session when the
sign-in's answer hands it out (in the redirect `Location`, or on the links of
the first page after the sign-in), its value looks like a session, and every
signed-in call carries it. Every operation that sent the key gets
`auth: <name>`, and itda drops the key from each operation's inputs. itda keeps
the value in the keyring, never in the definition. At run time, itda adds it to
each request's query, just as it sends a `cookie:` session as a cookie. itda
leaves out the sign-in call: a person signs in, never an agent. itda omits and
lists other token and session fields, since they come from sign-in, never the
agent. itda skips calls to other addresses and failed calls. Delete the HAR
file when you are done with it.

## Write one by hand

This page covers everything version 0.1 runs. The fastest way in is to copy
[`examples/sandbox/spec.yaml`](https://github.com/brnyxx/itda-cli/blob/main/examples/sandbox/spec.yaml) and change it.

## A complete example

```yaml
spec_version: 1
site:
  name: sandbox                 # lower case, digits and '-'; used in <site>.<action>
  title: Sandbox intranet       # shown to people and agents
  base_url: http://127.0.0.1:8787
auth:
  - name: key
    kind: header
    header: X-Api-Key
    value_from: store:sandbox/api-key   # filled by `itda login sandbox`
operations:
  - name: list-notices
    desc: Search notices by title
    method: GET
    path: /api/notices
    effect: read
    evidence: GET /api/notices returns {total, items}
    query:
      q: {type: string}
    response:
      format: json
      required_pointers: [/items, /total]
  - name: create-notice
    desc: Post a notice
    method: POST
    path: /api/notices
    auth: key
    effect: write
    evidence: POST /api/notices creates a notice
    body:
      json:
        title: {type: string, required: true}
```

`itda ls` lists what it found, and a broken definition shows up there and in
`itda doctor` with the problem and how to fix it.

To check a file before installing it, run `itda spec validate spec.yaml`. It
runs the checks `itda add` runs (the structure, the addresses, `confirm` and
the rest) and installs nothing. A valid file exits 0; otherwise it prints one
`problem: <field>: <detail>` line per problem and exits 2, the code of every
definition problem. `--json` prints `{status, file, site, actions, issues}`,
each issue `{code, field, detail}`; a file that does not parse prints the
usual error envelope.

## `site`

| Field | Required | Meaning |
|---|---|---|
| `name` | yes | Id used on the command line and in MCP tool names. |
| `title` | no | Human name, used in tool descriptions and approval prompts. |
| `base_url` | yes | `https://` origin every path is joined to. Local and private addresses also need `ITDA_ALLOW_LOCAL=1`. |

`defaults.headers` adds headers to every request (for example an `Accept`
language). Browser-controlled headers (`sec-*`, client hints) belong to the
transport and cannot be supplied by a definition;
see [HONEST_CLIENT.md](HONEST_CLIENT.md).

`defaults.interval` sets the minimum time between requests to a site, written as
`2s` or `500ms`. itda spaces action-request admissions to an origin about 1.2 seconds
apart by default; a definition can increase this to 30 seconds but cannot reduce
it. Use it when the site's terms name a rate, or when `itda check` keeps running
into `site_rate_limited`.

```yaml
defaults:
  interval: 2s
```

## `operations`

Each operation is one action, called as `itda call <site>.<name>`.

| Field | Meaning |
|---|---|
| `name` | Action id: lower case, digits and `-`. |
| `desc` | One line an agent reads to decide whether to use it. Write it as an instruction: "Search notices by title". |
| `method` | `GET`, `POST`, `PUT`, `PATCH` or `DELETE`. Defaults to `GET`. |
| `path` | Path under `base_url`. itda replaces `{name}` with a path input. |
| `effect` | `read` or `write`. See below. |
| `evidence` | Why you believe the effect is right: a doc link, a captured request. Reviewers read it. |
| `auth` | Name of an `auth` entry, for actions that need an account. |
| `params` | Path inputs, all required. |
| `query` | Query string inputs. |
| `body` | Request body inputs: `json:` or `form:` fields, a `json_template`, or `multipart:` for a form that sends a file. See below. |
| `headers` | Extra headers for this action only. |
| `response` | What a correct answer looks like. Strongly recommended for reads. |
| `model` | Name of an entry in `models` that picks and renames fields from the JSON. |
| `pagination` | How a read that lists things goes on to its next page. See below. |
| `confirm` | For a change: the read that shows it landed, run once when the change's answer is lost. See below. |

### Inputs

```yaml
query:
  q:     {type: string}
  page:  {type: int, default: 1}
  draft: {type: bool, required: true}
```

`type` is `string`, `int`, `float` or `bool`. itda checks inputs before sending
anything; agents see them as a JSON Schema. itda refuses inputs with names
resembling credentials (`password`, `token`, `api_key`…): those go in `auth`,
never through the agent.

### Sending a file

A form that takes a file is a `multipart/form-data` body. `fields` are its
text inputs, declared like `form` fields; `file` names the form field the file
goes in and the input (`from`) that carries the path of a file on this
computer:

```yaml
operations:
  - name: attach-file
    desc: Attach a file to a notice
    method: POST
    path: /board/{id}/attach
    auth: session
    effect: write
    evidence: the attach form on /board/42 posts multipart/form-data
    params:
      id: {type: int}
    body:
      multipart:
        fields:
          memo: {type: string}
        file: {name: attachment, from: file}
```

The agent gives `file` as a path. itda reads the file once, and the approval
the user sees names it by name, size and SHA-256 (`file  plan.csv (8 bytes,
sha256 3a1f…)`). The approval is for those bytes: other bytes under the same
path withdraw it (`approval_expired`, `reason: file_changed`), and the agent
asks again. itda streams the file from disk in small chunks, checking its size
and modification time before every chunk and its hash at the end. A file that
changes during the send stops it; the call ends with `action_outcome_unknown`
and `reason: file_changed`, like any change whose answer was lost. The audit
log records the hash. An action with a file is always a change, whatever its
method or name says, and its body goes only to the site's own origin. A
multipart body without a `file` sends its `fields` as parts. `multipart` is
one of the three body encodings; declare one of `json`, `form` or `multipart`.

### Read or write

An action is a read only if `effect` is `read`, its method is not `PUT`, `PATCH`
or `DELETE`, and neither name nor path has a change word (`delete`, `send`,
`pay`, `submit`, `logout`…). It must also have no input such as `action` or
`mode` asking for a change, and no mutation in a GraphQL `query` input. A `POST`
can be a read, for example a search endpoint or a GraphQL query. Anything else
is a change. itda sends a change once, without retries, only after the user
approves that exact request with `itda approve`.

When the checks disagree with your `effect: read`, itda treats the action as a
write. Rename the action or path rather than fighting the classifier: a
`GET /logout` really is a change. itda refuses headers and inputs that ask the
site to use another method or address (`X-HTTP-Method-Override`,
`X-Method-Override`, `X-HTTP-Method`, `_method`, `X-Original-URL`,
`X-Rewrite-URL`). It sends the reviewed method to the reviewed address. itda
refuses `transport` other than `http` and `auth` kinds other than `none`,
`header` and `browser`. How a read travels (direct HTTP, or the installed
browser after a challenge) is itda's choice, not the definition's; see
[HONEST_CLIENT.md](HONEST_CLIENT.md). A change always goes over direct HTTP.

Those checks look at what the definition declares. Just before sending, itda
reads the request it built once more: every query key and body field, the
value of a field such as `action` or `mode`, and any GraphQL document in
`query` (the body, or the address of a `GET`). A read whose rendered request
asks for a change, for example a `json_template` whose literal body is a
mutation or carries `{"op": "delete"}` deep inside, is reclassified as a
change: the call returns `approval_required` with `reclassified` naming the
reason (`body names a mutation`, `body asks for a change`, `address names a
mutation`, ...), the review shows the same line, and the user decides as for
any change. A read whose effect itda cannot tell, such as a GraphQL endpoint
sent no document, or a document it cannot read, stops with `effect_unknown`
(exit 2) and a remedy; nothing is sent. A declared write never gets looser
this way. A search whose text happens to contain a change word stays a read:
only keys, intent fields and GraphQL documents are read.

## `response`

A declared response is how itda knows the answer is real data and not a sign-in
page, an error page or a redesigned site.

```yaml
response:
  format: json                       # json | html | text | xml | empty | file
  required_pointers: [/items, /total]  # JSON Pointers that must exist
```

For records inside a wrapper, `extract_pointer` returns only that part.
`required_pointers` and `model` then apply to it:

```yaml
response:
  format: json
  extract_pointer: /items            # {"total_count": 3, "items": [...]} -> [...]
```

For server-rendered pages, extract records with CSS selectors:

```yaml
response:
  format: html
  required_html_fields:
    - {selector: "h1#board"}         # must be on the page
  html:
    items: tr.notice                 # one record per match
    fields:
      id:    {selector: td.id, required: true}
      title: {selector: td.title a, required: true}
      link:  {selector: td.title a, attribute: href, absolute_url: true}
    limit: 100                       # at most this many records
```

When the answer does not match, the call fails with `response_mismatch` (exit
11) instead of returning the wrong page as data.

### Reading a file

A read whose answer is a file to keep, an attachment on a board or a report
to download, declares `format: file` and the media types it accepts:

```yaml
operations:
  - name: get-attachment
    desc: Download an attachment from a notice
    method: GET
    path: /board/files/{file_id}
    effect: read
    evidence: /board/42 links to it as an attachment
    params:
      file_id: {type: int}
    response:
      format: file
      media_types: [application/pdf, application/vnd.ms-excel]
      max_bytes: 20000000            # optional; 64 MiB when left out, 1 GiB at most
      allow_empty: false             # optional; an empty answer is a failure
```

itda streams the answer to a file instead of reading it into memory. The
answer's content type (before any `;`) must be one of `media_types`; an HTML
answer counts only when the site marks it `Content-Disposition: attachment`,
so a sign-in page in place of the file is a failure, not a file. A 206 or an
answer with `Content-Range`, a body shorter than its `Content-Length`, an empty
body (unless `allow_empty`) and anything over `max_bytes` fail too, by name:
`wrong_media_type`, `partial_answer`, `empty_answer`, `too_large` (exit 11, or
30 for `too_large`). Nothing is kept from a failed answer. A good answer is
sealed at its end and gets its name from the site's `Content-Disposition`, else
the address, reduced to plain characters; a name already taken gets `-2`, `-3`.
The file lands under `files/<site>/` in the data directory, or in the folder
`itda call ... --out <dir>` names. The read returns a manifest, never the
bytes:

```json
{"path": "/home/you/.local/share/itda/files/board/minutes.pdf", "bytes": 48213,
 "media_type": "application/pdf", "sha256": "9c1e…"}
```

Over MCP the result carries the same manifest and a `resource_link` to the
file. A file read has no `model`, `pagination`, pointers or extraction, and
`itda check` skips it ("saves a file") rather than fill the folder. `itda
setup` tries it once, through the gate like every read: it reads the answer's
headers and first bytes, counts the read as working when they describe a file
the definition allows (status, media type, size), and drops the rest; nothing
is saved. A transfer
longer than the pace allows keeps its turn at the gate by renewing it; a turn
lost mid-transfer stops it (`request_lease_lost`, exit 30), never retried.

## `auth`

### API keys and tokens

A key or token sent in a header:

```yaml
auth:
  - name: key
    kind: header
    header: Authorization
    prefix: "Bearer "
    value_from: store:<site>/api-key
```

Store the value with `echo "$KEY" | itda login <site>` (OS keyring), or set
`ITDA_SECRET__<SITE>__<ACCOUNT>__<NAME>` for CI and containers. `--account`
picks among several accounts; the default account is `default`. itda sends the
header only to `base_url`'s origin, or to `origin` when you set it.

### Sign in with a password

For a site with its own sign-in form, name the form on the `browser` entry
and itda fills it in for you, as itself:

```yaml
auth:
  - name: session
    kind: browser
    value_from: cookie:JSESSIONID   # the cookie that shows you are signed in
    login:
      path: /login                  # the page with the form; a query is fine
      user: username                # the user name field
      password: password            # the password field
```

```console
$ itda login intranet --user me
    Password for me on intranet (sent once to the site's own sign-in form, never saved):
Signed in. itda sealed the session for intranet/default (2 cookies) under the data directory with a key in the system keyring; agents only see the account id.
```

itda reads the page at `path`, like any other read, and selects the same-site `POST` form
with those two fields. It fills them in and sends the form once, as your browser
would, with the page's hidden fields (a token such as `_csrf`, a return address)
and cookies. A redirect means sign-in succeeded. itda stores the site's reply
cookies as the session, as for a browser sign-in. If the form comes back,
sign-in failed. itda does not retry or pause anything. itda sends the password
only to the form's own action and never keeps it. Without a terminal, the
password is the first line of stdin, so a password manager can pipe it.

`itda setup <page> --user me` does the same if the page needs a sign-in
(redirects to one, answers 401 or 403, or says "log in to see this"). It drafts
from the page as you see it signed in. Every action gets `auth: session`; it
writes the `login` block from the form it found so the command above works
later. An agent never gets this path: `itda_setup` over MCP refuses pages behind
a sign-in and says what you should run.

Some older sites keep the session in the address instead of a cookie, as a
query parameter the sign-in hands out (`?sid=...`). Name that key with
`value_from: query:<key>` in place of the cookie:

```yaml
auth:
  - name: session
    kind: browser
    value_from: query:sid         # the query key the signed-in session rides in
    login:
      path: /login
      user: username
      password: password
```

itda signs in the same way; when the site answers with no session cookie but a
redirect (or a first page) carrying `sid`, that value is the session. itda keeps
it in the keyring like any other. At run time it adds it to the query of every
request to the site, the way it sends a cookie session as a cookie. Because it
travels in URLs, it can reach a server's or a proxy's logs; see
[THREAT_MODEL.md](THREAT_MODEL.md). A `browser` entry carries one or the other,
a cookie or a query key, not both.

### A session a single-page app keeps in storage

Some sites keep no session cookie at all. A single-page app signs you in, puts a
token (often a JWT) in the browser's `localStorage`, and its scripts send it on
every call as `Authorization: Bearer <token>` or a custom header. That is a
`header` credential whose value is a session, so its `value_from` names the
storage key instead of the site's own store:

```yaml
auth:
  - name: session
    kind: header
    header: Authorization
    prefix: "Bearer "
    value_from: storage:access_token   # the localStorage/sessionStorage key
```

When you set the site up or record it, itda reads that one origin's
`localStorage` and `sessionStorage` once, over the DevTools protocol, from the
window you signed in to. It keeps a value only when the recorded calls sent it
as a header; a stored value no request sent is reported and left alone ("the
site keeps a value in storage that no request sent; not used"). The token goes
to the keyring as the session, never into the definition. `itda login <site>
--browser` reads it again from the window at the next sign-in, the way a cookie
session is refreshed. itda reads only the signed-in origin's storage, never
another site's and never a file on disk, and never writes anything into a page.

If the token is a JWT with an `exp` claim, itda records when it expires:
`itda check` reports `signed_out` (not `site_changed`) once it has passed, and
`itda_sites` shows when the sign-in expires. itda never signs in again on its
own; it asks you to. `value_from` on a `header` entry accepts `store:<site>/<key>`
or `storage:<key>`.

When the site redirects after a change (post, then redirect, then get, as older
sites do), the change has landed. Its result carries the redirect address. A
redirect to a sign-in page means the site no longer accepts the session. The
call stops with `auth_required`; `itda login <site>` signs in again.

### Browser sign-in

For single sign-on, a one-time code, or a site whose form itda cannot fill
in:

```yaml
auth:
  - name: session
    kind: browser
    value_from: cookie:JSESSIONID   # optional: the cookie that shows you are signed in
```

Sign in once in your own browser:

```console
$ itda login intranet --browser
Sign in to intranet in the browser window that just opened.
itda continues when the JSESSIONID cookie appears, or press Enter once you are signed in.
Saved the session for intranet/default (3 cookies). It is sealed under the data directory with a key in the system keyring. Agents only see the account id.
```

itda opens Chrome, Edge or Chromium (or `ITDA_BROWSER`) in a fresh, empty
profile at the site's address, or at `--url`. You sign in by hand; itda never
clicks or types on the page. It keeps only the site's unexpired cookies. It
closes the window and deletes the profile. itda sends the session only to
addresses on the same site and removes cookie values from results.

A session is a whole `Cookie` header, too long for some credential stores
(Windows Credential Manager caps a credential at 2560 bytes). itda stores it in
`vault.sealed` under the data directory, encrypted with a system keyring key.
API keys stay in the keyring itself. The move is automatic in both directions,
and `itda doctor` says where secrets are.

When the session expires, the site answers 401, or 403 with a reason such as
`session_expired`. The account pauses with `auth_required`;
`itda login <site> --browser` signs it in again and resumes it. A 403 with any
other reason is `api_denied`: the site did not allow the request. Signing in
again will not help. On a server without a keyring, set
`ITDA_SECRET__<SITE>__<ACCOUNT>__SESSION` to a `Cookie` header value instead.

## `preflight`

Older sites often hand out a one-time value with the form page, such as a CSRF
token in a hidden input, and refuse a change without it. A preflight step reads
that page just before the action and passes the value along:

```yaml
operations:
  - name: post-notice
    method: POST
    path: /board/write.do
    auth: session
    effect: write
    evidence: the write form posts _csrf from /board/writeForm.do
    preflight:
      - name: form
        path: /board/writeForm.do
        extract:
          csrf: "css:input[name=_csrf]@value"
    headers:
      X-CSRF-Token: "{{step.form.csrf}}"           # if the site wants a header
    body:
      form:
        title:   {type: string, required: true}
        _csrf:   {type: string, default: "{{step.form.csrf}}"}   # or a form field
```

| `extract` source | Takes |
|---|---|
| `css:selector` | the text of the one element the selector matches |
| `css:selector@attribute` | that element's attribute |
| `json:/pointer` | a value from a JSON answer |
| `header:name` | a response header |
| `cookie:name` | a cookie the answer sets |
| `ssr:key#/pointer` | a value in the JSON a page embeds for its scripts: `ssr:__NEXT_DATA__#/buildId`, or `ssr:auto#/pointer` for the first payload found. Only a string, number or boolean counts; an object or a list stops the call |

A step is a `GET` to the action's own origin, with the same account, either to
`path` or to a read action named in `op` (with `args`). Steps run in order after
you approve a change, just before itda sends it. This keeps the value fresh.
itda sends cookies from a step's answer with the action, as a browser does, for
that request only; it never keeps them. A Rails site keeps its issued token in a
session cookie it rewrites on every answer. A value is used as
`{{step.<step>.<name>}}`. Agents cannot see or set inputs filled this way. If a
step finds nothing, itda stops the call with `response_mismatch` and sends
nothing. After the steps, itda classifies a read again with the values filled
in. If it now counts as a change, itda stops with `effect_changed` (exit 2)
without sending it.

## `models`

A model picks and renames fields from a JSON answer, so agents get small, stable
records. For a list answer, itda picks each record the same way.
`itda draft page` writes one for every list it finds in a page's embedded data;
`path` reaches into nested objects with dots (`item.headline`):

```yaml
models:
  notice:
    id:    {path: id, type: int}
    title: {path: subject}
    by:    {path: author.name}
operations:
  - name: list-notices
    model: notice
    ...
```

## `pagination`

A read that lists things can say how the site turns pages. The agent can then
read on without seeing the site's own marker:

```yaml
operations:
  - name: search
    method: GET
    path: /api/v1/crates
    effect: read
    evidence: https://crates.io/data-access#api
    query:
      q: {type: string, required: true}
      page: {type: int, default: 1}
    pagination: {style: page, next: /meta/next_page, send: {query: page}}
```

| Field | Meaning |
|---|---|
| `style` | `page` (a page number), `offset` (how many records to skip) or `cursor` (a marker the site hands out). |
| `next` | Where the next marker is in the answer: a JSON pointer such as `/meta/next_cursor` or `/data/notices/pageInfo/endCursor`, or `header:link` for the `rel="next"` address in the `Link` header. A page or offset read may omit it. If the answer has records, the next marker is the next page number or the offset past those records. |
| `more` | A JSON pointer that must hold `true` for another page to exist, such as `/data/notices/pageInfo/hasNextPage`. Optional. |
| `send` | The input the marker goes back in: `{query: page}` or, for a JSON body, `{body: after}`. Declare the input as an `int` for `page` and `offset`, or a `string` with no default for `cursor`. |

When the answer says there is another page, the call returns `meta.next`, an
opaque handle such as `ic_4f0a4207d04f4a0fb4e059d26bd54b6d`. itda keeps the
marker on the user's computer for fifteen minutes, bound to the site, action,
account, inputs and definition. `itda call ... --next <handle>` (over MCP,
`next`) sends it back in the declared input. Agents never see or set a `cursor`
input; they can still give `page` and `offset` inputs directly. For a pointer
holding an address or query string (`?page=2&per_page=10`), itda extracts only
the input's value, and only if the origin and path match. itda drops a marker
that would change the read or point elsewhere. Only a read turns pages.

`itda draft openapi` declares it when the document is plain about it: a
`cursor`, `page_token` or `after` query parameter together with a `next_cursor`,
`next_page_token` or `end_cursor` string in the answer; a `page` parameter, with
`header:link` when the answer documents a `Link` header; an `offset` parameter.
`itda draft har` does the same from the query a recorded read sent and the
fields its answers had, and from a GraphQL `after` variable with
`pageInfo.endCursor` in the answer. itda drafts GraphQL schema connections
(`after: String` in, `pageInfo { endCursor hasNextPage }` out) with
`send: {body: after}`. Pages with pagination links (`?page=2`) get `style: page`
on reads returning their records.

## `confirm`

itda sends a change once and never resends it. When the answer is lost after
sending (a timeout, a dropped connection), the call fails with
`action_outcome_unknown` and nobody knows whether the site applied it. A change
can name the read that would show it, so itda checks instead of guessing:

```yaml
operations:
  - name: list-notices
    method: GET
    path: /api/notices
    effect: read
    evidence: GET /api/notices answers {items: [{id, title}]}
    response:
      format: json
      required_pointers: [/items]
  - name: create-notice
    method: POST
    path: /api/notices
    effect: write
    evidence: POST /api/notices creates a notice; GET /api/notices lists it under title
    body:
      json:
        title: {type: string, required: true}
    confirm:
      read: list-notices
      match: {pointer: /items/*/title, from: body.title}
```

| Field | Meaning |
|---|---|
| `read` | A read in the same definition, with `effect: read`, that runs on its defaults alone (no required input without a default, no path input). |
| `match.pointer` | A JSON pointer into the read's answer as itda returns it, after `extract_pointer` and `model`: `/items/*/title` for records under `items`, `/*/title` for a list. `*` stands for every element. HTML reads return their records under `items`, so the same shape works for them. |
| `match.from` | The change's input to compare with, as `body.<name>`, `query.<name>` or `params.<name>`. A string or number input that is required or has a default; never one a preflight step fills. |

After `action_outcome_unknown`, itda runs the read once, as a read: no
approval, through the same gate and pacing, with the change's account, first
page only. It compares each value at the pointer with the input as text and
reports one of three results next to the error, as `outcome_check` in
`--json` and MCP results and as `outcome_check` on the audit record:

| `result` | Meaning | What follows |
|---|---|---|
| `landed` | the input is in the read's answer | the change is in; itda tells the agent not to ask for it again. A new approval request with the same inputs carries a `note` saying the earlier one landed, so you can decline it |
| `not_found` | it is not there | the normal path: the agent may ask again, and you review a new approval |
| `unknown` | the read failed too (`error` says why) | check the site before asking for the change again |

The result stays `action_outcome_unknown` (exit 22) in every case; the check is
information next to it, never a reason to resend. The read's answer is not
passed on, only the read's name and the match result. A `confirm` that names a
missing operation, a change, or a read that needs inputs is a definition
problem, reported by `itda ls` and `itda doctor` like any other.

`itda setup` and `itda draft` fill `confirm` when the draft itself shows it: a
`POST` whose list read is on the same path or above it (`POST /api/notices`
with `GET /api/notices`, `POST /issues/new` with `GET /issues`) and a string
input of the change that the read's records carry under the same name (an
HTML read's `fields`, a `model`'s fields, or `/0/<field>` in
`required_pointers`). The evidence says so. Without that, the change keeps no
`confirm`; add one by hand when you know the read.

## Keep it working

Sites change. `itda check` runs every read needing no input; give inputs a
`default` to include more. It reports whether each action's answer still matches
its `response`:

```console
$ itda check board
board.list-notices  ok                 HTTP 200 142 ms
board.get-notice    skipped            needs notice_id
board.post-notice   skipped            changes the site
board.my-messages   auth_required      sign in: itda login board
verdict  signed_out: sign in again with itda login board

1 ok, 1 failed, 2 skipped.
```

itda sends no changes. Each site gets a verdict (`ok`, `signed_out`,
`site_changed`, `blocked` or `unreachable`) and a matching exit code, per the
[contract](agents.md). Code 11 means a site no longer matches its definition,
letting a scheduled job warn you before an agent runs into it.
`itda check --all` checks every installed site and prints one verdict line each.
An address the site answers with 404, 405 or 410 and a query it answers with
errors count as `site_changed`. itda lists a refused read (`api_denied`) on its
own line; it does not affect the site's verdict.

### Draft a changed site again

A definition that `itda setup`, `itda draft ... --install` or `itda record
--install` made remembers where it came from, in `sources.json` next to it:

```json
{
  "pages": ["https://board.example/board"],
  "description": "https://board.example/openapi.json",
  "graphql": ["https://board.example/graphql"],
  "recording": "recording.har",
  "lang": "en",
  "drafted": {"get-board": "<sha256 of the operation as drafted>"},
  "drafted_at": "2026-10-22T09:00:00.000Z"
}
```

`pages` lists addresses the draft read; `description` is the API description it
found (an address or the file given to `itda draft openapi`); `graphql` lists
endpoints it asked what they offer. `recording` is the HAR kept under the site
when `itda draft har` or `itda record --save ... --install` had one. A refresh
needs only names and shapes, so the copy keeps the recording's shape: cookie,
token and session values, private query and body fields and sign-in call answers
become `<redacted>` before saving. `itda remove` deletes the copy with the site.
`template` names the site's ready-made template (`gitlab`, `jira`...), with its
API base in `base_url`. A refresh fills that template again and rereads the page
if it still opens. `base_url`, `include` and `lang` otherwise record the draft's
choices; itda omits any you did not give. Nothing in the file is a secret.
`drafted` holds each operation's original draft hash. A refresh uses it to
distinguish hand-edited operations from those it may replace. A definition
installed with `itda add` has no sources and cannot be refreshed.

When a check says `site_changed`, draft the site again from those sources:

```console
$ itda refresh board
Drafting 'board' again from its pages...
    Looking at board.example...
    As installed: site_changed (the site changed; run itda refresh board to draft it again from the same sources).
What would change:
    ~ get-board    changed: evidence, response
    ~ feed         changed: path, evidence
    = list-notices kept as you edited it
    + search-board added: Search on Notices
    Write these changes to 'board'? [y/N] y
Updated 'board'. Checking it...
board.get-board     ok                 HTTP 200 131 ms
...
verdict  ok: every read still matches the site
```

The merge keeps the site's name, title and `base_url`, `defaults`, `auth` and
the header comments as they are, and goes through the operations by name. YAML
differing from the last source draft means hand edits. itda keeps that operation
as is. One the new draft changed (a renamed column, a moved endpoint, a response
check) takes the new version. itda drops what the sources no longer draft only
if its read also failed the check. itda tries a new operation once and adds it
if it answers. itda withdraws pending approvals for the site, as for any
replaced definition. `-y` writes without asking; without it, itda writes only
after you say yes. itda checks the site right after and prints the verdict. The
exit code says whether the new definition matches. Over MCP, `itda_manage` with
`refresh` does the same after you confirm the diff in a form.

## Checklist before sharing a definition

- `itda spec validate` finds no problem.
- Every read declares a `response`, and `itda check` passes.
- Every change whose records a read lists names that read in `confirm`.
- Every `desc` says what the action does, in one line.
- `evidence` points at something a reviewer can check.
- No credentials, personal data or internal hostnames in the file you share.
- The site's terms allow automated access for your use.
