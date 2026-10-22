# Ready-made sites

Site definitions you can install as they are. Each one sends requests only to
the address in its `base_url`, and every change waits for your approval.

| Site | Account | Reads | Changes |
|---|---|---|---|
| [github](github.yaml) | Optional: a token for private repositories and changes | repository, search, issues, comments, latest release | open an issue, comment |
| [npm](npm.yaml) | None | package, version, search with weekly downloads | none |
| [crates-io](crates-io.yaml) | None | crate, versions, search | none |
| [pypi](pypi.yaml) | None | package and release, with known vulnerabilities | none |

Each one is built into `itda`, so the name is enough. A file from a checkout
or a raw URL works too, for a newer copy or your own:

```sh
itda add npm
itda add sites/npm.yaml
itda add https://raw.githubusercontent.com/brnyxx/itda-cli/main/sites/pypi.yaml
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

## Software templates

[`software/`](software/) holds a definition per kind of software, not per
site: GitLab (REST v4), Jira (REST 2, Data Center and Cloud), Confluence
(REST), Redmine (REST, JSON), Discourse (the `.json` form of its pages),
MediaWiki (`api.php`), WordPress (`wp-json/wp/v2`) and Gitea with Forgejo
(`api/v1`). Each has the reads an agent needs most, up to three changes, models
for the records and that software's own way of signing in, with the
documentation of every operation in `evidence`. `itda setup <url>` picks one
when the page says it runs that software and one request to the software's
API confirms it; `itda setup --as <name>` forces one. Their `base_url` is a
placeholder the setup fills in, so `itda add` does not take them.

## Adding a site

A good shared definition reads public data or uses the site's official API,
gives every read a `response` check and a `model` so agents get small records,
and links each action's documentation in `evidence`. Open a pull request with
the file named after its `site.name`; CI checks that it is valid.
