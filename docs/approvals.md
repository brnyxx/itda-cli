# Approvals and the audit log

## How a change is approved

1. An agent calls an action that changes something. itda records the exact
   request and answers `approval_required` with an id. Nothing is sent.
2. You list what is waiting with `itda approvals`, then run
   `itda approve <id>` in a terminal:

   ```console
   $ itda approve 9e96f90486f74733926ffaeb738825e2
   Post a notice wants to change Sandbox intranet (sandbox) as account default:
     request      POST http://127.0.0.1:8787/api/notices
     title        점검 공지
   Approve this change? [y/N] y
   Approved. The agent can now run it once, with exactly these inputs.
   ```

   `itda reject <id>`, or any answer but `y`, refuses it.
3. The agent calls again with `approval` set to the id. itda checks that the
   request is the one you saw, marks the approval used, and sends it once.

An approval covers one request: the site, action, account, every input, the
definition (by hash) and the credential (by fingerprint). Changing any of them
needs a new approval. It expires five minutes after it was requested, or
five minutes after you approve it. It works once, and it is used up before
anything is sent, so a failed change is not replayed.

Changes are never retried and never follow redirects. When the answer is lost
after sending, the call fails with `action_outcome_unknown` (exit 22): check
the site before trying again.

### Why a terminal

`itda approve` refuses to run when its input is not a terminal, and there is
no MCP tool that approves. An agent working through a pipe or an MCP
connection therefore cannot answer the question for you. This keeps honest
agents from approving their own changes. It is not a sandbox: software that
runs with your account and full shell access could drive a terminal or edit
the data directory. See the [threat model](THREAT_MODEL.md).

## The audit log

Every call, approval, rejection and change is appended to `audit.jsonl` in the
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

- **Changes are written down first.** A `change_started` record is written
  before the request goes out. If it cannot be written, the change is not
  sent.
- **Tampering shows.** Each record holds the hash of the one before it.
  Editing, deleting or reordering a line makes `itda audit --verify` name the
  first broken record, and `itda doctor` reports it.
- **Safe to share.** Records keep input names and a digest of their values,
  never the values or any credential.

`itda audit --json` prints the records for scripts.
