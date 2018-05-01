use cargo::{Config,
            core::{Dependency, Package, PackageId, Registry, Source, SourceId, Summary,
                   TargetKind},
            sources::RegistrySource};
use cargo::util::FileLock;
use cargo::core::manifest;
use failure::Error;
use semver::Version;
use itertools::Itertools;
use flate2::read::GzDecoder;
use tar::Archive;
use tempdir::TempDir;

use std;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::io::{self, Read, Write};
use std::fs;

use errors::*;
use debian::deb_dep;

pub struct CratesIo {
    config: Config,
    source_id: SourceId,
}

pub struct CrateInfo {
    package: Package,
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

impl CratesIo {
    pub fn new() -> Result<Self> {
        let config = Config::default()?;
        let source_id = SourceId::crates_io(&config)?;

        Ok(CratesIo {
            config: config,
            source_id: source_id,
        })
    }

    pub fn create_dependency(&self, name: &str, version: Option<&str>) -> Result<Dependency> {
        Dependency::parse_no_deprecated(name, version, &self.source_id)
    }

    pub fn fetch_candidates(&self, dep: &Dependency) -> Result<Vec<Summary>> {
        let mut registry = self.registry();
        let mut summaries = registry.query_vec(dep)?;
        summaries.sort_by(|a, b| b.package_id().partial_cmp(a.package_id()).unwrap());
        Ok(summaries)
    }

    pub fn fetch_as_dependency(&self, dep: &Dependency) -> Result<Vec<Dependency>> {
        let summaries = self.fetch_candidates(dep)?;
        let deps: Vec<Dependency> = summaries
            .iter()
            .map(|s| {
                self.create_dependency(s.name(), Some(format!("{}", s.version()).as_str()))
                    .unwrap()
            })
            .collect();
        Ok(deps)
    }

