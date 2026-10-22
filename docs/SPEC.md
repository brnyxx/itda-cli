# Writing a site definition

A site definition (`spec.yaml`) tells itda which actions a website has, what
each one sends, what a correct answer looks like and whether it changes
anything. itda reads it from `<data dir>/specs/<site>/spec.yaml` (next to it,
`check.json` keeps the time and verdict of the last `itda check`, and
`sources.json` what a drafted definition was made from; see
[Keep it working](#keep-it-working)); `itda add spec.yaml` copies it there
after a short review, `itda doctor` prints the data directory, and
`ITDA_HOME` moves it.

## Start from an OpenAPI document

When the site publishes an OpenAPI 3 or Swagger 2.0 description, let itda
draft the definition:

```sh
itda draft openapi openapi.json > spec.yaml            # print the draft
itda draft openapi openapi.json --include orders       # only matching paths, ids or tags
itda draft openapi openapi.json --base-url https://api.example.com --install
```

The draft keeps every operation's source in `evidence`, turns `GET` into reads
and everything else into changes, adds response checks from the required
fields of the success schema, and maps header API keys and bearer tokens to
`auth`. What it cannot translate (OAuth, cookie keys, header parameters,
multipart bodies, deprecated operations) is listed on stderr instead of being
guessed. Review the draft before agents use it: shorten descriptions, remove
actions agents should not call, and tighten the response checks.

## Start from the pages you use

For a site with no API description, and pages you can open without signing
in, give itda the addresses of the pages an agent should read:

```sh
itda draft page https://example.com/board > spec.yaml      # print the draft
itda draft page https://example.com/board https://example.com/board/42
itda draft page https://example.com/board --name board --install
```

itda fetches each page as itself (`itda/<version>`, never a browser), on the
same rules as every other request: private addresses need `ITDA_ALLOW_LOCAL=1`,
and a page that refuses or challenges the request pauses the site instead of
being retried. The draft then stops with what the gate saw, as `itda call`
would report it: `blocked_by_site` (exit 12) naming Cloudflare, a bot
manager or a captcha, or `site_rate_limited` (exit 13) with the wait the
site asked for. A page that sends itda on to a sign-in page stops with
`auth_required` (exit 10) and points at `itda record`. itda does not go
around a block; the site's official API, or its owner, is the way in. Each
page that answers becomes a read with a response check, so `itda check`
notices when the page changes:

| What the page has | What the draft gets |
|---|---|
| A table with a header row | `html.items` per row, one field per column named after its header, plus `link` |
| A list of similar elements with a title and a link (cards, posts, `tr.athing`) | `html.items` with `title`, `link` and, when present, `id`, `summary`, `meta`, `published_at`, plus whatever most records hold at one place under a class the site named (`score`, `author`, `release__version` becomes `version`); build names such as `heading__fzQskI` are left out |
| Any run of three or more siblings of one shape, outside navigation, sidebars, headers and footers, each holding a title at the same place: a link (`ol.stories > li.story`, `#articles > dt`, a `dl` per section), a heading, a text named as a title (`span.result__title`), or the record itself when it is a link with something inside (`a.release`) | The same, with the title and link read from that title's own path in the page and a text that reads as a date as `published_at`; a heading or a link that is the whole record needs one more field, so bare links, menus and paragraphs never qualify. The read is described by what the records are when the page names them (`List the projects on Projects`) |
| A list whose records' links share one shape: one path with one segment that differs per record (`/item/{id}`), or one path with one query key that does (`view.php?id=`) | A read of one record (`get-project`, `get-post`) with a required `id` input (`int` when the ids are numbers), the query keys every link sends as inputs with defaults, and no response check yet: run it once, then declare what the page shows |
| One thing (an article, a package, a product, a bug): an `h1` outside the site's header and menus, with a body of text, a date, or two or more labelled values beside it, outside any list of records (or the list takes less than a quarter of the main block) | `html.page` fields on the whole page, each matching once: `headline` (required), `body` (the longest block named as one, `div.project-description`, `#readme`, `[itemprop=articleBody]`, else the block that holds most of the text), what sits beside the headline under a class the site named (`project-header__summary` becomes `summary`), `description` from the page's meta, `published_at`, `author`, and one field per `dt`/`dd` pair or `th`/`td` row, named after its label. A page that is one thing with a named list of five or more records on it (a package with its releases) gets both reads |
| A list inside data embedded for its scripts: an `ItemList` or a list of one typed thing (`NewsArticle`, `Product`...) in `ld+json`; in `__NEXT_DATA__`, `__NEXT_F__`, `__APOLLO_STATE__` (grouped by typename), `__PRELOADED_STATE__` and the like, the list of objects with the most to read (a title, a link, a date) and, among those, the longest | One read per list, named after what it holds (`list-news-articles`), with `ssr_json`, `extract_pointer` to the list, `required_pointers` on its first record and a `model` that keeps up to eight plain fields of each record, ids and titles first; credentials are left out |
| Data embedded for its scripts with no list in it | `ssr_json` with `required_pointers` on the whole payload, and a note saying what to add |
| Only a title | `required_html_fields` on its `h1` or `title`, and a note saying what to add |
| Links to `?page=2` on the same path, or a link marked `rel="next"` with a number in its query | A query input named after that key (`int`); `page` and the like default to 1, `offset` and `start` to 0, and a key the page's own address carries keeps its value. The reads that return records also get `pagination` on that key (`style: page`, or `style: offset` for a key that starts at 0) |
| A `GET` form | A read with the form's fields as `query` inputs |
| A `POST` form | A change with the fields as `body.form`, using `auth: session`; a hidden token such as `_csrf` becomes a `preflight` step read from the form page |
| A sign-in form | Left out; `itda login <site> --browser` signs in |
| `<link rel="alternate">` to RSS or Atom | A read with `format: xml` |
| A link to an OpenAPI document (`rel="service-desc"`, or an `href` ending in `openapi.json`, `openapi.yaml` or `swagger.json`), or, when nothing is linked, `/openapi.json` on the same origin answering with one | Its reads join the draft as `itda draft openapi` would make them, under the page's address; its changes are listed as notes; see below |

When a site also publishes an API description, itda reads it: the one the
page links to, else `/openapi.json` on the same origin when it answers with
an OpenAPI 3 or Swagger 2.0 document (one GET, as itda, at most 4 MB; every
request to a site waits its turn, so no other path is tried). Its reads become actions the way `itda draft openapi`
makes them, so the same path is drafted once whether the page or the
description named it. The changes it lists are notes rather than actions (one
each up to ten, then a count); `itda draft openapi <url>` drafts those for
review. A read whose input is a credential, such as an `apikey` query
parameter, is left out and listed, as the install-time checks would refuse
it. A description whose server is another address is not merged, only noted,
and one that does not parse leaves the page draft as it is. The header
comment and every merged action's `evidence` name the description's address.

Field and input names go through the same checks as a hand-written
definition: a name that looks like a credential is left out and listed. A
hidden form field that is not a token keeps its value as a default, since the
form always sends it; nothing else from the page is copied. A page drawn by
scripts shows little to a plain fetch; `itda draft page` says so and points
at recording the site instead, and `itda setup` first watches the page load
in your browser (below).

## Start from the software a site runs

Most intranets run software everyone knows, and that software has a stable
API worth more than a reading of one page. `itda setup` recognises eight of
them from the page's HTML (a `generator` meta, a well-known asset path, an
Atlassian context-path meta, a body class) and confirms the guess with exactly
one GET to that software's own endpoint, as itda, through the same gate as
every other request. No fingerprint, no request. A confirmed match says so in
one line, "This looks like GitLab. Using its API at <base>.", and drafts from
the software's template under [`sites/software/`](https://github.com/brnyxx/itda-cli/tree/main/sites/software):

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

Each template holds the reads an agent needs most (lists, search, one item;
eight to fourteen), up to three changes marked as such, models for the
records and the software's own way of signing in, every operation with its
documentation in `evidence`. The site is named after the host
(`issues.apache.org` becomes `issues-apache`; `--name` overrides), the
template's `store:<software>/...` secrets follow that name, and every read
is tried once as on any site; a read the instance does not answer (a
private project list, a disabled module) is removed. The page draft still
runs: its lists, forms and feeds join the template under the page's own
address when it differs from the API's, the same method and path drafted
once. A page that is not enough on its own, because it needs a sign-in or is
drawn by scripts, still gets the template when the software is confirmed.
When the page says WordPress but `/wp-json/` does not answer the way
WordPress does, nothing is said and the page is drafted like any other.

`itda setup --as <software>` forces a template (the fingerprint's base when
the page agrees, else the API under the page's origin; the confirm request is
still made and the line says when it did not answer), `--no-template` reads
the page like any other, and `itda check` and `itda refresh` work on the
result as on any site: a refresh fills the same template at the same API
base and merges the page's reads again. Over MCP, `itda_setup` does the same, takes `template` (a name, or
`none`) and names the template it used in its result. The templates are not
installed by `itda add`: their `base_url` is a placeholder the setup fills.

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
profile at that page and watches, through the browser's own DevTools
protocol, what every page asks the server for. Use the site the way an agent
should: sign in, search, open an item, submit a form. Press Enter in the
terminal, or close the window, when you are done. itda never clicks, types or
navigates in that window; only the page you named is opened. The recording
stays in memory unless `--save` writes it to a file, which then holds your
cookies and tokens and should be deleted afterwards. With `--install`, itda
then offers to keep what the recording signed in with, each after a yes in
the terminal: the token a drafted header `auth` entry refers to, and the
browser session for a `browser` one. They go to the system keyring under
`<site>/default`, never on screen, so `itda login` is not needed until a
token expires.

