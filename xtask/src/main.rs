//! `cargo xtask check-changelog`: CHANGELOG.md has one `Unreleased` block with
//! at most one of each Keep-a-Changelog section, the newest released version
//! matches the workspace version, and the places that print the version agree.
//!
//! `cargo xtask check-layers`: every workspace crate may depend only on crates in
//! a lower layer. Cargo already rejects cycles; this keeps the direction right.
//!
//! `cargo xtask mcpb`: wrap the built binaries into MCP Bundles for Claude Desktop.
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

mod mcpb;

/// Lower number = lower layer. A new crate must be added here before it builds in CI.
const LAYERS: &[(&str, u8)] = &[
    ("itda-types", 0),
    ("itda-html", 1),
    ("itda-store", 1),
    ("itda-spec", 2),
    ("itda-policy", 3),
    ("itda-net", 4),
    ("itda-vault", 5),
    ("itda-browser", 6),
    ("itda-runtime", 7),
    ("itda-prep", 8),
    ("itda-setup", 9),
    ("itda-mcp", 10),
    ("itda-server", 10),
    ("itda-cli", 11),
    ("itda-desktop", 11),
    ("itda-sandbox", 255),
    ("xtask", 255),
];

fn layer(name: &str) -> Option<u8> {
    LAYERS.iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
}

fn check_layers() -> Result<(), Vec<String>> {
    let output = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| vec![format!("cargo metadata failed: {e}")])?;
    if !output.status.success() {
        return Err(vec![String::from_utf8_lossy(&output.stderr).into_owned()]);
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| vec![format!("cargo metadata output: {e}")])?;
    let packages = metadata["packages"].as_array().cloned().unwrap_or_default();
    let members: Vec<String> = packages
        .iter()
        .filter_map(|p| p["name"].as_str().map(str::to_owned))
        .collect();
    let mut problems = Vec::new();
    for package in &packages {
        let name = package["name"].as_str().unwrap_or_default();
        let Some(own) = layer(name) else {
            problems.push(format!(
                "{name}: not in the layer table in xtask/src/main.rs"
            ));
            continue;
        };
        for dep in package["dependencies"].as_array().into_iter().flatten() {
            let dep_name = dep["name"].as_str().unwrap_or_default();
            if dep["kind"].as_str() == Some("dev") || !members.iter().any(|m| m == dep_name) {
                continue;
            }
            match layer(dep_name) {
                Some(theirs) if theirs < own => {}
                Some(theirs) => problems.push(format!(
                    "{name} (layer {own}) depends on {dep_name} (layer {theirs}); dependencies must point to a lower layer"
                )),
                None => problems.push(format!("{dep_name}: not in the layer table")),
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

const SECTIONS: [&str; 6] = [
    "Added",
    "Changed",
    "Deprecated",
    "Removed",
    "Fixed",
    "Security",
];
const VERSION_MENTIONS: [&str; 2] = ["site/index.html", "docs/quickstart.md"];

fn check_changelog() -> Result<(), Vec<String>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut problems = Vec::new();
    let changelog = std::fs::read_to_string(root.join("CHANGELOG.md")).unwrap_or_default();
    let mut current: Option<String> = None;
    let mut seen: Vec<String> = Vec::new();
    let mut released: Option<String> = None;
    for (number, line) in changelog.lines().enumerate() {
        let number = number + 1;
        if let Some(rest) = line.strip_prefix("## [") {
            let name = rest.split(']').next().unwrap_or("").to_string();
            if name != "Unreleased" && released.is_none() {
                released = Some(name.clone());
            }
            current = Some(name);
            seen.clear();
        } else if let Some(section) = line.strip_prefix("### ") {
            let block = current.clone().unwrap_or_default();
            if !SECTIONS.contains(&section) {
                problems.push(format!(
                    "CHANGELOG.md:{number}: section `{section}` is not one of {}",
                    SECTIONS.join(", ")
                ));
            }
            if seen.contains(&section.to_string()) {
                problems.push(format!(
                    "CHANGELOG.md:{number}: `{block}` has `### {section}` twice; merge the entries under one heading"
                ));
            }
            seen.push(section.to_string());
        }
    }
    if !changelog.contains("\n## [Unreleased]\n") {
        problems.push("CHANGELOG.md: no `## [Unreleased]` block".to_string());
    }
    let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap_or_default();
    let version = cargo
        .lines()
        .find_map(|l| {
            l.strip_prefix("version = \"")
                .map(|v| v.trim_end_matches('"').to_string())
        })
        .unwrap_or_default();
    match released {
        Some(ref r) if *r == version => {}
        Some(r) => problems.push(format!(
            "Cargo.toml version {version} but the newest CHANGELOG.md release is {r}"
        )),
        None => problems.push("CHANGELOG.md has no released version".to_string()),
    }
    let mention = format!("itda {version}");
    for file in VERSION_MENTIONS {
        let text = std::fs::read_to_string(root.join(file)).unwrap_or_default();
        if !text.contains(&mention) {
            problems.push(format!("{file} does not mention `{mention}`"));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

/// `mcpb [--distrib dir] [--out dir]` packs every dist archive in `distrib`
/// (dist's `target/distrib` unless given); `mcpb --binary path --target triple
/// [--out dir]` packs one binary you built yourself. Bundles land in `out`
/// (`target/mcpb` unless given), which is where `dist-workspace.toml` expects
/// them.
fn mcpb(args: &[String]) -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("xtask has no parent directory")?
        .to_path_buf();
    // dist writes its archives under CARGO_TARGET_DIR when that is set.
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let mut distrib = target_dir.join("distrib");
    let mut out = root.join("target").join("mcpb");
    let mut binary: Option<PathBuf> = None;
    let mut target: Option<String> = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let value = args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--distrib" => distrib = PathBuf::from(value?),
            "--out" => out = PathBuf::from(value?),
            "--binary" => binary = Some(PathBuf::from(value?)),
            "--target" => target = Some(value?.clone()),
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    let version = env!("CARGO_PKG_VERSION");
    match (binary, target) {
        (Some(binary), Some(target)) => {
            let contents = mcpb::Contents {
                target: &target,
                binary: &binary,
                license: &root.join("LICENSE"),
                notice: &root.join("NOTICE"),
            };
            let path = mcpb::pack(&contents, version, &out).map_err(|e| e.to_string())?;
            let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
            println!("{target}: {} ({size} bytes)", path.display());
            Ok(())
        }
        (None, None) => mcpb::from_distrib(&distrib, &root, version, &out)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        _ => Err("--binary and --target go together".into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(2).collect();
    match std::env::args().nth(1).as_deref() {
        Some("check-changelog") => match check_changelog() {
            Ok(()) => {
                println!("changelog ok");
                ExitCode::SUCCESS
            }
            Err(problems) => {
                for problem in problems {
                    eprintln!("{problem}");
                }
                ExitCode::FAILURE
            }
        },
        Some("mcpb") => match mcpb(&args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(problem) => {
                eprintln!("{problem}");
                ExitCode::FAILURE
            }
        },
        Some("check-layers") => match check_layers() {
            Ok(()) => {
                println!("layers ok");
                ExitCode::SUCCESS
            }
            Err(problems) => {
                for problem in problems {
                    eprintln!("{problem}");
                }
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!(
                "usage: cargo xtask check-layers | check-changelog\n       cargo xtask mcpb [--distrib dir] [--out dir]\n       cargo xtask mcpb --binary path --target triple [--out dir]"
            );
            ExitCode::from(2)
        }
    }
}