    pub fn registry(&self) -> RegistrySource {
        RegistrySource::remote(&self.source_id, &self.config)
    }
}

impl CrateInfo {
    pub fn new(crate_name: &str, version: Option<&str>) -> Result<CrateInfo> {
        let version = version.map(|v| {
            if v.starts_with(|c: char| c.is_digit(10)) {
                ["=", v].concat()
            } else {
                v.to_string()
            }
        });

        let crates_io = CratesIo::new()?;
        let dependency = Dependency::parse_no_deprecated(
            crate_name,
            version.as_ref().map(String::as_str),
            &crates_io.source_id,
        )?;
        let summaries = crates_io.fetch_candidates(&dependency)?;

        let registry_name = format!(
            "{}-{:016x}",
            crates_io.source_id.url().host_str().unwrap_or(""),
            hash(&crates_io.source_id).swap_bytes()
        );

        let summary = summaries
            .iter()
            .max_by_key(|s| s.package_id())
            .ok_or_else(|| {
                format_err!(
                    concat!(
                        "Couldn't find any crate matching {} {}\n Try `cargo ",
                        "update` to",
                        "update the crates.io index"
                    ),
                    dependency.name(),
                    dependency.version_req()
                )
            })?;

        let pkgid = summary.package_id();

        let mut registry = crates_io.registry();
        let package = registry.download(pkgid)?;
        let manifest = package.manifest();
        let filename = format!("{}-{}.crate", pkgid.name(), pkgid.version());
        let crate_file = crates_io
            .config
            .registry_cache_path()
            .join(&registry_name)
            .open_ro(&filename, &crates_io.config, &filename)?;

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

    pub fn features(&self) -> &BTreeMap<String, Vec<String>> {
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

    pub fn package(&self) -> &Package {
        &self.package
    }

    pub fn crate_file(&self) -> &FileLock {
        &self.crate_file
    }

    pub fn dependencies(&self) -> &[Dependency] {
        self.manifest.dependencies()
    }

    pub fn default_deps_features(&self) -> (HashSet<&str>, HashSet<&str>) {
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

        (default_features, default_deps)
    }

    pub fn non_default_features(&self, default_features: &HashSet<&str>) -> Vec<&str> {
        let features = self.summary.features();
        let optional_deps = self.dependencies()
            .iter()
            .filter(|d| d.is_optional())
            .map(|d| d.name());
        features
            .keys()
            .map(String::as_str)
            .filter(|f| !default_features.contains(f))
            .chain(optional_deps)
            .collect()
    }

    pub fn is_lib(&self) -> bool {
        let mut lib = false;
        for target in self.manifest.targets() {
            match *target.kind() {
                TargetKind::Lib(_) => {
                    lib = true;
                    break;
                }
                _ => continue,
            }
        }

        lib
    }

    pub fn get_binary_targets(&self) -> Vec<&str> {
        let mut bins = Vec::new();
        for target in self.manifest.targets() {
            match *target.kind() {
                TargetKind::Bin => {
                    bins.push(target.name());
                }
                _ => continue,
            }
        }
        bins.sort();
        bins
    }

    pub fn version_suffix(&self) -> String {
        let lib = self.is_lib();
        let bins = self.get_binary_targets();

        match *self.package_id().version() {
            _ if !lib && !bins.is_empty() => "".to_string(),
            Version {
                major: 0, minor, ..
            } => format!("-0.{}", minor),
            Version { major, .. } => format!("-{}", major),
        }
    }

    pub fn dev_dependencies(&self) -> HashSet<&str> {
        use cargo::core::dependency::Kind;
        let mut dev_deps = HashSet::new();
        for dep in self.dependencies().iter() {
            if dep.kind() == Kind::Development {
                dev_deps.insert(dep.name());
            }
        }

        dev_deps
    }

    pub fn non_build_dependencies(&self) -> Result<HashMap<&str, &Dependency>> {
        let mut all_deps = HashMap::new();
        let dev_deps = self.dev_dependencies();
        for dep in self.dependencies().iter() {
            if dep.is_build() || dev_deps.contains(dep.name()) {
                continue;
            }

            if all_deps.insert(dep.name(), dep).is_some() {
                debcargo_bail!("Duplicate dependency for {}", dep.name());
            }
        }

        Ok(all_deps)
    }

    pub fn non_dev_dependencies(&self) -> Result<Vec<String>> {
        use std::iter::FromIterator;
        let (_, default_deps) = self.default_deps_features();
        let dev_deps = self.dev_dependencies();
        let mut deps = Vec::new();

        // Collect dependencies that are not [dev-dependencies] and either not
        // marked as optional or present in default_dep
        let non_devdeps = self.dependencies()
            .iter()
            .filter(|d| {
                (!dev_deps.contains(d.name())
                    && (!d.is_optional() || default_deps.contains(d.name())))
            })
            .collect::<Vec<&Dependency>>();

        let dep_names = HashSet::from_iter(non_devdeps.iter().map(|d| d.name()));

        // We generate set of deps which are not captured in non_devdeps
        let diff_deps: HashSet<_> = default_deps.difference(&dep_names).collect();

        // Lets get the pending dependency and add them to above
        for pending_dep in diff_deps {
            // Check if we are using feature of existing dependencies
            let mut tokens = pending_dep.splitn(2, '/');
            let dep_name = tokens.next().unwrap();
            for dep in self.dependencies() {
                if dep.name() == dep_name {
                    let mut tmpdep: Dependency = dep.clone();
                    match tokens.next() {
                        Some(feature) => {
                            // We are using feature of above dep in format
                            // dep/feature. So lets make  default-features =
                            // false and set features = [`feature`]
                            tmpdep.set_default_features(false);
                            tmpdep.set_features(vec![feature.to_string()]);
                        }
                        None => {}
                    }
                    deps.extend(deb_dep(&tmpdep)?);
                }
            }
        }

        for dep in non_devdeps {
            deps.extend(deb_dep(dep)?);
        }

        deps.sort();
        deps.dedup();
        Ok(deps)
    }

    pub fn optional_dependency_names(&self) -> Vec<&str> {
        self.dependencies()
            .iter()
            .filter(|d| d.is_optional())
            .map(|d| d.name())
            .collect::<Vec<&str>>()
    }

    pub fn get_summary_description(&self) -> (Option<String>, Option<String>) {
        let (summary, description) = if let Some(ref description) = self.metadata().description {
            let mut description = description.trim();
            for article in &["a ", "A ", "an ", "An ", "the ", "The "] {
                description = description.trim_left_matches(article);
            }

            let p1 = description.find('\n');
            let p2 = description.find(". ");
            match p1.into_iter().chain(p2.into_iter()).min() {
                Some(p) => {
                    let s = description[..p].trim_right_matches('.').to_string();
                    let d = description[p + 1..].trim();
                    if d.is_empty() {
                        (Some(s), None)
                    } else {
                        (Some(s), Some(d.to_string()))
                    }
                }
                None => (Some(description.trim_right_matches('.').to_string()), None),
            }
        } else {
            (None, None)
        };

        (summary, description)
    }

    pub fn get_feature_dependencies<F>(
        &self,
        feature: &str,
        deb_feature: &F,
        feature_deps: &mut Vec<String>,
    ) -> Result<()>
    where
        F: Fn(&str) -> String,
    {
        let (default_features, _) = self.default_deps_features();
        let dev_deps = self.dev_dependencies();
        let all_deps = self.non_build_dependencies()?;
        let opt_deps = self.optional_dependency_names();

        // Track the (possibly empty) additional features required for each dep, to call
        // deb_dep once for all of them.
        let mut deps_features = HashMap::new();
        let features = self.summary().features();

        if opt_deps.contains(&feature) {
            // Given feature is actually a optional dependency and not found in
            // features of crate. We insert the name of this optional dependency
            // into our map with empty features.
            deps_features.insert(feature, vec![]);
        } else {
            for dep_str in features.get(feature).unwrap() {
                let mut dep_tokens = dep_str.splitn(2, '/');
                let dep_name = dep_tokens.next().unwrap();
                match dep_tokens.next() {
                    None if features.contains_key(dep_name) => {
                        if !default_features.contains(dep_name) {
                            feature_deps
                                .push(format!("{} (= ${{binary:Version}})", deb_feature(dep_name)));
                        }
                    }
                    opt_dep_feature => {
                        deps_features
                            .entry(dep_name)
                            .or_insert_with(|| vec![])
                            .extend(opt_dep_feature.into_iter().map(String::from));
                    }
                }
            }
        }
        for (dep_name, dep_features) in deps_features.into_iter().sorted() {
            if let Some(&dep_dependency) = all_deps.get(dep_name) {
                if dep_features.is_empty() {
                    feature_deps.extend(deb_dep(dep_dependency)?);
                } else {
                    let mut dep_dependency = dep_dependency.clone();
                    let inner = dep_dependency.set_features(dep_features);
                    feature_deps.extend(deb_dep(&inner)?);
                }
            } else if dev_deps.contains(dep_name) {
                continue;
            } else {
                debcargo_bail!(
                    "Feature {} depended on non-existent dep {}",
                    feature,
                    dep_name
                );
            };
        }

        Ok(())
    }

    pub fn extract_crate(&self, path: &Path) -> Result<bool> {
        let mut archive = Archive::new(GzDecoder::new(self.crate_file.file()));
        let tempdir = TempDir::new_in(".", "debcargo")?;
        let mut source_modified = false;

        // Filter out static libraries, to avoid needing to patch all the winapi crates to remove
        // import libraries.
        let remove_path = |path: &Path| match path.extension() {
            Some(ext) if ext == "a" => true,
            _ => false,
        };

        for entry in archive.entries()? {
            let mut entry = entry?;
            if remove_path(&(entry.path()?)) {
                source_modified = true;
                continue;
            }

            if !entry.unpack_in(tempdir.path())? {
                debcargo_bail!("Crate contained path traversals via '..'");
            }
        }

        let entries = tempdir.path().read_dir()?.collect::<io::Result<Vec<_>>>()?;
        if entries.len() != 1 || !entries[0].file_type()?.is_dir() {
            let pkgid = self.package_id();
            debcargo_bail!(
                "{}-{}.crate did not unpack to a single top-level directory",
                pkgid.name(),
                pkgid.version()
            );
        }

        if let Err(e) = fs::rename(entries[0].path(), &path) {
            return Err(Error::from(Error::from(e).context(format!(
                concat!(
                    "Could not create source directory {0}\n",
                    "To regenerate,move or remove {0}"
                ),
                path.display()
            ))));
        }

        // Ensure that Cargo.toml is in standard form, e.g. does not contain
        // path dependencies, so can be built standalone (see #4030).
        let registry_toml = self.package().to_registry_toml()?;
        let mut actual_toml = String::new();
        let toml_path = path.join("Cargo.toml");
        fs::File::open(&toml_path)?.read_to_string(&mut actual_toml)?;

        if actual_toml != registry_toml {
            let old_toml_path = path.join("Cargo.toml.orig");
            fs::rename(&toml_path, &old_toml_path)?;
            fs::OpenOptions::new()
                .create(true)
                .write(true)
                .open(&toml_path)?
                .write_all(registry_toml.as_bytes())?;
            source_modified = true;
        }

        Ok(source_modified)
    }
}
