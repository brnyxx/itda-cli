//! Recognising the software a site runs, so `itda setup` can hand the agent
//! that software's real API from a ready-made template instead of reading
//! the page like any other. The page's HTML decides first (a generator
//! meta, well-known asset paths, body classes); then exactly one GET to the
//! software's own version or info endpoint, as itda, confirms it. No
//! fingerprint, no request.
use itda_html::evidence::markers;
use itda_net::{Network, Request};
use serde_json::Value;
use url::Url;

/// One kind of software and its template, `sites/software/<name>.yaml`.
#[derive(Debug)]
pub struct Software {
    /// The template's name, also what `--as` takes.
    pub name: &'static str,
    /// The name as people say it.
    pub title: &'static str,
    pub template: &'static str,
    /// Where the software keeps its API, under the instance's base.
    api_path: &'static str,
    /// What confirms the software, under the API base.
    confirm_path: &'static str,
}

pub const SOFTWARE: [Software; 8] = [
    Software {
        name: "gitlab",
        title: "GitLab",
        template: include_str!("../../../sites/software/gitlab.yaml"),
        api_path: "/api/v4",
        confirm_path: "/projects?per_page=1",
    },
    Software {
        name: "jira",
        title: "Jira",
        template: include_str!("../../../sites/software/jira.yaml"),
        api_path: "",
        confirm_path: "/rest/api/2/serverInfo",
    },
    Software {
        name: "confluence",
        title: "Confluence",
        template: include_str!("../../../sites/software/confluence.yaml"),
        api_path: "",
        confirm_path: "/rest/api/space?limit=1",
    },
    Software {
        name: "redmine",
        title: "Redmine",
        template: include_str!("../../../sites/software/redmine.yaml"),
        api_path: "",
        confirm_path: "/projects.json?limit=1",
    },
    Software {
        name: "discourse",
        title: "Discourse",
        template: include_str!("../../../sites/software/discourse.yaml"),
        api_path: "",
        confirm_path: "/site/basic-info.json",
    },
    Software {
        name: "mediawiki",
        title: "MediaWiki",
        template: include_str!("../../../sites/software/mediawiki.yaml"),
        api_path: "/w",
        confirm_path: "/api.php?action=query&meta=siteinfo&siprop=general&format=json&formatversion=2",
    },
    Software {
        name: "wordpress",
        title: "WordPress",
        template: include_str!("../../../sites/software/wordpress.yaml"),
        api_path: "/wp-json",
        confirm_path: "/",
    },
    Software {
        name: "gitea",
        title: "Gitea",
        template: include_str!("../../../sites/software/gitea.yaml"),
        api_path: "/api/v1",
        confirm_path: "/version",
    },
];

/// The template names, for `--as`.
pub fn names() -> Vec<&'static str> {
    SOFTWARE.iter().map(|s| s.name).collect()
}

pub fn by_name(name: &str) -> Option<&'static Software> {
    SOFTWARE.iter().find(|s| s.name.eq_ignore_ascii_case(name))
}

/// What the page's HTML says: which software, and where its API is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub software: &'static str,
    /// The API base: origin, context path and the API's own path.
    pub base: String,
    /// What the page called the software when it is a variant (Forgejo).
    pub title: String,
}

/// Software confirmed by its own API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recognised {
    pub software: &'static str,
    pub title: String,
    pub base: String,
    pub version: Option<String>,
    /// Whether the API answered; `false` when the template was forced and
    /// the confirm request did not answer the way the software does.
    pub confirmed: bool,
}

impl Recognised {
    pub fn definition(&self) -> &'static Software {
        by_name(self.software).unwrap_or(&SOFTWARE[0])
    }
}

