use cargo;
use std;
use cargo::core::{Dependency, Source, SourceId, PackageId, Summary, Registry};
use cargo::util::FileLock;
use cargo::core::{manifest, package};

use semver::Version;
use itertools::Itertools;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use errors::*;

pub struct CrateInfo {
    package: package::Package,
    manifest: manifest::Manifest,
    summary: Summary,
    crate_file: FileLock,
}

fn hash<H: Hash>(hashable: &H) -> u64 {
    #![allow(deprecated)]
    let mut hasher = std::hash::SipHasher::new();
    hashable.hash(&mut hasher);
    hasher.finish()
}

impl CrateInfo {
    pub fn new(crate_name: &str, version: Option<&str>) -> Result<CrateInfo> {
        let version = version.map(|v| if v.starts_with(|c: char| c.is_digit(10)) {
            ["=", v].concat()
        } else {
            v.to_string()
        });
        let config = cargo::Config::default()?;
        let crates_io = SourceId::crates_io(&config)?;
        let mut registry = cargo::sources::RegistrySource::remote(&crates_io, &config);
        let dependency = Dependency::parse_no_deprecated(&crate_name,
                                                         version.as_ref().map(String::as_str),
                                                         &crates_io)?;
        let summaries = registry.query(&dependency)?;
        let registry_name = format!("{}-{:016x}",
                                    crates_io.url().host_str().unwrap_or(""),
                                    hash(&crates_io).swap_bytes());




        let summary = summaries.iter()
            .max_by_key(|s| s.package_id())
            .ok_or_else(|| {
                format!("Couldn't find any crate matching {} {}\n Try `debcargo cargo-update` to \
                         update the crates.io index",
                        dependency.name(),
                        dependency.version_req())
            })?;

        let pkgid = summary.package_id();
        let package = registry.download(&pkgid)?;
        let manifest = package.manifest();
        let filename = format!("{}-{}.crate", pkgid.name(), pkgid.version());
        let crate_file = config.registry_cache_path()
            .join(&registry_name)
            .open_ro(&filename, &config, &filename)?;

        Ok(CrateInfo {
            package: package.clone(),
            manifest: manifest.clone(),
            summary: summary.clone(),
            crate_file: crate_file,
        })

    }

    pub fn targets(&self) -> &[manifest::Target] {
        self.manifest.targets()
    }

    pub fn version(&self) -> &Version {
        self.summary.package_id().version()
    }

    pub fn manifest(&self) -> &manifest::Manifest {
        &self.manifest
    }

    pub fn features(&self) -> &HashMap<String, Vec<String>> {
        self.summary.features()
    }

    pub fn checksum(&self) -> Option<&str> {
        self.summary.checksum()
    }

    pub fn package_id(&self) -> &PackageId {
        self.summary.package_id()
    }

    pub fn metadata(&self) -> &manifest::ManifestMetadata {
        self.manifest.metadata()
    }

    pub fn summary(&self) -> &Summary {
        &self.summary
    }

    pub fn package(&self) -> &package::Package {
        &self.package
    }

    pub fn crate_file(&self) -> &FileLock {
        &self.crate_file
    }

    pub fn dependencies(&self) -> &[Dependency] {
        self.manifest.dependencies()
    }

    pub fn default_deps_features(&self) -> Option<(HashSet<&str>, HashSet<&str>)> {
        let mut default_features = HashSet::new();
        let mut default_deps = HashSet::new();

        let mut defaults = Vec::new();
        let features = self.summary.features();

        defaults.push("default");
        default_features.insert("default");

        while let Some(feature) = defaults.pop() {
            match features.get(feature) {
                Some(l) => {
                    default_features.insert(feature);
                    for f in l {
                        defaults.push(f);
                    }
                }
                None => {
                    default_deps.insert(feature);
                }
            }
        }

        for (feature, deps) in features {
            if deps.is_empty() {
                default_features.insert(feature.as_str());
            }
        }

        Some((default_features, default_deps))
    }

    pub fn non_default_features(&self, default_features: &HashSet<&str>) -> Option<Vec<&str>> {
        let features = self.summary.features();
        Some(features.keys().map(String::as_str).filter(|f| !default_features.contains(f)).sorted())
    }
}
