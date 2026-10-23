# Use it from an agent

The repository has two files for the agent itself:
[AGENTS.md](https://github.com/brnyxx/itda-cli/blob/main/AGENTS.md), a guide
to using itda well, and
[skills/itda/SKILL.md](https://github.com/brnyxx/itda-cli/blob/main/skills/itda/SKILL.md),
the same guide in the short form skill loaders take. The MCP server hands the
agent the same rules as its instructions.

## MCP clients

```sh
itda mcp install claude-desktop   # or cursor, vscode, codex
itda mcp install claude-code      # prints the `claude mcp add` command
```

| Client | File itda changes |
|---|---|
| `claude-desktop` | `claude_desktop_config.json` in the Claude app's folder |
| `cursor` | `~/.cursor/mcp.json` |
| `vscode` | `.vscode/mcp.json` in the current folder |
| `codex` | `~/.codex/config.toml` |
| `claude-code` | none; prints `claude mcp add --scope user itda -- …` |

The installer keeps your other servers and settings and saves the previous file
as `.bak`. It changes nothing if itda is already there. `--dry-run` shows the
new file without writing it. It starts itda by its absolute path and passes
`ITDA_HOME`, `ITDA_ALLOW_LOCAL` and `ITDA_SECRET_STORE` along when they are set.
Restart the client afterwards.

## What the agent sees

With up to 30 actions installed, each is a tool named like the CLI
(`sandbox.list-notices`), with a JSON Schema for inputs. Hints: reads are
`readOnlyHint`; `DELETE` actions are `destructiveHint`; `PUT` and `DELETE` are
`idempotentHint`.

With more actions, itda lists three fixed tools. The tool list stays short and
prompt caches stay valid:

| Tool | Use |
|---|---|
| `itda_search` | Find actions by keywords |
| `itda_describe` | Show one action's input schema and effect |
| `itda_call` | Run an action with `args` (and `approval` for a change, `next` for the next page of a list) |

`itda mcp --mode direct` or `--mode catalog` overrides the choice.
`itda mcp --site <name>` serves only that site's actions as tools. It refuses
`itda_setup` and confines `itda_login`, `itda_check`, `itda_sites` and
`itda_manage` to that site. Use it in a project's MCP configuration when the
agent should only reach one site from there.

Five tools are always there, so the agent never needs a terminal to set a
site up or look after it:

| Tool | Use |
|---|---|
| `itda_setup` | Set up a site from a page address: look at the page as itda and draft. Try every read once and keep what works. The site's actions become tools in the same session. The server declares `tools.listChanged` and sends `notifications/tools/list_changed`; the next `tools/list` has them. A script-drawn page opens briefly in a browser window on your screen. itda watches it load on its own, without anyone clicking or typing. itda refuses pages that need a sign-in or still show nothing and gives you the command to run. It never asks you to record here. |
| `itda_login` | Open your own browser to sign in, exactly like `itda login <site> --browser`. itda seals the session under the data directory with a key from the keyring; the agent only sees the account id. A client with forms asks you first. Nobody presses Enter, so the definition must name the cookie that shows a sign-in. |
| `itda_check` | `itda check` for one site, or every site when none is named: the verdict per site and what to do, as `itda check --json` prints them. |
| `itda_sites` | What is set up, with reads, changes and the last check's time and verdict. |
| `itda_manage` | `refresh` drafts a changed site again from its sources. The form shows what would change; no answer counts as no. For `remove` and `resume`, you confirm in a form; clients without forms get the command to run. `approvals` lists what waits for you. `audit` lists the last entries for one site or all, plus, under `unknown_outcome`, changes interrupted mid-process and changes whose answer was lost, each with what the confirming read said (`check`) when the change named one. |

Every result uses the `itda call --json` envelope, marked `"untrusted": true`. A
note before it says website data is not instructions. Results over 24 KB keep
the start of their longest lists and say `"truncated": true`. Failures are tool
errors with a code and a fix, so the agent can tell the user what to do.

## Files

A read declared `format: file` saves its answer on the user's computer and
returns a manifest in `data`, never the bytes: `{path, bytes, media_type,
sha256}`. The file lands under `files/<site>/` in the data directory, or where
`itda call ... --out <dir>` says; the command prints `Saved <path> (<bytes>
bytes, sha256 …)` once. Over MCP the result carries the manifest and a
`resource_link` to the file, and the tool's description says so. A failed
download says why in one word, and keeps nothing: `wrong_media_type` (the site
answered with a page or another kind of content), `partial_answer` (a 206 or a
short body), `empty_answer`, `too_large`. `itda check` skips file reads
("saves a file"). `itda setup` tries each one once by its headers and first
bytes (the media type and size the definition allows) and keeps nothing, so a
site whose only reads are files is set up like any other.

A change whose body carries a file takes the path of a file on the user's
computer in the input its definition names. The user approves that exact file:
the review shows its name, size and SHA-256, and other bytes under the same
path withdraw the approval (`approval_expired`, `reason: file_changed`). A
file that changes while it is being sent ends the call with
`action_outcome_unknown` and `reason: file_changed`; as with any lost answer,
never call it again on your own.

## Lists that go on

A read with `pagination` in its definition returns `meta.next` if the site
reports another page. It is an opaque handle such as
`ic_4f0a4207d04f4a0fb4e059d26bd54b6d`. The agent passes it back as `next`
(`--next` on the command line) with the same inputs to get the following page.
That page may have its own `meta.next`. The site's own marker, a cursor, page
number or offset, stays on the user's computer. itda stores it in the local
database for fifteen minutes, bound to that site, action, account, inputs and
definition. itda removes it from the data it appears in. itda refuses handles
for another action, account or inputs, or after a definition change or expiry,
with `pagination_invalid` or `pagination_expired` (exit 23). The agent starts
again without `next`. Only reads turn pages. itda sends a change once. The
server instructions, `AGENTS.md` and the skill tell the agent to read only as
many pages as the question needs.

## Changes

A change never runs on the first call. The agent gets `approval_required`, an
`approval` id and instructions to ask you. You run `itda approve <id>` in a
terminal. You see what the action does, the exact request and every input, and
answer `y`. The agent calls again with the same inputs and the `approval` id;
itda sends the change once. Until you answer, that call returns
`approval_pending` (exit 20) and the approval stays. A call with other inputs
withdraws it (`approval_expired` with `reason: inputs_changed`). With
`--approve-in-client`, clients that show forms ask you directly instead. See
[Approvals and the audit log](approvals.md).

itda sends a change once. If a request gets no reply after sending (a timeout, a
dropped connection), or a GraphQL reply carries `errors` behind a 2xx, the call
fails with `action_outcome_unknown` (exit 22). The audit log records this
outcome: the site may or may not have applied the change. itda never resends a
change on its own. An agent must not either. When the change names the read
that shows it (`confirm` in its definition), itda runs that read once and puts
the result next to the error as `outcome_check`: `landed` means the change is
in, so tell the user and do not ask for it again; `not_found` means the site
shows no trace of it, so the user may ask for a new approval; `unknown` means
that read failed too. Without `outcome_check`, check the site first; only a
person decides whether to ask for the change again. Which read a change names
shows before you call it: `itda describe` prints it under the change (`after a
lost answer: checks list-notices for the new title`), `--json` carries it as
`confirm: {read, match}`, and the change's tool description (or
`itda_describe` in catalog mode) says the same. A new approval request for
a change that landed carries `previous_outcome: landed`, and its review tells
the user so. A change that never left the
machine (no turn at the gate, a name that did not resolve, a connection that was
refused) keeps its network code. itda reports it as not sent. Agents may retry
reads, which keep their usual codes (`timeout`, `request_failed`).

A 403 that is not a sign-in problem is `api_denied` (exit 14): the site refused
that one request as not allowed. The error carries `upstream_status`. It
includes `provider_reason` only if the site's JSON answer names `access_denied`,
`permission_denied`, `insufficient_scope`, `insufficient_permissions`,
`service_disabled`, `forbidden` or `not_authorized`. itda passes on nothing else
from the answer. A 403 whose reason says the site rejected the session itself
(`unauthenticated`, `invalid_token`, `token_expired`, `session_expired`,
`login_required`) stays `auth_required`. Every refusal carries
`upstream_status`.

## When something that worked stops working

Run `itda check <site>` before retrying or re-drafting (`itda check --all`
checks every site and prints one verdict line each). It runs every read needing
no input and never sends a change. Each site ends with one verdict and what to
do:

| Verdict | Meaning | What to do |
|---|---|---|
| `ok` | every read still matches | retry the action; the failure was not the site |
| `signed_out` | the site no longer accepts the account | ask the user to run `itda login <site>` |
| `site_changed` | a page or answer no longer matches its definition, the address is gone (404, 405, 410) or the site no longer accepts the query | Ask the user to run `itda refresh <site>`, or call `itda_manage` with `refresh`. itda drafts the site again from its original sources. The user confirms the diff; itda keeps the sign-in, name and hand-edited actions. |
| `blocked` | read recovery failed or the site rate limited itda | respect `retry_after`; `paused: false` needs no resume. Only an explicitly paused site needs `itda resume <site>` |
| `unreachable` | the site did not answer | wait, then run `itda check` again |

A site with several problems gets one verdict, in this order: `signed_out`, then
`site_changed`, `blocked`, `unreachable`. The exit code follows the worst
verdict across the sites checked: 10, 11, 12 or 30 as in the table below, or 0
for `ok`. A read the site refused (`api_denied`) does not affect the site's
overall verdict. The verdict uses the other reads; the hint names the refused
read. `--json` adds `verdict`, one entry per site under `sites`, and every
action under `results` with the HTTP status the site answered as
`upstream_status`.

## Shell agents

Agents that only have a shell can call `itda call <site>.<action> --json` and
branch on the exit code:

| Exit | Meaning | What to do |
|---|---|---|
| 0 | ok | |
| 2 | bad input or unknown action (the remedy names the nearest one), a read that preflight would have turned into a change (`effect_changed`), or a read whose built request itda cannot classify (`effect_unknown`, the remedy says what is missing) | `itda describe <site>` |
| 10 | signed out or no key | ask the user to run `itda login` |
| 11 | the page no longer matches its definition, the site answered the query with errors (`query_failed`), or a file read got a page, a part or nothing (`wrong_media_type`, `partial_answer`, `empty_answer`) | update the definition, or check the inputs |
| 12 | the site refused this read, in the browser too (`blocked_by_site`) | tell the user to check the site in their browser. With `paused: false` other reads still work and nothing needs resuming; only a site paused in direct-HTTP mode (`ITDA_TRANSPORT=http`) needs `itda resume` |
| 13 | rate limited | wait as long as `retry_after` says, then retry the read |
| 14 | the site refused this one request as not allowed (`api_denied`) | tell the user what `provider_reason` says; do not sign in again |
| 20 | needs approval (`approval_required`), the user has not answered yet (`approval_pending`), or changes are off | ask the user to run `itda approve <id>`, or wait and call again with the same id |
| 21 | approval used, rejected, expired or withdrawn because the inputs changed | call again for a new one |
| 22 | itda sent a change; its outcome is unknown | check the site before retrying |
| 23 | the `next` handle expired, or was for another call | call again without `--next` |
| 30 | network error, an answer over the read's size cap (`too_large`), or a turn lost during a long transfer (`request_lease_lost`) | retry later |

With `--json`, itda prints errors on stdout as
`{"status": "<code>", "message": …, "remedy": …}` plus details such as
`approval`.