/// The software a page's HTML points at, from its markers alone. `None`
/// when nothing well-known is there.
pub fn fingerprint(html: &str, page_url: &str) -> Option<Hint> {
    let page = Url::parse(page_url).ok()?;
    let found = markers(html);
    let origin = page.origin().ascii_serialization();
    let generator = found.meta("generator").unwrap_or_default();
    let hint = |software: &'static str, base: String| {
        let title = by_name(software).map(|s| s.title).unwrap_or(software);
        Some(Hint {
            software,
            base,
            title: title.to_owned(),
        })
    };

    // WordPress advertises its API root in a link.
    if let Some(href) = found.link("https://api.w.org/")
        && let Ok(api) = page.join(href)
    {
        return hint("wordpress", api.as_str().trim_end_matches('/').to_owned());
    }
    if generator.starts_with("WordPress") {
        return hint("wordpress", format!("{origin}/wp-json"));
    }

    // MediaWiki names its api.php in the EditURI link.
    if generator.starts_with("MediaWiki") || found.body_has_class("mediawiki") {
        let base = found
            .link("EditURI")
            .and_then(|href| page.join(href).ok())
            .filter(|api| api.path().ends_with("/api.php"))
            .map(|api| {
                let script_path = api.path().trim_end_matches("/api.php").to_owned();
                format!("{}{script_path}", api.origin().ascii_serialization())
            })
            .unwrap_or_else(|| format!("{origin}/w"));
        return hint("mediawiki", base);
    }

    if generator.starts_with("Discourse") {
        return hint("discourse", origin);
    }

    // Atlassian products name their context path in a meta tag.
    let context = |name: &str| {
        found
            .meta(name)
            .map(|path| path.trim_end_matches('/').to_owned())
    };
    if let Some(base) = found
        .meta("confluence-base-url")
        .map(|b| b.trim_end_matches('/').to_owned())
    {
        return hint("confluence", base);
    }
    if let Some(path) = context("confluence-context-path") {
        return hint("confluence", format!("{origin}{path}"));
    }
    if found
        .meta("application-name")
        .is_some_and(|name| name.eq_ignore_ascii_case("jira"))
    {
        let path = context("ajs-context-path").unwrap_or_default();
        return hint("jira", format!("{origin}{path}"));
    }

    // GitLab: its webpack assets, under the relative URL root when there is one.
    if found.meta("og:site_name") == Some("GitLab")
        || found
            .body_class
            .split_ascii_whitespace()
            .any(|c| c.starts_with("gl-"))
    {
        let root = found
            .scripts
            .iter()
            .find_map(|src| src.split_once("/assets/webpack/").map(|(root, _)| root))
            .and_then(|root| prefix(&page, root))
            .unwrap_or_else(|| origin.clone());
        return hint("gitlab", format!("{root}/api/v4"));
    }

    // Gitea and Forgejo: their default keywords, and appSubUrl in window.config.
    let keywords = found.meta("keywords").unwrap_or_default();
    let forge = if keywords.contains("forgejo") {
        Some("Forgejo")
    } else if keywords.contains("gitea") {
        Some("Gitea")
    } else {
        None
    };
    if let Some(title) = forge {
        let sub = found
            .inline_scripts
            .iter()
            .find_map(|script| quoted_after(script, "appSubUrl"))
            .unwrap_or_default();
        return Some(Hint {
            software: "gitea",
            base: format!("{origin}{}/api/v1", sub.trim_end_matches('/')),
            title: title.to_owned(),
        });
    }

    // Redmine: its body classes and its stylesheet, under its sub-directory.
    let redmine_body = found
        .body_class
        .split_ascii_whitespace()
        .any(|c| c.starts_with("controller-"))
        && found
            .body_class
            .split_ascii_whitespace()
            .any(|c| c.starts_with("action-"));
    let stylesheet = found.links.iter().find_map(|(_, href)| {
        href.split_once("/stylesheets/application.css")
            .map(|(root, _)| root)
    });
    if redmine_body && let Some(root) = stylesheet {
        let root = prefix(&page, root).unwrap_or_else(|| origin.clone());
        return hint("redmine", root);
    }
    None
}

/// The origin plus the path an asset address starts with: `""` and `/` are
/// the origin itself, `/gitlab` is a sub-directory, and a full address
/// keeps its own origin.
fn prefix(page: &Url, root: &str) -> Option<String> {
    if root.is_empty() || root == "/" {
        return Some(page.origin().ascii_serialization());
    }
    let url = page.join(root).ok()?;
    Some(format!(
        "{}{}",
        url.origin().ascii_serialization(),
        url.path().trim_end_matches('/')
    ))
}

/// The hint for a template the person asked for by name: the fingerprint's
/// base when the page agrees, else the API under the page's origin.
pub fn forced(software: &'static Software, html: Option<&str>, page_url: &str) -> Option<Hint> {
    if let Some(html) = html
        && let Some(hint) = fingerprint(html, page_url)
        && hint.software == software.name
    {
        return Some(hint);
    }
    let page = Url::parse(page_url).ok()?;
    Some(Hint {
        software: software.name,
        base: format!(
            "{}{}",
            page.origin().ascii_serialization(),
            software.api_path
        ),
        title: software.title.to_owned(),
    })
}

