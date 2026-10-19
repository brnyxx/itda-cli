//! Where itda keeps its data on each platform.
use std::path::PathBuf;

/// `$ITDA_HOME`, or the platform data directory: `~/Library/Application
/// Support/itda` on macOS, `%APPDATA%\itda` on Windows, and
/// `$XDG_DATA_HOME/itda` or `~/.local/share/itda` elsewhere. `None` when no
/// home directory can be found.
pub fn home() -> Option<PathBuf> {
    home_from(|name| std::env::var_os(name).filter(|v| !v.is_empty()))
}

fn home_from(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    if let Some(explicit) = var("ITDA_HOME") {
        return Some(PathBuf::from(explicit));
    }
    let base = if cfg!(target_os = "macos") {
        PathBuf::from(var("HOME")?).join("Library/Application Support")
    } else if cfg!(windows) {
        PathBuf::from(var("APPDATA")?)
    } else if let Some(xdg) = var("XDG_DATA_HOME") {
        PathBuf::from(xdg)
    } else {
        PathBuf::from(var("HOME")?).join(".local/share")
    };
    Some(base.join("itda"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_home_wins() {
        let home = home_from(|name| (name == "ITDA_HOME").then(|| "/data/itda".into()));
        assert_eq!(home, Some(PathBuf::from("/data/itda")));
    }

    #[test]
    fn platform_default_needs_a_home() {
        assert_eq!(home_from(|_| None), None);
        let home = home_from(|name| match name {
            "HOME" => Some("/home/me".into()),
            "APPDATA" => Some("C:\\Users\\me\\AppData\\Roaming".into()),
            _ => None,
        });
        assert!(home.is_some_and(|path| path.ends_with("itda")));
    }
}
