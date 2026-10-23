//! `cargo xtask mcpb`: put the itda binary and a manifest into an MCP Bundle,
//! the `.mcpb` file Claude Desktop installs a local MCP server from. The
//! format is a zip with a `manifest.json` at its root; the fields used here
//! follow MANIFEST.md in https://github.com/modelcontextprotocol/mcpb.
//!
//! In the release workflow dist runs this once, after every platform's
//! archive has landed in `target/distrib/`, and uploads one bundle per
//! platform. Locally, `--binary` wraps a binary you built yourself.
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use zip::write::SimpleFileOptions;

/// The targets dist builds, as listed in `dist-workspace.toml`.
pub const TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
];

/// The name of the bundle for one target.
pub fn bundle_name(target: &str) -> String {
    format!("itda-{target}.mcpb")
}

/// The `compatibility.platforms` value for a target: the manifest spec uses
/// Node's `process.platform` names.
pub fn platform(target: &str) -> Option<&'static str> {
    if target.contains("apple-darwin") {
        Some("darwin")
    } else if target.contains("windows") {
        Some("win32")
    } else if target.contains("linux") {
        Some("linux")
    } else {
        None
    }
}

fn binary_path(platform: &str) -> &'static str {
    if platform == "win32" {
        "server/itda.exe"
    } else {
        "server/itda"
    }
}

/// The `manifest.json` for one target. Returns `None` for a target the
/// manifest spec has no platform name for.
pub fn manifest(target: &str, version: &str) -> Option<Value> {
    let platform = platform(target)?;
    let binary = binary_path(platform);
    Some(json!({
        "manifest_version": "0.3",
        "name": "itda",
        "display_name": "itda",
        "version": version,
        "description": "Use the websites you use through your agent: named actions in plain words, reads that just work, changes that wait for your approval.",
        "long_description": "Tell Claude \"use the-site-you-use.example\" and itda looks at the page, names what the site can do in plain words, tries every read once and keeps what works. Reads run right away; a page behind a challenge gets one more try in the browser on this computer. A change never runs until you approve it, and is sent once. Sites behind a sign-in keep their session on this computer; the agent only ever sees the account id.",
        "author": {
            "name": "the itda contributors",
            "url": "https://github.com/brnyxx/itda-cli"
        },
        "repository": {
            "type": "git",
            "url": "https://github.com/brnyxx/itda-cli.git"
        },
        "homepage": "https://github.com/brnyxx/itda-cli",
        "documentation": "https://github.com/brnyxx/itda-cli/blob/main/docs/quickstart.md",
        "support": "https://github.com/brnyxx/itda-cli/issues",
        "server": {
            "type": "binary",
            "entry_point": binary,
            "mcp_config": {
                "command": format!("${{__dirname}}/{binary}"),
                "args": ["mcp"],
                "env": {}
            }
        },
        "tools": [
            {"name": "itda_setup", "description": "Set a website up from the address of a page on it; its actions become tools in this conversation."},
            {"name": "itda_login", "description": "Open your own browser to sign in to a site that is set up; the agent only sees the account id."},
            {"name": "itda_check", "description": "Run every read that needs no input and say per site whether it is ok, signed out, changed, blocked or unreachable."},
            {"name": "itda_sites", "description": "List the sites set up on this computer with the verdict of their last check."},
            {"name": "itda_manage", "description": "Remove or resume a site after you confirm, list the changes waiting for you, or show the audit log."}
        ],
        "tools_generated": true,
        "keywords": ["websites", "actions", "approval", "audit", "intranet", "mcp"],
        "license": "Apache-2.0",
        "compatibility": {
            "platforms": [platform]
        }
    }))
}

/// What goes into a bundle besides the manifest.
pub struct Contents<'a> {
    pub target: &'a str,
    pub binary: &'a Path,
    pub license: &'a Path,
    pub notice: &'a Path,
}

/// Write `<out>/itda-<target>.mcpb` and return its path.
pub fn pack(contents: &Contents<'_>, version: &str, out: &Path) -> io::Result<PathBuf> {
    let platform = platform(contents.target).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{}: no MCPB platform for this target", contents.target),
        )
    })?;
    let manifest = manifest(contents.target, version).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{}: no manifest for this target", contents.target),
        )
    })?;
    fs::create_dir_all(out)?;
    let path = out.join(bundle_name(contents.target));
    let file = fs::File::create(&path)?;
    let mut zip = zip::ZipWriter::new(file);
    // Mark the entries as Unix on every host: a bundle built on Windows
    // would otherwise carry DOS attributes and lose the binary's execute bit.
    let text = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .system(zip::System::Unix)
        .unix_permissions(0o644);
    let executable = text.unix_permissions(0o755);

    zip.start_file("manifest.json", text)?;
    zip.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    zip.write_all(b"\n")?;
    zip.start_file(binary_path(platform), executable)?;
    copy_into(&mut zip, contents.binary)?;
    zip.start_file("LICENSE", text)?;
    copy_into(&mut zip, contents.license)?;
    zip.start_file("NOTICE", text)?;
    copy_into(&mut zip, contents.notice)?;
    zip.finish()?;
    Ok(path)
}

