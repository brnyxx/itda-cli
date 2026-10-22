# Use it from an agent

For the agent itself there are two files in the repository:
[AGENTS.md](https://github.com/brnyxx/itda-cli/blob/main/AGENTS.md), a guide
to using itda well, and
[skills/itda/SKILL.md](https://github.com/brnyxx/itda-cli/blob/main/skills/itda/SKILL.md),
the same in the short form skill loaders take. The MCP server hands the agent
the same rules as its instructions.

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

Five tools are always there, so the agent never needs a terminal to set a
site up or look after it:

| Tool | Use |
|---|---|
| `itda_setup` | Set a site up from the address of a page: look at the page as itda, draft, try every read once, keep what works. The site's actions become tools in the same session: the server sends `notifications/tools/list_changed` (it declares `tools.listChanged`) and the next `tools/list` has them. A page drawn by scripts opens briefly in a browser window on your screen so itda can watch what it loads by itself (nothing is clicked or typed). Pages that need a sign-in, or still show nothing, are refused with the command you should run instead; itda never asks you to record from here. |
| `itda_login` | Open your own browser so you can sign in to a site, exactly like `itda login <site> --browser`; the session is kept in the keyring and the agent only sees the account id. A client that shows forms asks you first; the definition must name the cookie that shows a sign-in, since nobody presses Enter. |
| `itda_check` | `itda check` for one site, or every site when none is named: the verdict per site and what to do, as `itda check --json` prints them. |
| `itda_sites` | What is set up, with reads, changes and the last check's time and verdict. |
| `itda_manage` | `refresh` (draft a changed site again from its sources; the form shows what would change, and a form nobody answers counts as no), `remove` and `resume` (you confirm in a form; a client without forms gets the command to run), `approvals` (what waits for you) and `audit` (the last entries, for one site or all). |

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
terminal, see the exact request and answer `y`. The agent then calls again
with the same inputs and the `approval` id, and the change is sent once. With
`--approve-in-client`, clients that show forms ask you directly instead. See
[Approvals and the audit log](approvals.md).

A change is sent once. When the request left but no answer came back (a
timeout after sending, a dropped connection), the call fails with
`action_outcome_unknown` (exit 22) and the audit log records that outcome:
the site may or may not have applied the change. itda never resends a change
on its own, and an agent must not either. Check the site first; only a person
decides whether to ask for the change again. Reads keep their ordinary codes
(`timeout`, `request_failed`) and may be retried.

## When something that worked stops working

Run `itda check <site>` before retrying or re-drafting (`itda check --all`
checks every site and prints one verdict line each). It runs every read
that needs no input, never sends a change, and ends each site with one
verdict and what to do:

| Verdict | Meaning | What to do |
|---|---|---|
| `ok` | every read still matches | retry the action; the failure was not the site |
| `signed_out` | the account is no longer accepted | ask the user to run `itda login <site>` |
| `site_changed` | a page or answer no longer matches its definition | ask the user to run `itda refresh <site>`, or call `itda_manage` with `refresh`: the site is drafted again from the sources it was set up from, the user confirms the diff, and the sign-in, the name and hand-edited actions are kept |
| `blocked` | the site refused or rate limited itda; it is paused | tell the user; they check the site and run `itda resume <site>` |
| `unreachable` | the site did not answer | wait, then run `itda check` again |

A site with several problems gets one verdict, in this order: `signed_out`,
then `site_changed`, `blocked`, `unreachable`. The exit code follows the
worst verdict across the sites checked: 10, 11, 12 or 30 as in the table
below, or 0 for `ok`. `--json` adds `verdict`, one entry per
site under `sites`, and every action under `results`.

## Shell agents

Agents that only have a shell can call `itda call <site>.<action> --json` and
branch on the exit code:

| Exit | Meaning | What to do |
|---|---|---|
| 0 | ok | |
| 2 | bad input or unknown action | `itda ls` |
| 10 | signed out or no key | ask the user to run `itda login` |
| 11 | the page no longer matches its definition | update the definition |
| 12 | the site refused; it is paused | tell the user; `itda resume` after checking |
| 13 | rate limited | wait as long as `retry_after` says, then retry the read |
| 20 | needs approval, or changes are off | ask the user to run `itda approve <id>` |
| 21 | approval used, rejected, expired or for other inputs | call again for a new one |
| 22 | a change was sent but its outcome is unknown | check the site before retrying |
| 23 | the `next` handle expired, or was for another call | call again without `--next` |
| 30 | network error | retry later |

With `--json`, errors are printed on stdout as
`{"status": "<code>", "message": …, "remedy": …}` plus details such as
`approval`.
