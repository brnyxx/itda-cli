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

The installer keeps your other servers and settings, saves the previous file as
`.bak`, and changes nothing when itda is already there. `--dry-run` shows the
new file without writing it. It starts itda by its absolute path and passes
`ITDA_HOME`, `ITDA_ALLOW_LOCAL` and `ITDA_SECRET_STORE` along when they are
set. Restart the client afterwards.

## What the agent sees

With up to 30 actions installed, each action is its own tool, named like the
CLI (`sandbox.list-notices`), with a JSON Schema for its inputs and hints:
reads are `readOnlyHint`, `DELETE` actions are `destructiveHint`, and `PUT` and
`DELETE` are `idempotentHint`.

With more actions, itda lists three tools that never change, so the tool list
stays short and prompt caches stay valid:

| Tool | Use |
|---|---|
| `itda_search` | Find actions by keywords |
| `itda_describe` | Show one action's input schema and effect |
| `itda_call` | Run an action with `args` (and `approval` for a change, `next` for the next page of a list) |

`itda mcp --mode direct` or `--mode catalog` overrides the choice.
`itda mcp --site <name>` serves that one site: its actions are the only
tools, `itda_setup` is refused, and `itda_login`, `itda_check`, `itda_sites`
and `itda_manage` stay inside it. Use it in a project's MCP configuration
when the agent should only reach one site from there.

Five tools are always there, so the agent never needs a terminal to set a
site up or look after it:

| Tool | Use |
|---|---|
| `itda_setup` | Set a site up from the address of a page: look at the page as itda, draft, try every read once, keep what works. The site's actions become tools in the same session: the server sends `notifications/tools/list_changed` (it declares `tools.listChanged`) and the next `tools/list` has them. A page drawn by scripts opens briefly in a browser window on your screen so itda can watch what it loads by itself (nothing is clicked or typed). Pages that need a sign-in, or still show nothing, are refused with the command you should run instead; itda never asks you to record from here. |
| `itda_login` | Open your own browser so you can sign in to a site, exactly like `itda login <site> --browser`; the session is sealed under the data directory with a key from the keyring, and the agent only sees the account id. A client that shows forms asks you first; the definition must name the cookie that shows a sign-in, since nobody presses Enter. |
| `itda_check` | `itda check` for one site, or every site when none is named: the verdict per site and what to do, as `itda check --json` prints them. |
| `itda_sites` | What is set up, with reads, changes and the last check's time and verdict. |
| `itda_manage` | `refresh` (draft a changed site again from its sources; the form shows what would change, and a form nobody answers counts as no), `remove` and `resume` (you confirm in a form; a client without forms gets the command to run), `approvals` (what waits for you) and `audit` (the last entries, for one site or all, and under `unknown_outcome` the changes a process stopped in the middle of). |

Every result is the same envelope as `itda call --json`, marked
`"untrusted": true` and preceded by a note that website data is not
instructions. Results over 24 KB keep the start of their longest lists and say
`"truncated": true`. Failures are tool errors with a code and a fix, so the
agent can tell the user what to do.

## Lists that go on

A read whose definition declares `pagination` answers with `meta.next` when
the site says there is another page: an opaque handle such as
`ic_4f0a4207d04f4a0fb4e059d26bd54b6d`. The agent passes it back as `next`
(`--next` on the command line) with the same inputs and gets the following
page, which may carry a `meta.next` of its own. The site's own marker, a
cursor, a page number or an offset, stays on the user's computer: it is kept
in the local database for fifteen minutes, bound to that site, action,
account, inputs and definition, and it is removed from the data it appears
in. A handle given to another action, another account, other inputs, after
the definition changed or after it expired is refused with
`pagination_invalid` or `pagination_expired` (exit 23), and the agent starts
again without `next`. Only reads turn pages; a change is sent once. The
server instructions, `AGENTS.md` and the skill tell the agent to read only
as many pages as the question needs.

## Changes

A change never runs on the first call. The agent gets `approval_required` with
an `approval` id and is told to ask you. You run `itda approve <id>` in a
terminal, see what the action does, the exact request and every input, and
answer `y`. The agent then calls again with the same inputs and the `approval`
id, and the change is sent once. While you have not answered, that call
returns `approval_pending` (exit 20) and the approval stays; a call with other
inputs withdraws it (`approval_expired` with `reason: inputs_changed`). With
`--approve-in-client`, clients that show forms ask you directly instead. See
[Approvals and the audit log](approvals.md).