/// The value quoted after `key` in a script, as in `appSubUrl":"/git"` or
/// `appSubUrl: '/git'`.
fn quoted_after(script: &str, key: &str) -> Option<String> {
    let after = &script[script.find(key)? + key.len()..];
    let after = after
        .trim_start_matches(['"', '\''])
        .trim_start()
        .strip_prefix(':')?
        .trim_start();
    let quote = after.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let value = after[1..].split(quote).next()?;
    Some(value.to_owned())
}

/// The confirm request, exactly one GET to the software's own endpoint under
/// `hint.base`, through the gate like every other request. The answer has
/// to look the way that software answers; anything else is `None`.
pub async fn confirm(net: &Network, hint: &Hint) -> Option<Recognised> {
    let software = by_name(hint.software)?;
    let url = format!("{}{}", hint.base, software.confirm_path);
    let request = Request {
        limit: 1024 * 1024,
        ..Request::read(url)
    };
    let response = net.fetch(request).await.ok()?;
    if !response.successful() {
        return None;
    }
    let json = response.json()?;
    let (ok, version, base) = match software.name {
        "gitlab" => (json.is_array(), None, None),
        "jira" => (
            json.get("version").is_some() && json.get("baseUrl").is_some(),
            text(&json, "version"),
            text(&json, "baseUrl").map(|b| b.trim_end_matches('/').to_owned()),
        ),
        "confluence" => (json.get("results").is_some_and(Value::is_array), None, None),
        "redmine" => (
            json.get("projects").is_some_and(Value::is_array),
            None,
            None,
        ),
        "discourse" => (json.get("title").is_some(), None, None),
        "mediawiki" => {
            let general = &json["query"]["general"];
            let generator = general["generator"].as_str().unwrap_or_default();
            (
                generator.starts_with("MediaWiki"),
                Some(generator.trim_start_matches("MediaWiki ").to_owned())
                    .filter(|v| !v.is_empty()),
                None,
            )
        }
        "wordpress" => (
            json["namespaces"]
                .as_array()
                .is_some_and(|n| n.iter().any(|v| v.as_str() == Some("wp/v2"))),
            None,
            None,
        ),
        "gitea" => (json.get("version").is_some(), text(&json, "version"), None),
        _ => (false, None, None),
    };
    if !ok {
        return None;
    }
    let mut title = hint.title.clone();
    if software.name == "gitea" && version.as_deref().is_some_and(|v| v.contains("+gitea-")) {
        title = "Forgejo".into();
    }
    Some(Recognised {
        software: software.name,
        title,
        base: base.unwrap_or_else(|| hint.base.clone()),
        version,
        confirmed: true,
    })
}