`itda setup` runs a shorter form of this by itself when the page shows itda
nothing but a title and carries scripts that could fetch something (a script
file or a preloaded module, or an inline script that imports a module or
mentions `fetch`, `XMLHttpRequest`, `graphql` or `api`; a page without them
goes straight to the question below): it opens
the same kind of window on the page, waits
until the network has been quiet for two seconds (ten at most), scrolls once
to the bottom, waits again, closes the window and drafts from what the page
loaded on its own, the way `itda draft har` does, trying the page's own
address first so what the page itself shows is never lost to a busier API
elsewhere. It says what it saw in one line (`Watched the page load: 12
calls, 3 reads drafted.`, counting the reads beyond a title check) and only
asks you to record when no such read came out. Nothing in the window is clicked or
typed, and nothing it signed in with is kept, since nobody signed in; a page
whose scripts go to a sign-in page stops with `auth_required`, as a redirect
does. `--no-record` skips the window as well, `--watch-only` watches but
never asks you to record. A site whose software is recognised (above) gets
its template instead and is not watched.

A recording saved from the browser's developer tools works the same way:
open the Network tab, use the site, save it with "Save all as HAR" (Chrome,
Edge) or "Save All As HAR" (Firefox), and draft from the file:

```sh
itda draft har session.har > spec.yaml                  # print the draft
itda draft har session.har --base-url https://intranet.example.com/api
itda draft har session.har --name board --install
```