A change is sent once. When the request left but no answer came back (a
timeout after sending, a dropped connection), or a GraphQL answer carries
`errors` behind a 2xx, the call fails with `action_outcome_unknown` (exit 22)
and the audit log records that outcome: the site may or may not have applied
the change. itda never resends a change on its own, and an agent must not
either. Check the site first; only a person decides whether to ask for the
change again. A change that never left the machine (no turn at the gate, a
name that did not resolve, a connection that was refused) keeps its network
code and is reported as not sent. Reads keep their ordinary codes (`timeout`,
`request_failed`) and may be retried.

A 403 that is not a sign-in problem is `api_denied` (exit 14): the site refused
that one request as not allowed. The error carries `upstream_status` and,
when the site's JSON answer names one of a short list of reasons
(`access_denied`, `permission_denied`, `insufficient_scope`,
`insufficient_permissions`, `service_disabled`, `forbidden`,
`not_authorized`), `provider_reason`; nothing else from the answer is passed
on. A 403 whose reason says the session itself was rejected
(`unauthenticated`, `invalid_token`, `token_expired`, `session_expired`,
`login_required`) stays `auth_required`. Every refusal carries
`upstream_status`.

## When something that worked stops working

Run `itda check <site>` before retrying or re-drafting (`itda check --all`
checks every site and prints one verdict line each). It runs every read
that needs no input, never sends a change, and ends each site with one
verdict and what to do:

| Verdict | Meaning | What to do |
|---|---|---|
| `ok` | every read still matches | retry the action; the failure was not the site |
| `signed_out` | the account is no longer accepted | ask the user to run `itda login <site>` |
| `site_changed` | a page or answer no longer matches its definition, the address is gone (404, 405, 410) or the site no longer accepts the query | ask the user to run `itda refresh <site>`, or call `itda_manage` with `refresh`: the site is drafted again from the sources it was set up from, the user confirms the diff, and the sign-in, the name and hand-edited actions are kept |
| `blocked` | the site refused or rate limited itda; it is paused | tell the user; they check the site and run `itda resume <site>` |
| `unreachable` | the site did not answer | wait, then run `itda check` again |

A site with several problems gets one verdict, in this order: `signed_out`,
then `site_changed`, `blocked`, `unreachable`. The exit code follows the
worst verdict across the sites checked: 10, 11, 12 or 30 as in the table
below, or 0 for `ok`. A read the site refused (`api_denied`) says nothing
about the site as a whole: the verdict stays with the other reads and the
hint names that read. `--json` adds `verdict`, one entry per site under
`sites`, and every action under `results` with the HTTP status the site
answered as `upstream_status`.

## Shell agents

Agents that only have a shell can call `itda call <site>.<action> --json` and
branch on the exit code:

| Exit | Meaning | What to do |
|---|---|---|
| 0 | ok | |
| 2 | bad input or unknown action (the remedy names the nearest one), or a read that preflight would have turned into a change (`effect_changed`) | `itda describe <site>` |
| 10 | signed out or no key | ask the user to run `itda login` |
| 11 | the page no longer matches its definition, or the site answered the query with errors (`query_failed`) | update the definition |
| 12 | the site refused; it is paused | tell the user; `itda resume` after checking |
| 13 | rate limited | wait as long as `retry_after` says, then retry the read |
| 14 | the site refused this one request as not allowed (`api_denied`) | tell the user what `provider_reason` says; do not sign in again |
| 20 | needs approval (`approval_required`), the user has not answered yet (`approval_pending`), or changes are off | ask the user to run `itda approve <id>`, or wait and call again with the same id |
| 21 | approval used, rejected, expired or withdrawn because the inputs changed | call again for a new one |
| 22 | a change was sent but its outcome is unknown | check the site before retrying |
| 23 | the `next` handle expired, or was for another call | call again without `--next` |
| 30 | network error | retry later |

With `--json`, errors are printed on stdout as
`{"status": "<code>", "message": …, "remedy": …}` plus details such as
`approval`.
