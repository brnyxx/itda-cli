//! Installed site definitions: `<home>/specs/<site>/spec.{yaml,yml,json}`.
use itda_policy::validate_spec;
use itda_spec::Spec;
use itda_types::{
    error::{AppResult, usage},
    util,
};
use std::path::{Path, PathBuf};

const SPEC_LIMIT: usize = 4 * 1024 * 1024;
const FILE_NAMES: [&str; 3] = ["spec.yaml", "spec.yml", "spec.json"];

/// The ready-made definitions in `sites/`, built into the binary so
/// `itda add github` works without a checkout.
pub const BUNDLED: [(&str, &str); 4] = [
    ("crates-io", include_str!("../../../sites/crates-io.yaml")),
    ("github", include_str!("../../../sites/github.yaml")),
    ("npm", include_str!("../../../sites/npm.yaml")),
    ("pypi", include_str!("../../../sites/pypi.yaml")),
];

pub fn bundled(name: &str) -> Option<&'static str> {
    BUNDLED
        .iter()
        .find(|(bundled, _)| *bundled == name)
        .map(|(_, text)| *text)
}

pub struct Installed {
    pub name: String,
    pub path: PathBuf,
    pub spec: AppResult<Spec>,
}

pub fn specs_dir(home: &Path) -> PathBuf {
    home.join("specs")
}

/// Every installed site, sorted by name, each parsed and validated.
pub fn list(home: &Path) -> Vec<Installed> {
    let mut sites: Vec<Installed> = std::fs::read_dir(specs_dir(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            let path = spec_file(&entry.path())?;
            Some(Installed {
                spec: read(&name, &path),
                name,
                path,
            })
        })
        .collect();
    sites.sort_by(|a, b| a.name.cmp(&b.name));
    sites
}

/// One installed site, parsed and validated.
pub fn load(home: &Path, site: &str) -> AppResult<Spec> {
    let path = util::valid_id(site)
        .then(|| spec_file(&specs_dir(home).join(site)))
        .flatten()
        .ok_or_else(|| {
            let installed = list(home);
            let remedy = match util::nearest(site, installed.iter().map(|s| s.name.as_str())) {
                Some(close) => {
                    format!("Did you mean {close}? Run `itda ls` to see installed sites.")
                }
                None => "Run `itda ls` to see installed sites.".into(),
            };
            usage(format!("No installed site named {site}"), remedy)
        })?;
    read(site, &path)
}

/// Where an installed site's definition is; `None` when it is not installed.
pub fn path(home: &Path, site: &str) -> Option<PathBuf> {
    util::valid_id(site)
        .then(|| spec_file(&specs_dir(home).join(site)))
        .flatten()
}

fn spec_file(dir: &Path) -> Option<PathBuf> {
    FILE_NAMES
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
}

fn read(site: &str, path: &Path) -> AppResult<Spec> {
    let spec = itda_spec::parse(&util::read_bounded(path, SPEC_LIMIT)?)?;
    if spec.site.name != site {
        return Err(usage(
            format!(
                "{} declares site {}, but is installed as {site}",
                path.display(),
                spec.site.name
            ),
            "Rename the directory or the site so they match.",
        ));
    }
    validate_spec(&spec)?;
    Ok(spec)
}

/// Install a definition (YAML or JSON) under the site name it declares.
/// An installed site is only replaced with `force`.
pub fn install(home: &Path, bytes: &[u8], force: bool) -> AppResult<(Spec, PathBuf)> {
    if bytes.len() > SPEC_LIMIT {
        return Err(usage(
            "The definition is larger than 4 MB",
            "Split it into smaller sites.",
        ));
    }
    let spec = itda_spec::parse(bytes)?;
    let site = spec.site.name.clone();
    if !util::valid_id(&site) {
        return Err(usage(
            format!("{site} is not a usable site name"),
            "Use lowercase letters, digits and dashes in site.name.",
        ));
    }
    validate_spec(&spec)?;
    let dir = specs_dir(home).join(&site);
    if spec_file(&dir).is_some() && !force {
        return Err(usage(
            format!("{site} is already installed"),
            "Pass --force to replace it.",
        ));
    }
    let json = bytes.trim_ascii_start().starts_with(b"{");
    let path = dir.join(if json { "spec.json" } else { "spec.yaml" });
    util::atomic_file(&path, bytes)?;
    for name in FILE_NAMES {
        let other = dir.join(name);
        if other != path && other.is_file() {
            std::fs::remove_file(other)?;
        }
    }
    Ok((spec, path))
}

/// Remove an installed site's definition. Signed-in accounts stay until
/// `itda logout`.
pub fn remove(home: &Path, site: &str) -> AppResult<()> {
    let dir = specs_dir(home).join(site);
    if !util::valid_id(site) || spec_file(&dir).is_none() {
        return Err(usage(
            format!("No installed site named {site}"),
            "Run `itda ls` to see installed sites.",
        ));
    }
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

/// What to look at before trusting a definition: where it sends requests,
/// which actions change the site, and how it signs in.
pub fn review(spec: &Spec) -> Vec<String> {
    let mut origins: Vec<String> = std::iter::once(spec.site.base_url.as_str())
        .chain(spec.operations.iter().map(|op| op.base_url.as_str()))
        .chain(
            spec.operations
                .iter()
                .flat_map(|op| op.preflight.iter().map(|step| step.base_url.as_str())),
        )
        .filter_map(|base| url::Url::parse(base).ok())
        .map(|url| url.origin().ascii_serialization())
        .collect();
    origins.sort();
    origins.dedup();
    let changes: Vec<&str> = spec
        .operations
        .iter()
        .filter(|op| {
            itda_policy::operation_effect(op, &Default::default()) != itda_policy::Effect::Read
        })
        .map(|op| op.name.as_str())
        .collect();
    let mut lines = vec![format!("sends requests to {}", origins.join(", "))];
    lines.push(format!(
        "{}: {}, {}",
        count(spec.operations.len(), "action"),
        count(spec.operations.len() - changes.len(), "read"),
        count(changes.len(), "change")
    ));
    if !changes.is_empty() {
        lines.push(format!(
            "changes, each approved by you before it runs: {}",
            changes.join(", ")
        ));
    }
    let kinds: Vec<&str> = spec
        .auth
        .iter()
        .map(|auth| auth.kind.as_str())
        .filter(|kind| *kind != "none")
        .collect();
    if !kinds.is_empty() {
        lines.push(format!(
            "signs in with {}: run `itda login {}`",
            kinds.join(", "),
            spec.site.name
        ));
    }
    lines
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
