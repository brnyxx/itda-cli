# Writing a site definition

A site definition (`spec.yaml`) tells itda which actions a website has, what
each one sends, what a correct answer looks like and whether it changes
anything. itda reads it from `<data dir>/specs/<site>/spec.yaml`; `itda doctor`
prints the data directory, and `ITDA_HOME` moves it.

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

## Write one by hand

This page covers everything version 0.1 runs. The fastest way in is to copy
[`examples/sandbox/spec.yaml`](../examples/sandbox/spec.yaml) and change it.

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

Version 0.1 supports API keys and tokens sent in a header:

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

Browser sign-in (`kind: browser`) and preflight steps are part of the format
but refused by the 0.1 runtime.

## `models`

A model picks and renames fields from a JSON answer, so agents get small,
stable records:

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

## Checklist before sharing a definition

- Every read declares a `response`.
- Every `desc` says what the action does, in one line.
- `evidence` points at something a reviewer can check.
- No credentials, personal data or internal hostnames in the file you share.
- The site's terms allow automated access for your use.
