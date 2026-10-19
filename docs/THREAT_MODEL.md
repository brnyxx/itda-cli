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
| Agent runs changes the user did not want | Agent calls a write action, or repeats one | Reads are the default; an action is a read only when method, name, path, inputs and GraphQL agree; changes need a single-use approval bound to the exact request, valid five minutes, granted only with `itda approve` in a terminal (piped input is refused); no MCP tool approves; the approval is used up before sending and writes are never retried | Desktop and MCP elicitation prompts for approval |
| Prompt injection | Instructions hidden in a website response | Responses are checked against the declared expectation; CLI JSON and MCP results are marked untrusted with a data-not-instructions note; MCP results are cut to 24 KB | Output schemas limit fields and length |
| Credential leak | Credentials reaching the agent, logs or a model | Secret redaction for JSON and text; credential-named inputs and secret-looking defaults are refused | Agent sees account ids only; vault passes secrets straight to the network layer; captures are redacted before any model sees them |
| Credential sent to the wrong site | Malicious spec or redirect | Header credentials bound to one origin, session credentials to one site (Public Suffix List); redirects followed only on the same origin and only for reads | |
| Local network probing | A public spec targets private addresses | Private and local addresses refused unless explicitly allowed; DNS results pinned per request | Per-host allow list for intranet use |
| Malicious or altered spec pack | Registry supply chain | Unsafe specs refused before install with named problems | Signed packs, hash-locked installs, re-consent when a pack asks for more |
| Local HTTP API abuse | A web page calls localhost | No local server exists yet | Off by default, token required, `Origin` and `Host` checks |
| Audit tampering | Hiding what was done | Append-only, hash-chained audit log; `itda audit --verify` names the first changed record; changes are logged before sending | Signed checkpoints kept outside the device |
| Breaking a site's rules | Automating a site that refuses it | Honest user agent; block or challenge pauses the site until the user resumes it | Pack metadata for terms and robots.txt |

## Out of scope

- A compromised user account or operating system. itda cannot protect secrets
  from software already running as the user.
- An agent with unrestricted shell access as the user. It could edit the data
  directory or drive a terminal itself. The terminal prompt keeps honest agents
  from approving their own changes; it is not a sandbox.
- Websites' own vulnerabilities.

See [HONEST_CLIENT.md](HONEST_CLIENT.md) for the rules on how itda treats
websites, and [SECURITY.md](../SECURITY.md) to report a problem.
