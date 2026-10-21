//! Descriptions a person reads: what an action does, in their language,
//! built from what the page or the call showed rather than from its path.
//! The action id stays machine-shaped (`list-notices`); this is the `desc`
//! an agent and the person both see.

/// What an action does, with the subject it does it to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Label<'a> {
    /// Read a page by its title.
    Page { title: &'a str },
    /// Read the records a page lists.
    List { title: &'a str },
    /// Read the data a page embeds for its scripts.
    Embedded { title: &'a str },
    /// A search form on a page.
    Search { title: &'a str },
    /// A form that changes the site.
    Submit { title: &'a str },
    /// An RSS or Atom feed.
    Feed { title: &'a str },
    /// An API call, by the verb its name starts with and the resource.
    Api { verb: &'a str, resource: &'a str },
    /// A GraphQL operation.
    Graphql { name: &'a str, mutation: bool },
}

/// `ko` or `en`: the language the person set, or the page's own.
pub fn language(options: &str, page_lang: &str) -> String {
    for candidate in [options, page_lang] {
        let lower = candidate.trim().to_ascii_lowercase();
        if lower.starts_with("ko") {
            return "ko".into();
        }
        if lower.starts_with("en") {
            return "en".into();
        }
    }
    "en".into()
}

/// One line, 80 characters at most.
pub fn describe(label: &Label<'_>, lang: &str) -> String {
    let text = if lang == "ko" {
        korean(label)
    } else {
        english(label)
    };
    text.chars().take(80).collect()
}

fn korean(label: &Label<'_>) -> String {
    let subject = |s: &str| {
        if s.is_empty() {
            "페이지".to_owned()
        } else {
            s.to_owned()
        }
    };
    match label {
        Label::Page { title } => format!("{} 읽기", subject(title)),
        Label::List { title } => format!("{} 목록 읽기", subject(title)),
        Label::Embedded { title } => format!("{} 데이터 읽기", subject(title)),
        Label::Search { title } => format!("{}에서 검색", subject(title)),
        Label::Submit { title } => format!("{}의 폼 보내기", subject(title)),
        Label::Feed { title } => {
            if title.is_empty() {
                "최신 글 피드 읽기".into()
            } else {
                format!("{title} 피드 읽기")
            }
        }
        Label::Api { verb, resource } => {
            let what = resource.replace(['-', '_'], " ");
            match *verb {
                "list" => format!("{what} 목록 가져오기"),
                "get" => format!("{what} 하나 가져오기"),
                "create" => format!("{what} 만들기"),
                "update" => format!("{what} 고치기"),
                "delete" => format!("{what} 지우기"),
                "search" => format!("{what} 검색"),
                _ => format!("{what} 보내기"),
            }
        }
        Label::Graphql { name, mutation } => {
            let what = words(name);
            if *mutation {
                format!("{what} 실행 (변경)")
            } else {
                format!("{what} 조회")
            }
        }
    }
}

fn english(label: &Label<'_>) -> String {
    let subject = |s: &str| {
        if s.is_empty() {
            "the page".to_owned()
        } else {
            s.to_owned()
        }
    };
    match label {
        Label::Page { title } => format!("Read {}", subject(title)),
        Label::List { title } => format!("List what {} shows", subject(title)),
        Label::Embedded { title } => format!("Read the data behind {}", subject(title)),
        Label::Search { title } => format!("Search on {}", subject(title)),
        Label::Submit { title } => format!("Submit the form on {}", subject(title)),
        Label::Feed { title } => {
            if title.is_empty() {
                "Read the site's feed".into()
            } else {
                format!("Read the {title} feed")
            }
        }
        Label::Api { verb, resource } => {
            let what = resource.replace(['-', '_'], " ");
            match *verb {
                "list" => format!("List {what}"),
                "get" => format!("Get one of {what}"),
                "create" => format!("Create {what}"),
                "update" => format!("Update {what}"),
                "delete" => format!("Delete {what}"),
                "search" => format!("Search {what}"),
                _ => format!("Send to {what}"),
            }
        }
        Label::Graphql { name, mutation } => {
            let what = words(name);
            if *mutation {
                format!("Run {what} (changes the site)")
            } else {
                format!("Query {what}")
            }
        }
    }
}

/// `exploreProjects` and `explore-projects` both read as `explore projects`.
fn words(name: &str) -> String {
    let mut out = String::new();
    let mut previous_lower = false;
    for c in name.chars() {
        if c.is_ascii_uppercase() && previous_lower {
            out.push(' ');
        }
        if c == '-' || c == '_' {
            out.push(' ');
            previous_lower = false;
        } else {
            out.push(c.to_ascii_lowercase());
            previous_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_read_as_a_person_would_say_them() {
        assert_eq!(
            describe(&Label::List { title: "Notices" }, "en"),
            "List what Notices shows"
        );
        assert_eq!(
            describe(
                &Label::List {
                    title: "공지사항"
                },
                "ko"
            ),
            "공지사항 목록 읽기"
        );
        assert_eq!(
            describe(&Label::Search { title: "" }, "ko"),
            "페이지에서 검색"
        );
        assert_eq!(
            describe(
                &Label::Api {
                    verb: "create",
                    resource: "notice"
                },
                "en"
            ),
            "Create notice"
        );
        assert_eq!(
            describe(
                &Label::Graphql {
                    name: "exploreProjects",
                    mutation: false
                },
                "ko"
            ),
            "explore projects 조회"
        );
        assert_eq!(
            describe(
                &Label::Graphql {
                    name: "create-issue",
                    mutation: true
                },
                "en"
            ),
            "Run create issue (changes the site)"
        );
        assert_eq!(language("", "ko-KR"), "ko");
        assert_eq!(language("en", "ko"), "en");
        assert_eq!(language("", ""), "en");
        assert_eq!(
            describe(
                &Label::Page {
                    title: &"x".repeat(200)
                },
                "en"
            )
            .chars()
            .count(),
            80
        );
    }
}