fn copy_into<W: Write>(zip: &mut W, path: &Path) -> io::Result<()> {
    let mut file = fs::File::open(path)
        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
    io::copy(&mut file, zip)?;
    Ok(())
}

/// Build one bundle per dist archive found in `distrib`
/// (`itda-cli-<target>.tar.xz` or `.zip`, as dist names them) and return the
/// bundles written. Fails when no archive is there.
pub fn from_distrib(
    distrib: &Path,
    root: &Path,
    version: &str,
    out: &Path,
) -> io::Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    for target in TARGETS {
        let stem = format!("itda-cli-{target}");
        let (archive, zipped) = if distrib.join(format!("{stem}.zip")).is_file() {
            (distrib.join(format!("{stem}.zip")), true)
        } else if distrib.join(format!("{stem}.tar.xz")).is_file() {
            (distrib.join(format!("{stem}.tar.xz")), false)
        } else {
            println!("{target}: no archive in {}, skipped", distrib.display());
            continue;
        };
        let scratch = out.join("extract").join(target);
        if scratch.exists() {
            fs::remove_dir_all(&scratch)?;
        }
        fs::create_dir_all(&scratch)?;
        let binary = if zipped {
            unzip_binary(&archive, &scratch)?
        } else {
            untar_binary(&archive, &stem, &scratch)?
        };
        let contents = Contents {
            target,
            binary: &binary,
            license: &root.join("LICENSE"),
            notice: &root.join("NOTICE"),
        };
        let path = pack(&contents, version, out)?;
        println!(
            "{target}: {} ({} bytes)",
            path.display(),
            fs::metadata(&path)?.len()
        );
        written.push(path);
    }
    if written.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no itda-cli archive in {}", distrib.display()),
        ));
    }
    Ok(written)
}

/// dist's Windows zip keeps `itda.exe` at its root.
fn unzip_binary(archive: &Path, scratch: &Path) -> io::Result<PathBuf> {
    let file = fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let mut entry = zip.by_name("itda.exe")?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    let path = scratch.join("itda.exe");
    fs::write(&path, bytes)?;
    Ok(path)
}

