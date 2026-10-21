//! `itda mcp install <client>`: add itda to an MCP client's configuration.
//!
//! The existing file is kept as `<file>.bak` before it changes, other servers
//! in it are left alone, and running the command again changes nothing.
use itda_types::error::{AppResult, usage};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

pub const CLIENTS: &[&str] = &["claude-desktop", "claude-code", "cursor", "vscode", "codex"];

/// How itda is started by the client.
pub struct Launch {
    pub command: String,
    pub env: Vec<(String, String)>,
}

pub enum Plan {
    /// Write `content` to `path`.
    Write { path: PathBuf, content: String },
    /// The client manages its own configuration; run this command instead.
    Run(String),
}

/// What installing into `client` would do. `user_home` is the user's home
/// directory and `cwd` the project directory for project-scoped clients.
pub fn plan(client: &str, user_home: &Path, cwd: &Path, launch: &Launch) -> AppResult<Plan> {
    let entry = || {
        let mut server = json!({"command": launch.command, "args": ["mcp"]});
        if !launch.env.is_empty() {
            server["env"] = Value::Object(
                launch
                    .env
                    .iter()
                    .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                    .collect(),
            );
        }
        server
    };
    match client {
        "claude-desktop" => json_plan(&claude_desktop_config(user_home), "mcpServers", entry()),
        "cursor" => json_plan(&user_home.join(".cursor/mcp.json"), "mcpServers", entry()),
        "vscode" => {
            let mut server = entry();
            server["type"] = "stdio".into();
            json_plan(&cwd.join(".vscode/mcp.json"), "servers", server)
        }
        "codex" => codex_plan(&user_home.join(".codex/config.toml"), launch),
        "claude-code" => {
            let env: String = launch
                .env
                .iter()
                .map(|(k, v)| format!(" -e {k}={}", quote(v)))
                .collect();
            Ok(Plan::Run(format!(
                "claude mcp add --scope user{env} itda -- {} mcp",
                quote(&launch.command)
            )))
        }
        other => Err(usage(
            format!("Unknown MCP client: {other}"),
            format!("Use one of: {}", CLIENTS.join(", ")),
        )),
    }
}

/// The clients that look installed for this person: each one's own
/// directory exists, or, for VS Code, the project has a `.vscode` folder.
pub fn detected(user_home: &Path, cwd: &Path) -> Vec<&'static str> {
    let present = |client: &str| match client {
        "claude-desktop" => claude_desktop_config(user_home)
            .parent()
            .is_some_and(Path::is_dir),
        "claude-code" => user_home.join(".claude").is_dir(),
        "cursor" => user_home.join(".cursor").is_dir(),
        "vscode" => cwd.join(".vscode").is_dir(),
        "codex" => user_home.join(".codex").is_dir(),
        _ => false,
    };
    CLIENTS.iter().copied().filter(|c| present(c)).collect()
}

fn claude_desktop_config(user_home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        user_home.join("Library/Application Support/Claude/claude_desktop_config.json")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| user_home.join("AppData/Roaming"))
            .join("Claude/claude_desktop_config.json")
    } else {
        user_home.join(".config/Claude/claude_desktop_config.json")
    }
}

fn json_plan(path: &Path, key: &str, server: Value) -> AppResult<Plan> {
    let mut config = match std::fs::read(path) {
        Ok(bytes) if !bytes.iter().all(u8::is_ascii_whitespace) => {
            serde_json::from_slice::<Value>(&bytes).map_err(|_| {
                usage(
                    format!("{} is not valid JSON", path.display()),
                    "Fix the file by hand, then run the command again.",
                )
            })?
        }
        _ => Value::Object(Map::new()),
    };
    let Some(object) = config.as_object_mut() else {
        return Err(usage(
            format!("{} does not hold a JSON object", path.display()),
            "Fix the file by hand, then run the command again.",
        ));
    };
    let servers = object
        .entry(key)
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(servers) = servers.as_object_mut() else {
        return Err(usage(
            format!("{key} in {} is not an object", path.display()),
            "Fix the file by hand, then run the command again.",
        ));
    };
    servers.insert("itda".into(), server);
    let mut content = serde_json::to_string_pretty(&config)?;
    content.push('\n');
    Ok(Plan::Write {
        path: path.to_owned(),
        content,
    })
}

