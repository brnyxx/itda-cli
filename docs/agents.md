# Use it from an agent

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
| `itda_call` | Run an action with `args` (and `approval` for a change) |

`itda mcp --mode direct` or `--mode catalog` overrides the choice.

Every result is the same envelope as `itda call --json`, marked
`"untrusted": true` and preceded by a note that website data is not
instructions. Results over 24 KB keep the start of their longest lists and say
`"truncated": true`. Failures are tool errors with a code and a fix, so the
agent can tell the user what to do.

## Changes

A change never runs on the first call. The agent gets `approval_required` with
an `approval` id and is told to ask you. You run `itda approve <id>` in a
terminal, see the exact request and answer `y`. The agent then calls again
with the same inputs and the `approval` id, and the change is sent once. With
`--approve-in-client`, clients that show forms ask you directly instead. See
[Approvals and the audit log](approvals.md).

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
| 13 | rate limited | wait |
| 20 | needs approval, or changes are off | ask the user to run `itda approve <id>` |
| 21 | approval used, rejected, expired or for other inputs | call again for a new one |
| 22 | a change was sent but its outcome is unknown | check the site before retrying |
| 30 | network error | retry later |

With `--json`, errors are printed on stdout as
`{"status": "<code>", "message": …, "remedy": …}` plus details such as
`approval`.