A page that talks GraphQL sends every call to one path, so each recorded
operation becomes its own action, named after it (`exploreProjects` is
`explore-projects`): a query is a read, a mutation a change, the document and
the operation name are fixed inputs, and the variables are the inputs an agent
fills in, rendered through a `json_template` into the same JSON the page sent.
Variables that held objects or lists are listed for you to add by hand.

A GraphQL endpoint can also say what else it offers. When a draft sees one
(a recorded call to it, or a page naming a same-origin `/graphql` path), itda
sends it the standard introspection query once, as itself, as a read with a
size cap, and drafts one read per root `Query` field whose arguments are
scalars, enums or ids: the field name becomes the action (`countries`), its
arguments the typed inputs (required when the schema says so), and the
selection is the field's scalar fields, two levels deep, up to 24 of them.
An optional argument that holds an object or a list is left out of the read
and listed, so the field is still read without it; a field that requires one,
or a credential, is listed instead of drafted. An argument named `query`
becomes the input `<field>_query`, since `query` holds the document.
`Mutation` fields are only listed in the notes, never drafted as changes: a
schema says what a change takes, not what it does. A site that has
turned introspection off is drafted from its pages and calls alone, with no
error. This is the one request `itda draft har` makes on its own.

Either way, the pages you saw are drafted too, the way `itda draft page`
does it, so a site that renders its lists and forms on the server gets
actions even when its pages call no API. A recording holds real tokens,
cookies and personal data, so the draft keeps only the shape of each call:
method, path, input names and types, and the response fields every sample
returned. Numbers and ids in paths become
placeholders (`/notices/42` becomes `/notices/{notice_id}`). Inputs every
sample sent are marked required. Token headers become `auth` entries whose
values you add with `itda login`. Calls that sent a sign-in cookie (by name,
such as `JSESSIONID` or `SID`) and no token get a `kind: browser` entry, so
`itda login <site> --browser` signs them in. A form token such as `_csrf` becomes a
`preflight` step that reads it from the form page recorded before the post, so
each change sends a fresh one. A header such as `X-CSRF-Token` gets the same
treatment from a `<meta name="csrf-token">` tag on an earlier page, the pattern
Rails, Laravel and many single-page apps use; other token and session fields are left out and
listed, since they come from sign-in, never from the agent. Calls to other addresses, file uploads and failed calls are skipped.
Delete the HAR file when you are done with it.

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

