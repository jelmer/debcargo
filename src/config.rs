use serde::Deserialize;
use toml;

use crate::errors::*;

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const RUST_MAINT: &str =
    "Debian Rust Maintainers <pkg-rust-maintainers@alioth-lists.debian.net>";

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
    pub summary: Option<String>,
    pub description: Option<String>,
    pub maintainer: String,
    pub uploaders: Option<Vec<String>>,
    pub collapse_features: bool,
    pub requires_root: Option<String>,

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
    test_depends: Option<Vec<String>>,
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
            summary: None,
            description: None,
            maintainer: RUST_MAINT.to_string(),
            uploaders: None,
            collapse_features: false,
            source: None,
            packages: None,
            requires_root: None,
        }
    }
}

impl Config {
    pub fn parse(src: &Path) -> Result<Config> {
        let mut config_file = File::open(src)?;
        let mut content = String::new();
        config_file.read_to_string(&mut content)?;

        Ok(toml::from_str(&content)?)
    }

    pub fn build_bin_package(&self) -> bool {
        match self.bin {
            None => !self.semver_suffix,
            Some(b) => b,
        }
    }

    pub fn overlay_dir(&self, config_path: Option<&Path>) -> Option<PathBuf> {
        Some(config_path?.parent()?.join(self.overlay.as_ref()?))
    }

    pub fn crate_src_path(&self, config_path: Option<&Path>) -> Option<PathBuf> {
        Some(config_path?.parent()?.join(self.crate_src_path.as_ref()?))
    }

    pub fn orig_tar_excludes(&self) -> Option<&Vec<String>> {
        self.excludes.as_ref()
    }

    pub fn orig_tar_whitelist(&self) -> Option<&Vec<String>> {
        self.whitelist.as_ref()
    }

    pub fn maintainer(&self) -> &str {
        self.maintainer.as_str()
    }

    pub fn uploaders(&self) -> Option<&Vec<String>> {
        self.uploaders.as_ref()
    }

    pub fn requires_root(&self) -> Option<&String> {
        self.requires_root.as_ref()
    }

    // Source shortcuts

    pub fn section(&self) -> Option<&str> {
        Some(self.source.as_ref()?.section.as_ref()?)
    }

    pub fn policy_version(&self) -> Option<&str> {
        Some(self.source.as_ref()?.policy.as_ref()?)
    }

    pub fn homepage(&self) -> Option<&str> {
        Some(self.source.as_ref()?.homepage.as_ref()?)
    }

    pub fn vcs_git(&self) -> Option<&str> {
        Some(self.source.as_ref()?.vcs_git.as_ref()?)
    }

    pub fn vcs_browser(&self) -> Option<&str> {
        Some(self.source.as_ref()?.vcs_browser.as_ref()?)
    }

    pub fn build_depends(&self) -> Option<&Vec<String>> {
        self.source.as_ref()?.build_depends.as_ref()
    }

    pub fn build_depends_excludes(&self) -> Option<&Vec<String>> {
        self.source.as_ref()?.build_depends_excludes.as_ref()
    }

    // Packages shortcuts

    fn with_package<'a, T, F: FnOnce(&'a PackageOverride) -> Option<T>>(
        &'a self,
        key: PackageKey,
        f: F,
    ) -> Option<T> {
        self.packages
            .as_ref()?
            .get(&package_key_string(key))
            .and_then(f)
    }

    pub fn package_section(&self, key: PackageKey) -> Option<&str> {
        self.with_package(key, |pkg| pkg.section.as_deref())
    }

    pub fn package_summary(&self, key: PackageKey) -> Option<&str> {
        self.with_package(key, |pkg| pkg.summary.as_deref())
    }

    pub fn package_description(&self, key: PackageKey) -> Option<&str> {
        self.with_package(key, |pkg| pkg.description.as_deref())
    }

    pub fn package_depends(&self, key: PackageKey) -> Option<&Vec<String>> {
        self.with_package(key, |pkg| pkg.depends.as_ref())
    }

    pub fn package_recommends(&self, key: PackageKey) -> Option<&Vec<String>> {
        self.with_package(key, |pkg| pkg.recommends.as_ref())
    }

    pub fn package_suggests(&self, key: PackageKey) -> Option<&Vec<String>> {
        self.with_package(key, |pkg| pkg.suggests.as_ref())
    }

    pub fn package_provides(&self, key: PackageKey) -> Option<&Vec<String>> {
        self.with_package(key, |pkg| pkg.provides.as_ref())
    }

    pub fn package_extra_lines(&self, key: PackageKey) -> Option<&Vec<String>> {
        self.with_package(key, |pkg| pkg.extra_lines.as_ref())
    }

    pub fn package_test_is_broken(&self, key: PackageKey) -> Option<bool> {
        self.with_package(key, |pkg| pkg.test_is_broken)
    }

    pub fn package_test_depends(&self, key: PackageKey) -> Option<&Vec<String>> {
        self.with_package(key, |pkg| pkg.test_depends.as_ref())
    }
}

pub fn package_field_for_feature<'a>(
    get_field: &'a dyn Fn(PackageKey) -> Option<&'a Vec<String>>,
    feature: PackageKey,
    f_provides: &[&str],
) -> Vec<String> {
    Some(feature)
        .into_iter()
        .chain(f_provides.iter().map(|s| PackageKey::feature(s)))
        .map(move |f| get_field(f).into_iter().flatten())
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
        if f.is_empty() {
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

pub fn force_for_testing() -> bool {
    std::env::var("DEBCARGO_FORCE_FOR_TESTING") == Ok("1".to_string())
}
