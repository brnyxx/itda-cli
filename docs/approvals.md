# Approvals and the audit log

## How a change is approved

1. An agent calls an action that changes something. itda records the exact
   request and answers `approval_required` with an id. Nothing is sent.
2. You list what is waiting with `itda approvals`, then run
   `itda approve <id>` in a terminal:

   ```console
   $ itda approve 9e96f90486f74733926ffaeb738825e2
   create-notice wants to change Sandbox intranet (sandbox) as account default:
     does         Post a notice
     request      POST http://127.0.0.1:8787/api/notices
     title        점검 공지
   Approve this change? [y/N] y
   Approved. The agent can now run it once, with exactly these inputs.
   ```

   `does` is the action's description from the site definition, followed by
   every input. itda cuts values longer than 500 characters and shows their
   length. `itda reject <id>`, or any answer but `y`, refuses it. `itda reject`
   also withdraws an approval you already granted and the agent has not used.
3. The agent calls again with `approval` set to the id. itda checks the request
   matches what you saw. It marks the approval used and sends it once.

Until you answer, a call with the id gets `approval_pending` (exit 20) and the
command to run. The approval stays unchanged; the agent waits instead of asking
for a new one. A call with inputs other than those you saw withdraws the
approval (`approval_expired` with `reason: inputs_changed`). To use those
inputs, the agent asks again for you to review them.

An approval covers one request: the site, action, account, every input, the
definition (by hash) and the credential (by fingerprint). Changing any of them
needs a new approval. It expires five minutes after the request or five minutes
after you approve it. itda uses an approval once, before sending anything, so it
never replays a failed change. `itda logout`, `itda remove`, a new `itda login`
and replacing a definition (`itda setup`, `itda refresh`, `itda add --force`,
`itda draft ... --install --force`) withdraw all pending or granted approvals
for that site or account: what you reviewed differs from what itda would send.

itda never retries changes or follows their redirects. If the answer is lost
after sending or a GraphQL answer carries `errors` behind a 2xx, the call fails
with `action_outcome_unknown` (exit 22). Check the site before trying again. A
change that never left the machine (the gate gave no turn, the name did not
resolve, the connection was refused) keeps its network code. itda reports it as
not sent.

### Approving in your MCP client

Clients with MCP elicitation forms can ask for approval there, without a switch
to a terminal:

```sh
itda mcp install claude-code --approve-in-client   # or set ITDA_APPROVE_IN_CLIENT=1
```

When an agent asks for a change, the client shows you the same review
`itda approve` prints, with an "Approve this change" box. Ticking the box and
accepting approves that one request. The call continues and runs it. Declining
or closing the form rejects it, and the agent gets `approval_rejected`. Both
decisions go into the audit log. If the client cannot show forms, nothing
changes. The agent gets the approval id; you can still use `itda approve`.

How the form reaches you depends on the protocol revision the client speaks. Up
to 2025-11-25 itda sends the client a request and waits 290 seconds for an
answer. If nobody answers, the form closes; the agent gets the approval id with
seconds left of its five minutes. From 2026-07-28 the form is the call's result.
The client keeps it open as long as it likes and calls itda again with your
answer. itda ignores answers that arrive after the approval expires and asks you
again for a new approval.

What each client showed, measured on 2026-10-22:

- **Claude Code 2.1.289** shows the form in the terminal, under "MCP server
  "itda" requests your input": the review lines, the box ("Approve this
  change"), and Accept and Decline buttons. Space ticks the box, Enter on Accept
  sends it, Esc closes the form. The agent waits ("Calling itda") and sees
  nothing until you answer. Accept runs the change; Decline and Esc both end in
  `approval_rejected`. A four-line review showed whole. A longer one showed only
  its first three lines with "(+2 more lines)" and no key to show the rest; a
  change with several inputs may not show them all. Claude Code speaks protocol
  2026-07-28 with itda. Its `Elicitation` hook can answer a form from a script
  without showing it. A hook answering itda's form would approve changes on its
  own: do not add one.
- **Claude Desktop 3.1623.1** declares no elicitation capability, so itda never
  sends it a form. Desktop shows its own "Allow once" card for the tool call.
  The agent gets `approval_required` with the id; you approve in a terminal.
  `--approve-in-client` changes nothing there.

Turn this on only for clients that put elicitation forms in front of you. The
form goes to the client application, never the model. A client that answers
forms on its own would also approve on its own.

### Why a terminal

Without the client option above, approvals come only from a terminal.
`itda approve` refuses to run when its input is not a terminal, and there is
no MCP tool that approves. An agent working through a pipe or an MCP
connection therefore cannot answer the question for you. This keeps honest
agents from approving their own changes. It is not a sandbox: software that
runs with your account and full shell access could drive a terminal or edit
the data directory. See the [threat model](THREAT_MODEL.md).

## The audit log

itda appends every call, approval, rejection and change to `audit.jsonl` in the
data directory:

```console
$ itda audit
    1  2026-10-19T08:43:01.846Z  call            sandbox.create-notice  approval_required  approval 9e96…
    2  2026-10-19T08:43:01.895Z  approved        sandbox.create-notice  ok  approval 9e96…
    3  2026-10-19T08:43:02.168Z  change_started  sandbox.create-notice  sending  approval 9e96…
    4  2026-10-19T08:43:02.187Z  call            sandbox.create-notice  ok  approval 9e96…
$ itda audit --verify
The audit log is intact: 4 records in /home/you/.local/share/itda/audit.jsonl.
```

- **Changes are written down first.** itda writes a `change_started` record
  before sending the request. If itda cannot write it, it sends no change.
- **Tampering shows.** Each record holds the hash of the one before it.
  Editing, deleting or reordering a line makes `itda audit --verify` name the
  first broken record, and `itda doctor` reports it.
- **Safe to share.** Records keep input names and a digest of their values,
  never the values or any credential.

- **A stopped process shows.** A `change_started` record with no later `call`
  record means the process stopped after sending the change. Nobody knows
  whether the site applied it. `itda audit` lists those changes at the end as
  "unknown outcome". `itda doctor` counts them under `changes`.
  `itda_manage audit` returns them under `unknown_outcome`. Check the site
  before asking for the change again.

`itda audit --json` prints the records for scripts.
