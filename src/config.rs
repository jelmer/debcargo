use serde::Deserialize;
use toml;

use crate::errors::*;
use crate::util::vec_opt_iter;

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Config {
    pub bin: Option<bool>,
    pub bin_name: String,
    pub semver_suffix: bool,
    pub overlay: Option<PathBuf>,
    pub excludes: Option<Vec<String>>,
    pub whitelist: Option<Vec<String>>,
    pub allow_prerelease_deps: bool,
    pub crate_src_path: Option<PathBuf>,
    pub summary: String,
    pub description: String,
    pub maintainer: Option<String>,
    pub uploaders: Option<Vec<String>>,

    pub source: Option<SourceOverride>,
    pub packages: Option<HashMap<String, PackageOverride>>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct SourceOverride {
    section: Option<String>,
    policy: Option<String>,
    homepage: Option<String>,
    vcs_git: Option<String>,
    vcs_browser: Option<String>,
    build_depends: Option<Vec<String>>,
    build_depends_excludes: Option<Vec<String>>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct PackageOverride {
    section: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    depends: Option<Vec<String>>,
    recommends: Option<Vec<String>>,
    suggests: Option<Vec<String>>,
    provides: Option<Vec<String>>,
    extra_lines: Option<Vec<String>>,
    test_is_broken: Option<bool>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bin: None,
            bin_name: "<default>".to_string(),
            semver_suffix: false,
            overlay: None,
            excludes: None,
            whitelist: None,
            allow_prerelease_deps: false,
            crate_src_path: None,
            summary: "".to_string(),
            description: "".to_string(),
            maintainer: None,
            uploaders: None,
            source: None,
            packages: None,
        }
    }
}

impl Config {
    pub fn build_bin_package(&self) -> bool {
        match self.bin {
            None => !self.semver_suffix,
            Some(b) => b,
        }
    }

    pub fn overlay_dir(&self, config_path: Option<&Path>) -> Option<PathBuf> {
        self.overlay
            .as_ref()
            .map(|p| config_path.unwrap().parent().unwrap().join(p))
    }

    pub fn crate_src_path(&self, config_path: Option<&Path>) -> Option<PathBuf> {
        self.crate_src_path
            .as_ref()
            .map(|p| config_path.unwrap().parent().unwrap().join(p))
    }

    pub fn is_source_present(&self) -> bool {
        self.source.is_some()
    }

    pub fn is_packages_present(&self) -> bool {
        self.packages.is_some()
    }

    pub fn orig_tar_excludes(&self) -> Option<&Vec<String>> {
        self.excludes.as_ref()
    }

    pub fn orig_tar_whitelist(&self) -> Option<&Vec<String>> {
        self.whitelist.as_ref()
    }

    pub fn policy_version(&self) -> Option<&str> {
        if let Some(ref s) = self.source {
            if let Some(ref policy) = s.policy {
                return Some(policy);
            }
        }
        None
    }

    pub fn homepage(&self) -> Option<&str> {
        if let Some(ref s) = self.source {
            if let Some(ref homepage) = s.homepage {
                return Some(homepage);
            }
        }
        None
    }

    pub fn build_depends(&self) -> Option<&Vec<String>> {
        self.source.as_ref().and_then(|s| s.build_depends.as_ref())
    }

    pub fn maintainer(&self) -> Option<&str> {
        if let Some(ref m) = self.maintainer {
            return Some(m);
        }
        None
    }

    pub fn uploaders(&self) -> Option<&Vec<String>> {
        self.uploaders.as_ref()
    }

    pub fn build_depends_excludes(&self) -> Option<&Vec<String>> {
        self.source
            .as_ref()
            .and_then(|s| s.build_depends_excludes.as_ref())
    }

    pub fn section(&self) -> Option<&str> {
        if let Some(ref s) = self.source {
            if let Some(ref section) = s.section {
                return Some(section);
            }
        }
        None
    }