## `site`

| Field | Required | Meaning |
|---|---|---|
| `name` | yes | Id used on the command line and in MCP tool names. |
| `title` | no | Human name, used in tool descriptions and approval prompts. |
| `base_url` | yes | `https://` origin every path is joined to. Local and private addresses also need `ITDA_ALLOW_LOCAL=1`. |

`defaults.headers` adds headers to every request (for example an `Accept`
language). Headers that imitate a browser (`sec-*`, client hints) are refused;
see [HONEST_CLIENT.md](HONEST_CLIENT.md).

`defaults.interval` is the least time between two requests to the site,
written as `2s` or `500ms`. itda sends one request at a time to a site and
leaves about 1.2 seconds between them on its own; a definition can ask for
more room (up to 30 seconds), never for less. Use it when the site's terms
name a rate, or when `itda check` keeps running into `site_rate_limited`.

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
| `path` | Path under `base_url`. `{name}` is replaced by a path input. |
| `effect` | `read` or `write`. See below. |
| `evidence` | Why you believe the effect is right: a doc link, a captured request. Reviewers read it. |
| `auth` | Name of an `auth` entry, for actions that need an account. |
| `params` | Path inputs, all required. |
| `query` | Query string inputs. |
| `body` | Request body inputs: `json:` or `form:` fields, or a `json_template`. |
| `headers` | Extra headers for this action only. |
| `response` | What a correct answer looks like. Strongly recommended for reads. |
| `model` | Name of an entry in `models` that picks and renames fields from the JSON. |
| `pagination` | How a read that lists things goes on to its next page. See below. |

### Inputs

```yaml
query:
  q:     {type: string}
  page:  {type: int, default: 1}
  draft: {type: bool, required: true}
```

`type` is `string`, `int`, `float` or `bool`. Inputs are checked before
anything is sent, and an agent sees them as a JSON Schema. Inputs whose names
look like credentials (`password`, `token`, `api_key`…) are refused: those go
in `auth`, never through the agent.

