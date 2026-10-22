# Threat model

itda runs on the user's machine and acts on websites for an AI agent. The
agent, the SiteSpec and every website response are treated as untrusted. Only
the user can approve a change or resume a paused site.

"Today" lists controls that exist in the code now. "Planned" lists controls
from the roadmap that are not built yet.

## Assets

- Website credentials and session cookies
- The user's accounts on those websites (what a change can do to them)
- The user's local network (what a request can reach)
- The record of what was done (audit trail)

## Threats and controls

| Threat | Path | Today | Planned |
|---|---|---|---|
| Agent runs changes the user did not want | Agent calls a write action, or repeats one | Reads are the default; an action is a read only when method, name, path, inputs and GraphQL agree, and a read is classified again after preflight with the found values filled in; headers and inputs that override the method are refused; changes need a single-use approval bound to the exact request, valid five minutes, granted only with `itda approve` in a terminal (piped input is refused) or a form the MCP client shows the person; the review shows the action's description and every input; an unanswered approval stays (`approval_pending`), a call with other inputs withdraws it, and logout, remove, a new login or a replaced definition withdraw every approval for the site; no MCP tool approves; the approval is used up before sending and writes are never retried; a GraphQL answer with `errors` behind a 2xx is an unknown outcome, never a success | |
| Prompt injection | Instructions hidden in a website response | Responses are checked against the declared expectation; CLI JSON and MCP results are marked untrusted with a data-not-instructions note; MCP results are cut to 24 KB | Output schemas limit fields and length |
| Credential leak | Credentials reaching the agent, logs or a model | Secret redaction for JSON and text; credential-named inputs and secret-looking defaults are refused; a refusal carries the site's status and one of a short list of reasons, never its words; short secrets live in the OS keyring and long ones (browser sessions) in `vault.sealed`, AES-256-GCM under a per-directory key kept in the keyring, with no plaintext fallback; a password typed for a site's own sign-in form is sent once to that form, on the same site, and never kept or printed; only a person in a terminal can start that sign-in, never an MCP tool; a session a site keeps in the URL (`value_from: query:<key>`) is kept in the keyring like any other and redacted from results, error text and the audit log, but it travels in the request address, so unlike a cookie it can reach the site's own server logs and any proxy in between, which itda cannot redact | Agent sees account ids only; vault passes secrets straight to the network layer; captures are redacted before any model sees them |
| Credential sent to the wrong site | Malicious spec or redirect | Header credentials bound to one origin, session credentials to one site (Public Suffix List); redirects followed only on the same origin and only for reads | |
| Local network probing | A public spec targets private addresses | Private and local addresses refused unless explicitly allowed; DNS results pinned per request | Per-host allow list for intranet use |
| Malicious or altered spec pack | Registry supply chain | Unsafe specs refused before install with named problems | Signed packs, hash-locked installs, re-consent when a pack asks for more |
| Local HTTP API abuse | A web page calls localhost | No local server exists yet | Off by default, token required, `Origin` and `Host` checks |
| Audit tampering | Hiding what was done | Append-only, hash-chained audit log; `itda audit --verify` names the first changed record; changes are logged before sending, and a change whose process stopped before an outcome was recorded is shown by `itda audit` and `itda doctor` as unknown; a change that never left the machine is reported as not sent | Signed checkpoints kept outside the device |
| Breaking a site's rules | Automating a site that refuses it, or hitting it with many requests at once | Honest user agent; one request at a time per site, spaced about 1.2 seconds apart by every itda process on the same data directory, with a definition able to ask for more room but not less; a short `Retry-After` is waited out once for reads, a longer one stops the call; block or challenge pauses the site until the user resumes it | Pack metadata for terms and robots.txt |

## Out of scope

- A compromised user account or operating system. itda cannot protect secrets
  from software already running as the user.
- An agent with unrestricted shell access as the user. It could edit the data
  directory or drive a terminal itself. The terminal prompt keeps honest agents
  from approving their own changes; it is not a sandbox.
- Websites' own vulnerabilities.

See [HONEST_CLIENT.md](HONEST_CLIENT.md) for the rules on how itda treats
websites, and [SECURITY.md](https://github.com/brnyxx/itda-cli/blob/main/SECURITY.md) to report a problem.