/// dist's tar.xz keeps everything under `itda-cli-<target>/`.
fn untar_binary(archive: &Path, stem: &str, scratch: &Path) -> io::Result<PathBuf> {
    let status = Command::new("tar")
        .arg("-xJf")
        .arg(archive)
        .arg("-C")
        .arg(scratch)
        .status()?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "tar -xJf {} failed with {status}",
            archive.display()
        )));
    }
    let path = scratch.join(stem).join("itda");
    if !path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{}: no itda inside", archive.display()),
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("itda-xtask-{name}-{}", std::process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir).expect("clear scratch dir");
        }
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn read_bundle(path: &Path) -> (Value, BTreeSet<String>, Option<u32>) {
        let file = fs::File::open(path).expect("open bundle");
        let mut zip = zip::ZipArchive::new(file).expect("bundle is a zip");
        let names: BTreeSet<String> = (0..zip.len())
            .map(|i| {
                zip.by_index(i)
                    .expect("entry")
                    .name()
                    .expect("entry name")
                    .into_owned()
            })
            .collect();
        let mut text = String::new();
        zip.by_name("manifest.json")
            .expect("manifest.json at the root")
            .read_to_string(&mut text)
            .expect("manifest is text");
        let manifest: Value = serde_json::from_str(&text).expect("manifest is JSON");
        let binary = names.iter().find(|n| n.starts_with("server/")).cloned();
        let mode = binary.and_then(|n| zip.by_name(&n).expect("binary entry").unix_mode());
        (manifest, names, mode)
    }

    #[test]
    fn manifest_has_the_required_fields_for_every_target() {
        for target in TARGETS {
            let m = manifest(target, "1.2.3").expect("every dist target has a platform");
            for field in [
                "manifest_version",
                "name",
                "version",
                "description",
                "author",
                "server",
            ] {
                assert!(m.get(field).is_some(), "{target}: {field} missing");
            }
            assert_eq!(m["manifest_version"], "0.3");
            assert_eq!(m["version"], "1.2.3");
            assert_eq!(m["author"]["name"], "the itda contributors");
            assert_eq!(m["server"]["type"], "binary");
            assert_eq!(m["server"]["mcp_config"]["args"], json!(["mcp"]));
            assert_eq!(m["tools_generated"], true);
            assert_eq!(
                m["compatibility"]["platforms"].as_array().map(Vec::len),
                Some(1)
            );
        }
    }

    #[test]
    fn windows_bundle_runs_the_exe_and_the_others_the_bare_binary() {
        let win = manifest("x86_64-pc-windows-msvc", "0.1.0").expect("windows manifest");
        assert_eq!(win["server"]["entry_point"], "server/itda.exe");
        assert_eq!(
            win["server"]["mcp_config"]["command"],
            "${__dirname}/server/itda.exe"
        );
        assert_eq!(win["compatibility"]["platforms"], json!(["win32"]));
        let mac = manifest("aarch64-apple-darwin", "0.1.0").expect("mac manifest");
        assert_eq!(mac["server"]["entry_point"], "server/itda");
        assert_eq!(
            mac["server"]["mcp_config"]["command"],
            "${__dirname}/server/itda"
        );
        assert_eq!(mac["compatibility"]["platforms"], json!(["darwin"]));
        let linux = manifest("x86_64-unknown-linux-gnu", "0.1.0").expect("linux manifest");
        assert_eq!(linux["compatibility"]["platforms"], json!(["linux"]));
        assert!(manifest("wasm32-unknown-unknown", "0.1.0").is_none());
    }

    #[test]
    fn pack_puts_manifest_binary_license_and_notice_in_the_bundle() {
        let dir = scratch("pack");
        let binary = dir.join("itda");
        fs::write(&binary, b"#!/bin/sh\necho itda\n").expect("write fake binary");
        let license = dir.join("LICENSE");
        let notice = dir.join("NOTICE");
        fs::write(&license, "license text").expect("write license");
        fs::write(&notice, "notice text").expect("write notice");
        let contents = Contents {
            target: "aarch64-apple-darwin",
            binary: &binary,
            license: &license,
            notice: &notice,
        };
        let path = pack(&contents, "0.9.0", &dir.join("out")).expect("pack");
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("itda-aarch64-apple-darwin.mcpb")
        );
        let (manifest, names, mode) = read_bundle(&path);
        let expected: BTreeSet<String> = ["manifest.json", "server/itda", "LICENSE", "NOTICE"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(names, expected);
        assert_eq!(manifest["version"], "0.9.0");
        assert_eq!(manifest["server"]["entry_point"], "server/itda");
        assert_eq!(mode.map(|m| m & 0o777), Some(0o755));
        fs::remove_dir_all(&dir).expect("clean up");
    }

    #[test]
    fn from_distrib_unpacks_the_windows_zip_dist_makes() {
        let dir = scratch("distrib-zip");
        let distrib = dir.join("distrib");
        fs::create_dir_all(&distrib).expect("create distrib");
        let archive = fs::File::create(distrib.join("itda-cli-x86_64-pc-windows-msvc.zip"))
            .expect("create zip");
        let mut zip = zip::ZipWriter::new(archive);
        zip.start_file("itda.exe", SimpleFileOptions::default())
            .expect("start exe");
        zip.write_all(b"MZ fake").expect("write exe");
        zip.start_file("README.md", SimpleFileOptions::default())
            .expect("start readme");
        zip.write_all(b"readme").expect("write readme");
        zip.finish().expect("finish zip");
        fs::write(dir.join("LICENSE"), "l").expect("write license");
        fs::write(dir.join("NOTICE"), "n").expect("write notice");

        let written = from_distrib(&distrib, &dir, "0.4.0", &dir.join("out")).expect("bundles");
        assert_eq!(written.len(), 1);
        let (manifest, names, _) = read_bundle(&written[0]);
        assert!(names.contains("server/itda.exe"), "{names:?}");
        assert!(!names.contains("README.md"), "only the binary comes along");
        assert_eq!(manifest["compatibility"]["platforms"], json!(["win32"]));
        fs::remove_dir_all(&dir).expect("clean up");
    }

    #[cfg(unix)]
    #[test]
    fn from_distrib_unpacks_the_tar_xz_dist_makes() {
        let dir = scratch("distrib-tar");
        let distrib = dir.join("distrib");
        let stem = "itda-cli-aarch64-apple-darwin";
        fs::create_dir_all(distrib.join(stem)).expect("create archive dir");
        fs::write(distrib.join(stem).join("itda"), b"fake").expect("write binary");
        let status = Command::new("tar")
            .arg("-cJf")
            .arg(distrib.join(format!("{stem}.tar.xz")))
            .arg("-C")
            .arg(&distrib)
            .arg(stem)
            .status()
            .expect("run tar");
        assert!(status.success(), "tar -cJf failed");
        fs::remove_dir_all(distrib.join(stem)).expect("drop the unpacked dir");
        fs::write(dir.join("LICENSE"), "l").expect("write license");
        fs::write(dir.join("NOTICE"), "n").expect("write notice");

        let written = from_distrib(&distrib, &dir, "0.4.0", &dir.join("out")).expect("bundles");
        assert_eq!(written.len(), 1);
        let (manifest, names, mode) = read_bundle(&written[0]);
        assert!(names.contains("server/itda"), "{names:?}");
        assert_eq!(manifest["compatibility"]["platforms"], json!(["darwin"]));
        assert_eq!(mode.map(|m| m & 0o777), Some(0o755));
        fs::remove_dir_all(&dir).expect("clean up");
    }

    #[test]
    fn from_distrib_fails_when_no_archive_is_there() {
        let dir = scratch("distrib-empty");
        let err = from_distrib(&dir, &dir, "0.4.0", &dir.join("out")).expect_err("nothing to pack");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        fs::remove_dir_all(&dir).expect("clean up");
    }
}
