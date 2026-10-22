//! `cargo xtask check-layers`: every workspace crate may depend only on crates in
//! a lower layer. Cargo already rejects cycles; this keeps the direction right.
use std::process::{Command, ExitCode};

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

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
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
            eprintln!("usage: cargo xtask check-layers");
            ExitCode::from(2)
        }
    }
}
