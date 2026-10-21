# Writing a site definition

A site definition (`spec.yaml`) tells itda which actions a website has, what
each one sends, what a correct answer looks like and whether it changes
anything. itda reads it from `<data dir>/specs/<site>/spec.yaml`; `itda add
spec.yaml` copies it there after a short review, `itda doctor` prints the data
directory, and `ITDA_HOME` moves it.

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
being retried. Each page becomes a read with a response check, so `itda check`
notices when the page changes:

| What the page has | What the draft gets |
|---|---|
| A table with a header row | `html.items` per row, one field per column named after its header, plus `link` |
| A list of similar elements with a title and a link (cards, posts, `tr.athing`) | `html.items` with `title`, `link` and, when present, `id`, `summary`, `meta`, `published_at` |
| Any run of three or more siblings of one shape, outside navigation, sidebars, headers and footers, each holding one link at the same place (`ol.stories > li.story`, `#articles > dt`, a `dl` per section) | The same, with the title and link read from that link's own path in the page |
| Data embedded for its scripts (`__NEXT_DATA__`, `__NUXT_DATA__`, `__remixContext`, `__APOLLO_STATE__`, `ld+json`) | `ssr_json` with `required_pointers` |
| Only a title | `required_html_fields` on its `h1` or `title`, and a note saying what to add |
| Links to `?page=2` on the same path | A `page` query input (`int`, default 1) |
| A `GET` form | A read with the form's fields as `query` inputs |
| A `POST` form | A change with the fields as `body.form`, using `auth: session`; a hidden token such as `_csrf` becomes a `preflight` step read from the form page |
| A sign-in form | Left out; `itda login <site> --browser` signs in |
| `<link rel="alternate">` to RSS or Atom | A read with `format: xml` |
| A link to an OpenAPI document | A note suggesting `itda draft openapi` |

Field and input names go through the same checks as a hand-written
definition: a name that looks like a credential is left out and listed. A
hidden form field that is not a token keeps its value as a default, since the
form always sends it; nothing else from the page is copied. A page drawn by
scripts shows little to a plain fetch; the draft says so and points at
recording the site instead.

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
cookies and tokens and should be deleted afterwards.

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

A step is a `GET` to the action's own origin, with the same account, either
to `path` or to a read action named in `op` (with `args`). Steps run in order,
after you approve a change and right before it is sent, so the value is always
fresh. A value is used as `{{step.<step>.<name>}}`; inputs filled this way are not
shown to agents and they cannot set them. When a step finds nothing, the call
stops with `response_mismatch` and nothing is sent.

## `models`

A model picks and renames fields from a JSON answer, so agents get small,
stable records. When the answer is a list, each record is picked the same way:

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

1 ok, 1 failed, 2 skipped.
```

Changes are never sent. The exit code follows the [contract](agents.md):
11 when a site no longer matches its definition, so a scheduled job can warn
you before an agent runs into it.

## Checklist before sharing a definition

- Every read declares a `response`, and `itda check` passes.
- Every `desc` says what the action does, in one line.
- `evidence` points at something a reviewer can check.
- No credentials, personal data or internal hostnames in the file you share.
- The site's terms allow automated access for your use.
