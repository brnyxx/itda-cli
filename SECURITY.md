# Security policy

## Reporting a vulnerability

Report it privately through GitHub's **Report a vulnerability** button on the
Security tab of this repository. Do not open a public issue.

Please include what an attacker can do, the steps to reproduce it and the
version or commit. You will get an answer within seven days. The maintainer
releases fixes before publishing the details, and credits reporters unless they
ask not to be.

## Scope

In scope: a SiteSpec, website response or AI agent causing itda to act without
the user's approval, for example:

- a change running without a matching single-use approval
- a credential reaching another origin, a log, an agent or a model
- a request reaching a private address without the user's permission
- itda contacting a blocked or challenged site again before the user resumes it

Out of scope: websites that refuse itda's requests. itda reports and pauses on
a block by design; see [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md).

## Supported versions

Before 1.0 only the latest release receives fixes.