    pub fn package_section(&self, key: PackageKey) -> Option<&str> {
        self.packages.as_ref().and_then(|pkg| {
            pkg.get(&package_key_string(key))
                .and_then(|package| package.section.as_ref().map(|s| s.as_str()))
        })
    }

    pub fn package_summary(&self, key: PackageKey) -> Option<(&str, &str)> {
        self.packages.as_ref().and_then(|pkg| {
            pkg.get(&package_key_string(key)).map(|package| {
                let s = match package.summary {
                    Some(ref s) => s,
                    None => "",
                };
                let d = match package.description {
                    Some(ref d) => d,
                    None => "",
                };
                (s, d)
            })
        })
    }

    pub fn package_depends(&self, key: PackageKey) -> Option<&Vec<String>> {
        self.packages.as_ref().and_then(|pkg| {
            pkg.get(&package_key_string(key))
                .and_then(|package| package.depends.as_ref())
        })
    }

    pub fn package_recommends(&self, key: PackageKey) -> Option<&Vec<String>> {
        self.packages.as_ref().and_then(|pkg| {
            pkg.get(&package_key_string(key))
                .and_then(|package| package.recommends.as_ref())
        })
    }

    pub fn package_suggests(&self, key: PackageKey) -> Option<&Vec<String>> {
        self.packages.as_ref().and_then(|pkg| {
            pkg.get(&package_key_string(key))
                .and_then(|package| package.suggests.as_ref())
        })
    }

    pub fn package_provides(&self, key: PackageKey) -> Option<&Vec<String>> {
        self.packages.as_ref().and_then(|pkg| {
            pkg.get(&package_key_string(key))
                .and_then(|package| package.provides.as_ref())
        })
    }

    pub fn package_extra_lines(&self, key: PackageKey) -> Option<&Vec<String>> {
        self.packages.as_ref().and_then(|pkg| {
            pkg.get(&package_key_string(key))
                .and_then(|package| package.extra_lines.as_ref())
        })
    }

    pub fn package_test_is_broken(&self, key: PackageKey) -> Option<bool> {
        self.packages.as_ref().and_then(|pkg| {
            pkg.get(&package_key_string(key))
                .and_then(|package| package.test_is_broken)
        })
    }

    pub fn vcs_git(&self) -> Option<&str> {
        if let Some(ref s) = self.source {
            if let Some(ref vcs_git) = s.vcs_git {
                return Some(vcs_git);
            }
        }
        None
    }

    pub fn vcs_browser(&self) -> Option<&str> {
        if let Some(ref s) = self.source {
            if let Some(ref vcs_browser) = s.vcs_browser {
                return Some(vcs_browser);
            }
        }
        None
    }
}

pub fn parse_config(src: &Path) -> Result<Config> {
    let mut config_file = File::open(src)?;
    let mut content = String::new();
    config_file.read_to_string(&mut content)?;

    Ok(toml::from_str(&content)?)
}

pub fn package_field_for_feature<'a>(
    get_field: &'a dyn Fn(PackageKey) -> Option<&'a Vec<String>>,
    feature: PackageKey,
    f_provides: &[&str],
) -> Vec<String> {
    Some(feature)
        .into_iter()
        .chain(f_provides.iter().map(|s| PackageKey::feature(s)))
        .map(move |f| vec_opt_iter(get_field(f)))
        .flatten()
        .map(|s| s.to_string())
        .collect()
}

#[derive(Clone, Copy)]
pub enum PackageKey<'a> {
    Bin,
    BareLib,
    FeatureLib(&'a str),
}

impl<'a> PackageKey<'a> {
    pub fn feature(f: &'a str) -> PackageKey<'a> {
        use self::PackageKey::*;
        if f == "" {
            BareLib
        } else {
            FeatureLib(f)
        }
    }
}

fn package_key_string(key: PackageKey) -> String {
    use self::PackageKey::*;
    match key {
        Bin => "bin".to_string(),
        BareLib => "lib".to_string(),
        FeatureLib(feature) => format!("lib+{}", feature),
    }
}
