use toml;

use std::io::Read;
use std::collections::HashMap;
use std::path::{Path,PathBuf};
use std::fs::File;
use errors::*;


#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Config {
    pub bin: bool,
    pub bin_name: String,
    pub overlay: Option<PathBuf>,
    pub overlay_write_back: bool,
    pub allow_prerelease_deps: bool,

    pub source: Option<SourceOverride>,
    pub packages: Option<HashMap<String, PackageOverride>>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct SourceOverride {
    section: Option<String>,
    policy: Option<String>,
    homepage: Option<String>,
    build_depends: Option<Vec<String>>,
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

    pub fn summary_description_for(&self, pkgname: &str) -> Option<(&str, &str)> {
        match self.packages {
            Some(ref pkg) => {
                if pkg.contains_key(pkgname) {
                    let package = pkg.get(pkgname).unwrap();
                    let s = match package.summary {
                        Some(ref s) => s,
                        None => "",
                    };

                    let d = match package.description {
                        Some(ref d) => d,
                        None => "",
                    };
                    Some((s, d))
                } else {
                    None
                }
            }
            None => None,
        }
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

    pub fn build_depends(&self) -> Option<Vec<&str>> {
        if let Some(ref s) = self.source {
            if let Some(ref bdeps) = s.build_depends {
                let build_deps = bdeps.iter().map(|x| x.as_str()).collect();
                return Some(build_deps);
            }
        }
        None
    }

    pub fn section(&self) -> Option<&str> {
        if let Some(ref s) = self.source {
            if let Some(ref section) = s.section {
                return Some(section);
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
