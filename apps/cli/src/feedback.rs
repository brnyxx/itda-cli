//! `itda feedback`: a link to a new GitHub issue with the version, the
//! platform and the doctor checks filled in. itda sends nothing itself; the
//! person opens the page and decides what to post.

pub const ISSUES: &str = "https://github.com/brnyxx/itda-cli/issues/new";

/// The issue title: where it happened, with room for what happened.
pub fn title(version: &str, os: &str, arch: &str) -> String {
    format!("itda {version} on {os}/{arch}: ")
}

/// The issue body. Of each doctor check only whether it passed and what it
/// checked are kept: the detail can hold a path or an account name.
pub fn body(version: &str, os: &str, arch: &str, checks: &[(bool, &str, String)]) -> String {
    let mut text = String::from(
        "## What happened\n\n\
         <!-- What you typed, what you expected, and what itda said. \
         Remove credentials, cookies, tokens and private site data first. -->\n\n\
         ## Environment\n\n",
    );
    text.push_str(&format!("- itda {version}\n- {os}/{arch}\n"));
    for (ok, what, _) in checks {
        text.push_str(&format!(
            "- {}  {what}\n",
            if *ok { "ok  " } else { "FAIL" }
        ));
    }
    text
}

pub fn issue_url(version: &str, os: &str, arch: &str, checks: &[(bool, &str, String)]) -> String {
    url::Url::parse_with_params(
        ISSUES,
        [
            ("title", title(version, os, arch)),
            ("body", body(version, os, arch, checks)),
        ],
    )
    .map(|url| url.to_string())
    .unwrap_or_else(|_| ISSUES.to_owned())
}

/// Open the address with what the platform opens links with. The address was
/// already printed, so a failure here is not reported.
pub fn open(url: &str) {
    let _ = opener(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(not(windows))]
fn opener(url: &str) -> std::process::Command {
    let mut command = std::process::Command::new(if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    });
    command.arg(url);
    command
}

/// cmd splits a bare address at every `&`, so it gets the whole line quoted.
#[cfg(windows)]
fn opener(url: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new("cmd");
    command.raw_arg(format!("/C start \"\" \"{url}\""));
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_link_carries_the_platform_and_the_checks_but_no_path() {
        let checks = [
            (
                true,
                "data directory",
                "/Users/someone/.local/share/itda".to_owned(),
            ),
            (
                false,
                "secrets",
                "no keyring at /run/user/1000/keyring".to_owned(),
            ),
        ];
        let link = issue_url("0.4.0", "macos", "aarch64", &checks);
        let url = url::Url::parse(&link).unwrap();
        assert_eq!(
            format!(
                "{}://{}{}",
                url.scheme(),
                url.host_str().unwrap(),
                url.path()
            ),
            ISSUES
        );
        let params: std::collections::BTreeMap<String, String> =
            url.query_pairs().into_owned().collect();
        assert_eq!(params["title"], "itda 0.4.0 on macos/aarch64: ");
        let body = &params["body"];
        assert!(body.contains("## What happened"), "{body}");
        assert!(body.contains("Remove credentials"), "{body}");
        assert!(body.contains("- itda 0.4.0\n- macos/aarch64\n"), "{body}");
        assert!(body.contains("- ok    data directory\n"), "{body}");
        assert!(body.contains("- FAIL  secrets\n"), "{body}");
        assert!(!body.contains("/Users"), "{body}");
        assert!(!body.contains("/run/user"), "{body}");
        assert!(!body.contains("keyring at"), "{body}");
    }
}
