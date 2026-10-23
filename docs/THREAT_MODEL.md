# Threat model

itda runs on the user's machine and acts on websites for an AI agent. itda
treats the agent, the SiteSpec and every website response as untrusted. Only the
user can approve a change or resume a paused site.

"Today" lists controls that exist in the code now. "Planned" lists controls
from the roadmap that are not built yet.

## Assets

- Website credentials and session cookies
- The user's accounts on those websites (what a change can do to them)
- The user's local network (what a request can reach)
- The record of what was done (audit trail)
- Files on disk: attachments itda saved, and files the user sends

## Threats and controls

| Threat | Path | Today | Planned |
|---|---|---|---|
| Agent runs changes the user did not want | Agent calls a write action, or repeats one | Reads are the default. An action is a read only if method, name, path, inputs and GraphQL agree. itda reads the request it built once more before sending it (rendered address and body, GraphQL documents in them): a declared read that changes the site goes to approval with the reason, one whose effect cannot be told is not sent. itda classifies reads again after preflight with the found values filled in and refuses headers and inputs that override the method or rewrite the address. A redirect or next-page address carrying a GraphQL mutation is not followed. Changes need a single-use approval bound to the exact request, valid five minutes. Only `itda approve` in a terminal (no piped input) or a form the MCP client shows the person grants it. Review shows the action's description and every input. An unanswered approval stays (`approval_pending`); other inputs withdraw it. Logout, remove, a new login or a replaced definition withdraw every approval for the site. No MCP tool approves. itda uses up approval before sending and never retries writes. A GraphQL answer with `errors` behind a 2xx is an unknown outcome, never a success. After a lost answer itda may run the read the change names (`confirm`) to report whether it landed; that is a read, and the change is not sent again. | |
| Prompt injection | Instructions hidden in a website response | itda checks responses against the declared expectation. It marks CLI JSON and MCP results untrusted with a data-not-instructions note and cuts MCP results to 24 KB. | Output schemas limit fields and length |
| Credential leak | Credentials reaching the agent, logs or a model | itda redacts secrets in JSON and text and refuses credential-named inputs and secret-looking defaults. A refusal carries the site's status and one of a short list of reasons, never its words. Short secrets live in the OS keyring; long ones (browser sessions) live in `vault.sealed`, encrypted with AES-256-GCM and a per-directory key kept in the keyring, with no plaintext fallback. itda sends a password typed for a site's own sign-in form once to that form, on the same site, and never keeps or prints it. Only a person in a terminal can start that sign-in, never an MCP tool. itda keeps a URL session (`value_from: query:<key>`) in the keyring like any other and redacts it from results, error text and the audit log. Unlike a cookie, it travels in the request address and can reach the site's server logs and any proxy in between; itda cannot redact those logs. A session a single-page app keeps in the browser (`value_from: storage:<key>`) is read once, over the DevTools protocol, from the window you signed in to: only that origin's `localStorage` and `sessionStorage`, never another site's, never a browser profile on disk, and itda never writes anything into a page. The value is kept in the keyring, sent as the header the site's own scripts use, and redacted from results, errors and the audit log like a cookie. | The agent sees only account ids; the vault passes secrets straight to the network layer. itda redacts captures before any model sees them. |
| Another user on the machine reads the local database | A loose umask or a data directory that was made readable | The data directory is private to the user (0700 on Unix) and so is the SQLite file with its write-ahead log and shared-memory files (0600 on Unix, set when the store opens, so files left by an earlier run are tightened too). On Windows the directory's ACL covers them. | |
| Credential sent to the wrong site | Malicious spec or redirect | itda binds header credentials to one origin, session credentials to one site (Public Suffix List); follows same-origin redirects only for reads | |
| Local network probing | A public spec targets private addresses | itda refuses private and local addresses unless explicitly allowed; pins DNS results per request | Per-host allow list for intranet use |
| Malicious or altered spec pack | Registry supply chain | itda refuses unsafe specs before install and names the problems | Signed packs, hash-locked installs, re-consent when a pack asks for more |
| Another process talks to the browser worker | A web page or another program on the computer finds the worker's port and asks it for a read with the account's credentials | The worker listens on the loopback address only and speaks its own message format, not HTTP. Before any credential is sent, both sides prove they hold the secret kept in a file only this user can read, signing a fresh nonce from each side together with the generation, protocol version and the itda executable's hash. Messages are bounded in size. | |
| Audit tampering | Hiding what was done | The audit log is append-only and hash-chained. `itda audit --verify` names the first changed record. itda logs changes before sending. If a process stops before recording an outcome, `itda audit` and `itda doctor` show the change as unknown. itda reports a change that never left the machine as not sent. | Signed checkpoints kept outside the device |
| A site answers a file read with something else | A sign-in page, an error page or a part of the file in place of the attachment | itda streams a file read to a temporary name and seals it only when the status, media type, size and completeness match the definition; an HTML answer counts only when the site marks it an attachment. Nothing is kept from a failed answer. The name comes from the site's `Content-Disposition` or the address, reduced to plain characters, so it never leaves `files/<site>/` (or `--out`) or replaces a file there. The manifest carries the file's size and SHA-256; the agent gets the manifest and a link, never the bytes. | |
| A file other than the one approved is sent | An agent swaps the file after approval, or the file changes while it is being sent | The approval names the file by name, size and SHA-256 and is bound to that hash; other bytes under the same path withdraw it. itda streams the file from one open handle, checks size and modification time before each chunk and the hash at the end, and aborts the send on a mismatch (`action_outcome_unknown`, `reason: file_changed`). The audit log records the hash. A body that carries a file is always a change, and goes only to the site's own origin. | |
| Breaking a site's rules | Automating a site that refuses it, or hitting it with many requests at once | All itda processes sharing a data directory send one request at a time per site, about 1.2 seconds apart, whichever way the request goes; definitions can increase this gap, not reduce it. Reads wait out a short `Retry-After` once; a longer one stops the call. A read answered with a challenge is tried again inside the browser, once each way, then fails without another try, and a way that failed is skipped for a few minutes. itda does not solve captchas. In direct-HTTP mode a block or challenge pauses the site until the user resumes it. | Pack metadata for terms and robots.txt |
| A page escapes the browser | A script, frame or worker in the rendered page sends the account's cookies elsewhere, or reaches the local network | Every connection the browser makes goes through itda's own proxy, which resolves the name and refuses private addresses the way a direct request does. Other sites get neither the account's credentials nor the page's address as referrer. Only reads pass, plus a GraphQL query itda has read and found to be one. The page and its workers are stopped and the proxy drained before the session is handed back. | |
| A browser left behind | The CLI or the worker exits while a browser is starting or working | Each browser is started through a small supervisor process. When the worker dies, the supervisor kills the browser and removes its profile, even before the browser has reported its DevTools port. The account's turn is held until itda knows the read ended, so no second read overlaps it. The worker exits after five idle minutes. | |

## Out of scope

- A compromised user account or operating system. itda cannot protect secrets
  from software already running as the user.
- An agent with unrestricted shell access as the user. It could edit the data
  directory or drive a terminal itself. The terminal prompt keeps honest agents
  from approving their own changes; it is not a sandbox.
- Websites' own vulnerabilities.

See [HONEST_CLIENT.md](HONEST_CLIENT.md) for the rules on how itda treats
websites, and [SECURITY.md](https://github.com/brnyxx/itda-cli/blob/main/SECURITY.md) to report a problem.