fn text(json: &Value, key: &str) -> Option<String> {
    json.get(key).and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(html: &str, url: &str) -> (&'static str, String) {
        let hint = fingerprint(html, url).unwrap_or_else(|| panic!("no fingerprint for {url}"));
        (hint.software, hint.base)
    }

    #[test]
    fn wordpress_from_its_api_link_or_generator() {
        let html = "<html><head><link rel='https://api.w.org/' href='https://blog.example/news/wp-json/' />
            <meta name='generator' content='WordPress 6.6' /></head><body class='home blog'></body></html>";
        assert_eq!(
            found(html, "https://blog.example/news/"),
            ("wordpress", "https://blog.example/news/wp-json".into())
        );
        let generator_only = "<meta name='generator' content='WordPress 6.6'><body>";
        assert_eq!(
            found(generator_only, "https://blog.example/"),
            ("wordpress", "https://blog.example/wp-json".into())
        );
    }

    #[test]
    fn mediawiki_from_the_edituri_link_keeps_the_script_path() {
        let html = "<html><head><meta name='generator' content='MediaWiki 1.43.0'>
            <link rel='EditURI' type='application/rsd+xml' href='//wiki.example/w/api.php?action=rsd'>
            </head><body class='skin-vector mediawiki ltr'></body></html>";
        assert_eq!(
            found(html, "https://wiki.example/wiki/Main_Page"),
            ("mediawiki", "https://wiki.example/w".into())
        );
        let root = "<html><body class='mediawiki'></body></html>";
        assert_eq!(
            found(root, "https://wiki.example/index.php/Main_Page"),
            ("mediawiki", "https://wiki.example/w".into())
        );
    }

    #[test]
    fn discourse_from_its_generator() {
        let html = "<meta name='generator' content='Discourse 3.4.0 - https://github.com/discourse/discourse version abc'>";
        assert_eq!(
            found(html, "https://forum.example/latest"),
            ("discourse", "https://forum.example".into())
        );
    }

    #[test]
    fn jira_and_confluence_keep_their_context_paths() {
        let jira =
            "<meta name='application-name' content='JIRA' data-name='jira' data-version='10.3.26'>
            <meta name='ajs-base-url' content='https://issues.example/jira'>
            <meta name='ajs-context-path' content='/jira'>";
        assert_eq!(
            found(jira, "https://issues.example/jira/projects/HADOOP/issues"),
            ("jira", "https://issues.example/jira".into())
        );
        let cloud = "<meta name='application-name' content='JIRA' data-name='jira' data-version='1001.0.0-SNAPSHOT'>";
        assert_eq!(
            found(cloud, "https://team.atlassian.net/jira/projects"),
            ("jira", "https://team.atlassian.net".into())
        );
        let confluence = "<meta name='ajs-context-path' content='/confluence'>
            <meta id='confluence-context-path' name='confluence-context-path' content='/confluence'>
            <meta id='confluence-base-url' name='confluence-base-url' content='https://cwiki.example/confluence'>
            <meta name='ajs-version-number' content='9.2.26'>";
        assert_eq!(
            found(
                confluence,
                "https://cwiki.example/confluence/spaces/HADOOP/overview"
            ),
            ("confluence", "https://cwiki.example/confluence".into())
        );
    }

    #[test]
    fn gitlab_from_its_assets_under_a_relative_root() {
        let html = "<html><head><meta content='GitLab' property='og:site_name'>
            <meta name='csrf-param' content='authenticity_token' />
            <script src='/gitlab/assets/webpack/runtime.be4ec074.bundle.js' defer='defer'></script>
            </head><body class='tab-width-8 gl-browser-generic gl-platform-other' data-page='explore:projects:index'></body></html>";
        assert_eq!(
            found(html, "https://code.example/gitlab/explore"),
            ("gitlab", "https://code.example/gitlab/api/v4".into())
        );
        let plain = "<body class='gl-browser-generic'></body>";
        assert_eq!(
            found(plain, "https://gitlab.example/explore"),
            ("gitlab", "https://gitlab.example/api/v4".into())
        );
        // gitlab.com: assets at the root, seen from a deep page.
        let root = "<meta content='GitLab' property='og:site_name'>
            <script src='/assets/webpack/runtime.be4ec074.bundle.js'></script>
            <body class='tab-width-8 gl-browser-generic gl-platform-other'>";
        assert_eq!(
            found(root, "https://gitlab.com/explore/projects/topics"),
            ("gitlab", "https://gitlab.com/api/v4".into())
        );
    }

    #[test]
    fn gitea_and_forgejo_from_their_keywords_and_sub_url() {
        let gitea = "<html><head><meta name='keywords' content='go,git,self-hosted,gitea'>
            <script>window.config = {appUrl: 'https://git.example/code/', appSubUrl: '/code', assetUrlPrefix: '/code/assets'};</script>
            </head><body></body></html>";
        let hint = fingerprint(gitea, "https://git.example/code/explore/repos").unwrap();
        assert_eq!(hint.software, "gitea");
        assert_eq!(hint.base, "https://git.example/code/api/v1");
        assert_eq!(hint.title, "Gitea");
        let forgejo = "<meta name='keywords' content='git,forge,forgejo'><script>window.config={\"appSubUrl\":\"\"}</script>";
        let hint = fingerprint(forgejo, "https://codeberg.example/explore/repos").unwrap();
        assert_eq!(hint.base, "https://codeberg.example/api/v1");
        assert_eq!(hint.title, "Forgejo");
    }

    #[test]
    fn redmine_from_its_body_classes_and_stylesheet() {
        let html = "<html><head><meta name='csrf-param' content='authenticity_token' />
            <link rel='stylesheet' media='all' href='/redmine/stylesheets/application.css?1679302129' />
            </head><body class='project-redmine has-main-menu controller-issues action-index'><div id='top-menu'></div></body></html>";
        assert_eq!(
            found(html, "https://pm.example/redmine/projects/redmine/issues"),
            ("redmine", "https://pm.example/redmine".into())
        );
        // www.redmine.org: at the root, seen from a project page.
        let root =
            "<link rel='stylesheet' media='all' href='/stylesheets/application.css?1679302129' />
            <body class='project-redmine has-main-menu controller-issues action-index avatars-on'>";
        assert_eq!(
            found(root, "https://www.redmine.org/projects/redmine/issues"),
            ("redmine", "https://www.redmine.org".into())
        );
    }

    #[test]
    fn a_plain_page_has_no_fingerprint() {
        let html =
            "<html><head><title>Board</title><link rel='stylesheet' href='/style.css'></head>
            <body class='page'><table><tr><th>No</th></tr></table></body></html>";
        assert!(fingerprint(html, "https://board.example/").is_none());
    }

    #[test]
    fn forcing_a_template_falls_back_to_the_origin() {
        let software = by_name("gitea").unwrap();
        let hint = forced(
            software,
            Some("<title>x</title>"),
            "https://git.example/org/repo",
        )
        .unwrap();
        assert_eq!(hint.base, "https://git.example/api/v1");
        let hint = forced(
            by_name("jira").unwrap(),
            None,
            "https://issues.example/jira/browse/X-1",
        )
        .unwrap();
        assert_eq!(hint.base, "https://issues.example");
    }

    #[test]
    fn every_template_parses_and_passes_the_install_checks() {
        for software in &SOFTWARE {
            let base = format!("https://{}.test/sub", software.name);
            let draft = itda_prep::from_template(
                &itda_prep::Fill {
                    template: software.template,
                    software: software.title,
                    base_url: &base,
                },
                &[],
                None,
                &itda_prep::Options::default(),
            )
            .unwrap_or_else(|e| panic!("{}: {e:?}", software.name));
            assert!(
                draft.problems.is_empty(),
                "{}: {:?}",
                software.name,
                draft.problems
            );
            let spec = itda_spec::parse(draft.yaml.as_bytes()).unwrap();
            itda_policy::validate_spec(&spec)
                .unwrap_or_else(|e| panic!("{}: {e:?}", software.name));
            assert_eq!(spec.site.base_url, base, "{}", software.name);
            assert_eq!(spec.site.name, software.name, "{}", software.name);
            let (reads, changes): (Vec<_>, Vec<_>) = spec.operations.iter().partition(|op| {
                itda_policy::operation_effect(op, &Default::default()) == itda_policy::Effect::Read
            });
            assert!(
                (8..=14).contains(&reads.len()),
                "{}: {} reads",
                software.name,
                reads.len()
            );
            assert!(
                changes.len() <= 3,
                "{}: {} changes",
                software.name,
                changes.len()
            );
            for op in &spec.operations {
                assert!(
                    op.evidence.contains("https://"),
                    "{}.{}: evidence {}",
                    software.name,
                    op.name,
                    op.evidence
                );
                assert_eq!(
                    op.effect == itda_policy::Effect::Read,
                    reads.iter().any(|r| r.name == op.name),
                    "{}.{}: the classifier disagrees with effect",
                    software.name,
                    op.name
                );
            }
            for auth in &spec.auth {
                if auth.kind == "header" {
                    assert!(
                        auth.value_from
                            .starts_with(&format!("store:{}/", software.name)),
                        "{}: {}",
                        software.name,
                        auth.value_from
                    );
                }
            }
        }
    }

    #[test]
    fn template_secrets_follow_the_site_name() {
        let draft = itda_prep::from_template(
            &itda_prep::Fill {
                template: by_name("gitlab").unwrap().template,
                software: "GitLab",
                base_url: "https://code.example/api/v4",
            },
            &[],
            None,
            &itda_prep::Options::default(),
        )
        .unwrap();
        assert_eq!(draft.site, "code");
        assert!(draft.yaml.contains("store:code/api-key"), "{}", draft.yaml);
        assert!(
            draft.yaml.starts_with(
                "# Drafted by `itda setup` from the GitLab template for code.example."
            ),
            "{}",
            draft.yaml
        );
    }
}