/// Codex keeps TOML; the itda table is replaced, everything else is kept.
fn codex_plan(path: &Path, launch: &Launch) -> AppResult<Plan> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut kept = String::new();
    let mut skipping = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            skipping = trimmed == "[mcp_servers.itda]" || trimmed.starts_with("[mcp_servers.itda.");
        }
        if !skipping {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    let mut block = format!(
        "[mcp_servers.itda]\ncommand = {}\nargs = [\"mcp\"]\n",
        toml_string(&launch.command)
    );
    if !launch.env.is_empty() {
        let env: Vec<String> = launch
            .env
            .iter()
            .map(|(k, v)| format!("{k} = {}", toml_string(v)))
            .collect();
        block.push_str(&format!("env = {{ {} }}\n", env.join(", ")));
    }
    let kept = kept.trim_end();
    let content = if kept.is_empty() {
        block
    } else {
        format!("{kept}\n\n{block}")
    };
    Ok(Plan::Write {
        path: path.to_owned(),
        content,
    })
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
}

fn quote(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-:\\".contains(c))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

/// Write the plan, keeping the previous file as `.bak`. Returns whether the
/// file changed.
pub fn apply(path: &Path, content: &str) -> AppResult<bool> {
    let previous = std::fs::read_to_string(path).ok();
    if previous.as_deref() == Some(content) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if previous.is_some() {
        let mut backup = path.as_os_str().to_owned();
        backup.push(".bak");
        std::fs::copy(path, PathBuf::from(backup))?;
    }
    itda_types::util::atomic_file(path, content.as_bytes())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn detects_the_clients_whose_folders_exist() {
        let home = dir();
        assert!(detected(&home, &home).is_empty());
        std::fs::create_dir_all(home.join(".cursor")).unwrap();
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::create_dir_all(home.join(".vscode")).unwrap();
        assert_eq!(detected(&home, &home), ["cursor", "vscode", "codex"]);
        std::fs::remove_dir_all(home).ok();
    }

    fn launch() -> Launch {
        Launch {
            command: "/opt/itda/bin/itda".into(),
            env: vec![("ITDA_ALLOW_LOCAL".into(), "1".into())],
        }
    }

    fn dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("itda-install-{}", itda_types::util::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_plan(plan: Plan) -> (PathBuf, String) {
        match plan {
            Plan::Write { path, content } => (path, content),
            Plan::Run(_) => panic!("expected a file"),
        }
    }

    #[test]
    fn json_clients_keep_other_servers_and_back_up() {
        let home = dir();
        let path = home.join(".cursor/mcp.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"mcpServers":{"other":{"command":"x"}},"theme":"dark"}"#,
        )
        .unwrap();

        let (target, content) = write_plan(plan("cursor", &home, &home, &launch()).unwrap());
        assert_eq!(target, path);
        assert!(apply(&target, &content).unwrap());
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["mcpServers"]["other"]["command"], "x");
        assert_eq!(written["theme"], "dark");
        assert_eq!(written["mcpServers"]["itda"]["args"], json!(["mcp"]));
        assert_eq!(
            written["mcpServers"]["itda"]["env"]["ITDA_ALLOW_LOCAL"],
            "1"
        );
        assert!(home.join(".cursor/mcp.json.bak").exists());

        // Running it again changes nothing.
        let (_, again) = write_plan(plan("cursor", &home, &home, &launch()).unwrap());
        assert!(!apply(&target, &again).unwrap());
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn vscode_uses_the_project_servers_key() {
        let home = dir();
        let (path, content) = write_plan(plan("vscode", &home, &home, &launch()).unwrap());
        assert!(path.ends_with(".vscode/mcp.json"));
        let value: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(value["servers"]["itda"]["type"], "stdio");
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn codex_replaces_only_its_own_table() {
        let home = dir();
        let path = home.join(".codex/config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "model = \"o4\"\n\n[mcp_servers.itda]\ncommand = \"old\"\n\n[mcp_servers.other]\ncommand = \"x\"\n",
        )
        .unwrap();
        let (_, content) = write_plan(plan("codex", &home, &home, &launch()).unwrap());
        assert!(content.contains("model = \"o4\""));
        assert!(content.contains("[mcp_servers.other]"));
        assert!(!content.contains("\"old\""));
        assert_eq!(content.matches("[mcp_servers.itda]").count(), 1);
        assert!(content.contains("env = { ITDA_ALLOW_LOCAL = \"1\" }"));
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn broken_files_are_not_overwritten() {
        let home = dir();
        let path = home.join(".cursor/mcp.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        assert!(plan("cursor", &home, &home, &launch()).is_err());
        assert!(plan("unknown", &home, &home, &launch()).is_err());
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn claude_code_gets_a_command() {
        let home = dir();
        match plan("claude-code", &home, &home, &launch()).unwrap() {
            Plan::Run(command) => {
                assert_eq!(
                    command,
                    "claude mcp add --scope user -e ITDA_ALLOW_LOCAL=1 itda -- /opt/itda/bin/itda mcp"
                );
            }
            Plan::Write { .. } => panic!("expected a command"),
        }
        std::fs::remove_dir_all(home).ok();
    }
}