### Read or write

An action counts as a read only when everything agrees: `effect` says `read`,
the method is not `PUT`, `PATCH` or `DELETE`, neither the name nor the path
contains a word that means a change (`delete`, `send`, `pay`, `submit`,
`logout`…), no input such as `action` or `mode` asks for one, and a GraphQL
`query` input holds no mutation. A `POST` can be a read, for example a search
endpoint or a GraphQL query. Anything else is a change. A change never runs until the user approves that exact request
with `itda approve`, and it is sent once, without retries.

When the checks disagree with your `effect: read`, itda treats the action as a
write. Rename the action or path rather than fighting the classifier: a `GET
/logout` really is a change.

## `response`

A declared response is how itda knows the answer is real data and not a sign-in
page, an error page or a redesigned site.

```yaml
response:
  format: json                       # json | html | text | xml | empty
  required_pointers: [/items, /total]  # JSON Pointers that must exist
```

When the records sit inside a wrapper, `extract_pointer` returns only that
part, and `required_pointers` and `model` then apply to it:

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
picks among several accounts; the default account is `default`. The header is
only ever sent to `base_url`'s origin, or to `origin` when you set it.

### Browser sign-in

For sites you sign in to with a password, single sign-on or a one-time code,
and that offer no API key:

```yaml
auth:
  - name: session
    kind: browser
    value_from: cookie:JSESSIONID   # optional: the cookie that shows you are signed in
```

Then sign in once in your own browser:

```console
$ itda login intranet --browser
Sign in to intranet in the browser window that just opened.
itda continues when the JSESSIONID cookie appears, or press Enter once you are signed in.
Saved the session for intranet/default (3 cookies) in the system keyring. Agents only see the account id.
```

itda opens Chrome, Edge or Chromium (or `ITDA_BROWSER`) in a fresh, empty
profile at the site's address, or at `--url`. You sign in by hand; itda never
clicks or types on the page. It keeps only the unexpired cookies for that
site, closes the window and deletes the profile. The session is sent only to
addresses on the same site, and cookie values are removed from results.

When the session expires, the site answers 401 or 403, the account pauses with
`auth_required`, and `itda login <site> --browser` signs it in again and
resumes it. On a server without a keyring, set
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

A step is a `GET` to the action's own origin, with the same account, either
to `path` or to a read action named in `op` (with `args`). Steps run in order,
after you approve a change and right before it is sent, so the value is always
fresh. A value is used as `{{step.<step>.<name>}}`; inputs filled this way are not
shown to agents and they cannot set them. When a step finds nothing, the call
stops with `response_mismatch` and nothing is sent.

## `models`

A model picks and renames fields from a JSON answer, so agents get small,
stable records. When the answer is a list, each record is picked the same way.
`itda draft page` writes one for every list it finds in a page's embedded
data; `path` reaches into nested objects with dots (`item.headline`):

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

A read that lists things can say how the site turns pages, so an agent can
read on without ever seeing the site's own marker:

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
| `next` | Where the next marker is in the answer: a JSON pointer such as `/meta/next_cursor` or `/data/notices/pageInfo/endCursor`, or `header:link` for the `rel="next"` address in the `Link` header. A page or offset read may leave it out: the next page is then the one after the current page, or the offset past the records returned, when the answer had records. |
| `more` | A JSON pointer that must hold `true` for another page to exist, such as `/data/notices/pageInfo/hasNextPage`. Optional. |
| `send` | The input the marker goes back in: `{query: page}` or, for a JSON body, `{body: after}`. The input must be declared: an `int` for `page` and `offset`, a `string` with no default for `cursor`. |

