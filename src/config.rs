use toml;

use std::io::Read;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs::File;
use errors::*;
use itertools::Itertools;
use util::vec_opt_iter;

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Config {
    pub bin: Option<bool>,
    pub bin_name: String,
    pub semver_suffix: bool,
    pub overlay: Option<PathBuf>,
    pub excludes: Option<Vec<String>>,
    pub allow_prerelease_deps: bool,
    pub summary: String,
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
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bin: None,
            bin_name: "<default>".to_string(),
            semver_suffix: false,
            overlay: None,
            excludes: None,
            allow_prerelease_deps: false,
            summary: "".to_string(),
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
        self.overlay.as_ref()
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
        self.source.as_ref().and_then(|s| {
            s.build_depends.as_ref()
        })
    }

    pub fn uploaders(&self) -> Option<&Vec<String>> {
        self.uploaders.as_ref()
    }

    pub fn build_depends_excludes(&self) -> Option<&Vec<String>> {
        self.source.as_ref().and_then(|s| {
            s.build_depends_excludes.as_ref()
        })
    }

    pub fn section(&self) -> Option<&str> {
        if let Some(ref s) = self.source {
            if let Some(ref section) = s.section {
                return Some(section);
            }
        }
        None
    }

    pub fn package_section(&self, pkgname: &str) -> Option<&str> {
        self.packages.as_ref().and_then(|pkg| {
            pkg.get(pkgname).and_then(|package| {
                package.section.as_ref().map(|s| s.as_str())
            })
        })
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

    pub fn package_depends(&self, pkgname: &str) -> Option<&Vec<String>> {
        self.packages.as_ref().and_then(|pkg| {
            pkg.get(pkgname).and_then(|package| {
                package.depends.as_ref()
            })
        })
    }

    pub fn package_depends_for_feature<'a>(
        &'a self,
        feature: &'a str,
        f_depends: Vec<&'a str>)
    -> impl Iterator<Item = &str> + 'a {
        Some(feature).into_iter().chain(f_depends.into_iter()).map(move |f|
            vec_opt_iter(self.package_depends(&package_key_for_feature(f)))
        ).flatten().map(|s| s.as_str())
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

pub fn package_key_for_feature(feature: &str) -> String {
    if feature == "" {
        PACKAGE_KEY_FOR_LIB.to_string()
    } else {
        format!("{}+{}", PACKAGE_KEY_FOR_LIB, feature)
    }
}

pub const PACKAGE_KEY_FOR_BIN : &'static str = "bin";
pub const PACKAGE_KEY_FOR_LIB : &'static str = "lib";
