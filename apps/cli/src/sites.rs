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
            usage(
                format!("No installed site named {site}"),
                "Run `itda ls` to see installed sites.",
            )
        })?;
    read(site, &path)
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