When the answer says there is another page, the call returns `meta.next`,
an opaque handle such as `ic_4f0a4207d04f4a0fb4e059d26bd54b6d`. The marker
itself is kept on the user's computer for fifteen minutes, bound to the
site, action, account, inputs and definition; `itda call ... --next
<handle>` (over MCP, `next`) sends it back in the declared input. A
`cursor` input is never shown to agents and cannot be set by them; `page`
and `offset` inputs can still be given directly. A pointer that holds an
address or a query string (`?page=2&per_page=10`) gives only the input's
value from it, and only when that address is the same origin and path. A
marker that would turn the read into something else, or an address
elsewhere, is dropped. Only a read turns pages.

`itda draft openapi` declares it when the document is plain about it: a
`cursor`, `page_token` or `after` query parameter together with a
`next_cursor`, `next_page_token` or `end_cursor` string in the answer; a
`page` parameter, with `header:link` when the answer documents a `Link`
header; an `offset` parameter. `itda draft har` does the same from the
query a recorded read sent and the fields its answers had, and from a
GraphQL `after` variable with `pageInfo.endCursor` in the answer. A GraphQL
schema's connection (`after: String` in, `pageInfo { endCursor hasNextPage }`
out) is drafted with `send: {body: after}`. A page whose links turn pages
(`?page=2`) gets `style: page` on the reads that return its records.

## Keep it working

Sites change. `itda check` runs every read that needs no input (give inputs a
`default` to include more) and reports, per action, whether the answer still
matches its `response`:

```console
$ itda check board
board.list-notices  ok                 HTTP 200 142 ms
board.get-notice    skipped            needs notice_id
board.post-notice   skipped            changes the site
board.my-messages   auth_required      sign in: itda login board
verdict  signed_out: sign in again with itda login board

1 ok, 1 failed, 2 skipped.
```

Changes are never sent. Each site ends with one verdict (`ok`, `signed_out`,
`site_changed`, `blocked` or `unreachable`) and the exit code follows it, as
the [contract](agents.md) lists: 11 when a site no longer matches its
definition, so a scheduled job can warn you before an agent runs into it.
`itda check --all` checks every installed site and prints one verdict line
each.

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

`pages` are the addresses the draft read, `description` the API description
it found (an address, or the file given to `itda draft openapi`), `graphql`
the endpoints it asked what they offer, and `recording` the HAR kept under
the site when `itda draft har` or `itda record --save ... --install` had
one. That copy keeps the shape of the recording and not its values: cookie,
token and session values, private query and body fields and the answers of
sign-in calls are blanked to `<redacted>` before it is written, since a
refresh needs names and shapes only; `itda remove` deletes it with the site. `template` names the ready-made template a
site came from (`gitlab`, `jira`...), with its API base in `base_url`, so a
refresh fills the same template again and reads the page again when it still
opens. `base_url`, `include` and `lang` are otherwise the
choices the draft was made with, each left out when it was not given. Nothing in the file is a secret. `drafted`
holds a hash of every operation as it was drafted, which is how a refresh
tells an operation you edited by hand from one it may replace. A definition
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

The merge keeps the site's name, title and `base_url`, `defaults`, `auth`
and the header comments as they are, and goes through the operations by
name: one whose YAML differs from what the sources drafted last time was
edited by hand and is kept as it is; one the new draft changed (a renamed
column, a moved endpoint, a response check) takes the new version; one the
sources no longer draft is dropped only when its read also failed in the
check; one the draft now has is tried once and added when it answers.
Approvals are untouched. `-y` writes without asking; without it, nothing is
written until you say yes. The site is checked right after and the verdict
is printed, so the exit code says whether the new definition matches.
Over MCP, `itda_manage` with `refresh` does the same after you confirm the
diff in a form.

## Checklist before sharing a definition

- Every read declares a `response`, and `itda check` passes.
- Every `desc` says what the action does, in one line.
- `evidence` points at something a reviewer can check.
- No credentials, personal data or internal hostnames in the file you share.
- The site's terms allow automated access for your use.
