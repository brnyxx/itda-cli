# Ready-made sites

Site definitions you can install as they are. Each one sends requests only to
the address in its `base_url`, and every change waits for your approval.

| Site | Account | Reads | Changes |
|---|---|---|---|
| [github](github.yaml) | Optional: a token for private repositories and changes | repository, search, issues, comments, latest release | open an issue, comment |
| [npm](npm.yaml) | None | package, version, search with weekly downloads | none |
| [crates-io](crates-io.yaml) | None | crate, versions, search | none |
| [pypi](pypi.yaml) | None | package and release, with known vulnerabilities | none |

Install one from a checkout, or straight from GitHub:

```sh
itda add sites/npm.yaml
curl -sL https://raw.githubusercontent.com/brnyxx/itda-cli/main/sites/pypi.yaml | itda add -
```

`itda add` prints where the definition sends requests and which actions change
the site before anything runs. `itda check` then confirms each read still
answers the way the definition says.

For GitHub changes or private repositories, create a
[fine-grained token](https://github.com/settings/personal-access-tokens/new)
with only the repositories and permissions agents should have, then:

```sh
echo "$GITHUB_TOKEN" | itda login github
```

## Adding a site

A good shared definition reads public data or uses the site's official API,
gives every read a `response` check and a `model` so agents get small records,
and links each action's documentation in `evidence`. Open a pull request with
the file named after its `site.name`; CI checks that it is valid.
