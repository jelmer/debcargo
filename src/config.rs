use toml;

use std::io::Read;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs::File;
use errors::*;

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Config {
    pub bin: bool,
    pub bin_name: String,
    pub overlay: Option<PathBuf>,
    pub overlay_write_back: bool,
    pub excludes: Option<Vec<String>>,
    pub allow_prerelease_deps: bool,

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
    summary: Option<String>,
    description: Option<String>,
    depends: Option<Vec<String>>,
}

pub trait OverrideDefaults {
    fn apply_overrides(&mut self, config: &Config);
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bin: true,
            bin_name: "<default>".to_string(),
            overlay: None,
            overlay_write_back: true,
            excludes: None,
            allow_prerelease_deps: false,
            source: None,
            packages: None,
        }
    }
}

impl Config {
    pub fn is_source_present(&self) -> bool {
        self.source.is_some()
    }

    pub fn is_packages_present(&self) -> bool {
        self.packages.is_some()
    }

    pub fn orig_tar_excludes(&self) -> Vec<&str> {
        self.excludes.iter().flat_map(|vstring| {
            vstring.iter().map(String::as_str)
        }).collect()
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

    pub fn build_depends(&self) -> Vec<&str> {
        self.source.iter().flat_map(|s| {
            s.build_depends.iter().flat_map(|vstring| {
                vstring.iter().map(String::as_str)
            })
        }).collect()
    }

    pub fn build_depends_excludes(&self) -> Vec<&str> {
        self.source.iter().flat_map(|s| {
            s.build_depends_excludes.iter().flat_map(|vstring| {
                vstring.iter().map(String::as_str)
            })
        }).collect()
    }

    pub fn section(&self) -> Option<&str> {
        if let Some(ref s) = self.source {
            if let Some(ref section) = s.section {
                return Some(section);
            }
        }
        None
    }

    pub fn package_summary(&self, pkgname: &str) -> Option<(&str, &str)> {
        self.packages.as_ref().and_then(|pkg| {
            pkg.get(pkgname).map(|package| {
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

    pub fn package_depends(&self, pkgname: &str) -> Vec<&str> {
        self.packages.as_ref().and_then(|pkg| {
            pkg.get(pkgname).as_ref().and_then(|package| {
                package.depends.as_ref()
            })
        }).iter().flat_map(|deps| {
            deps.iter().map(String::as_str)
        }).collect()
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
