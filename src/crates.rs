use anyhow::{format_err, Error};
use cargo::{
    core::manifest::ManifestMetadata,
    core::registry::PackageRegistry,
    core::source::MaybePackage,
    core::{
        resolver::features::CliFeatures, Dependency, EitherManifest, FeatureValue, Manifest,
        Package, PackageId, Registry, Source, SourceId, Summary, Target, TargetKind, Workspace,
    },
    ops,
    ops::{PackageOpts, Packages},
    sources::RegistrySource,
    util::{interning::InternedString, toml::read_manifest, FileLock},
    Config,
};
use filetime::{set_file_times, FileTime};
use flate2::read::GzDecoder;
use glob::Pattern;
use regex::Regex;
use semver::Version;
use tar::Archive;
use tempfile;

use std;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};
use std::path::Path;

use crate::config::testing_ignore_debpolv;
use crate::errors::*;

pub struct CrateInfo {
    // only used for to_registry_toml in extract_crate. DO NOT USE ELSEWHERE
    package: Package,
    // allows overriding package.manifest() e.g. via patches
    manifest: Manifest,
    crate_file: FileLock,
    config: Config,
    source_id: SourceId,
    excludes: Vec<Pattern>,
    includes: Vec<Pattern>,
}

pub type CrateDepInfo = BTreeMap<
    &'static str, // name of feature / optional dependency,
    // or "" for the base package w/ no default features, guaranteed to be in the map
    (
        Vec<&'static str>, // dependencies: other features (of the current package)
        Vec<Dependency>,
    ),
>;

fn hash<H: Hash>(hashable: &H) -> u64 {
    #![allow(deprecated)]
    let mut hasher = std::hash::SipHasher::new();
    hashable.hash(&mut hasher);
    hasher.finish()
}

fn fetch_candidates(registry: &mut PackageRegistry, dep: &Dependency) -> Result<Vec<Summary>> {
    let mut summaries = match registry.query_vec(dep, false) {
        std::task::Poll::Ready(res) => res?,
        std::task::Poll::Pending => {
            registry.block_until_ready()?;
            return fetch_candidates(registry, dep);
        }
    };
    summaries.sort_by(|a, b| b.package_id().partial_cmp(&a.package_id()).unwrap());
    Ok(summaries)
}

pub fn invalidate_crates_io_cache() -> Result<()> {
    let config = Config::default()?;
    let _lock = config.acquire_package_cache_lock()?;
    let source_id = SourceId::crates_io(&config)?;
    let yanked_whitelist = HashSet::new();
    let mut r = RegistrySource::remote(source_id, &yanked_whitelist, &config)?;
    r.invalidate_cache();
    Ok(())
}

pub fn crate_name_ver_to_dep(crate_name: &str, version: Option<&str>) -> Result<Dependency> {
    // note: this forces a network call
    let config = Config::default()?;
    let source_id = SourceId::crates_io(&config)?;
    let version = version.and_then(|v| {
        if v.is_empty() {
            None
        } else if v.starts_with(|c: char| c.is_digit(10)) {
            Some(["=", v].concat())
        } else {
            Some(v.to_string())
        }
    });
    Dependency::parse(crate_name, version.as_deref(), source_id)
}

pub fn show_dep(dep: &Dependency) -> String {
    format!("{} {}", dep.package_name(), dep.version_req())
}

impl CrateInfo {
    pub fn new(crate_name: &str, version: Option<&str>) -> Result<CrateInfo> {
        CrateInfo::new_with_update(crate_name, version, true)
    }

    pub fn new_with_local_crate(
        crate_name: &str,
        version: Option<&str>,
        crate_path: &Path,
    ) -> Result<CrateInfo> {
        let config = Config::default()?;
        let crate_path = crate_path.canonicalize()?;
        let source_id = SourceId::for_path(&crate_path)?;

        let (package, crate_file) = {
            let yanked_whitelist = HashSet::new();

            let mut source = source_id.load(&config, &yanked_whitelist)?;

            let package_id = match version {
                None | Some("") => {
                    let dep = Dependency::parse(crate_name, None, source_id)?;
                    let mut package_id: Option<PackageId> = None;
                    loop {
                        match source.query(&dep, &mut |p| package_id = Some(p.package_id())) {
                            std::task::Poll::Ready(res) => {
                                res?;
                                break;
                            }
                            std::task::Poll::Pending => {
                                source.block_until_ready()?;
                            }
                        }
                    }
                    package_id.unwrap()
                }
                Some(version) => PackageId::new(crate_name, version, source_id)?,
            };

            let maybe_package = source.download(package_id)?;
            let package = match maybe_package {
                MaybePackage::Ready(p) => Ok(p),
                _ => Err(format_err!(
                    "Failed to 'download' local crate {} from {}",
                    crate_name,
                    crate_path.display()
                )),
            }?;

            let crate_file = {
                let workspace = Workspace::ephemeral(package.clone(), &config, None, true)?;

                let opts = PackageOpts {
                    config: &config,
                    verify: false,
                    list: false,
                    check_metadata: true,
                    allow_dirty: true,
                    cli_features: CliFeatures::from_command_line(&[], true, false)?,
                    jobs: None,
                    targets: Vec::new(),
                    to_package: Packages::Default,
                    keep_going: false,
                };

                // as of cargo 0.41 this returns a FileLock with a temp path, instead of the one
                // it got renamed to
                if ops::package(&workspace, &opts)?.is_none() {
                    return Err(format_err!(
                        "Failed to assemble crate file for local crate {} at {}\n",
                        crate_name,
                        crate_path.display()
                    ));
                }
                let filename = format!("{}-{}.crate", crate_name, package_id.version().to_string());
                workspace
                    .target_dir()
                    .join("package")
                    .open_rw(&filename, &config, "crate file")?
            };

            (package, crate_file)
        };

        let manifest = package.manifest().clone();

        Ok(CrateInfo {
            package,
            manifest,
            crate_file,
            config,
            source_id,
            excludes: vec![],
            includes: vec![],
        })
    }

    pub fn new_with_update(
        crate_name: &str,
        version: Option<&str>,
        update: bool,
    ) -> Result<CrateInfo> {
        let dep = crate_name_ver_to_dep(crate_name, version)?;
        Self::new_from_dependency(&dep, update)
    }

    pub fn new_from_dependency(dependency: &Dependency, update: bool) -> Result<CrateInfo> {
        let mut config = Config::default()?;
        if !update {
            // unfriendly API from cargo; we'll have to make do with it for
            // now as there is no other alternative
            config.configure(
                0,
                false,
                None,
                config.frozen(),
                config.locked(),
                true, // offline
                &config.target_dir()?.map(|x| x.into_path_unlocked()),
                &[],
                &[],
            )?;
        }

        let source_id = dependency.source_id();
        let registry_name = format!(
            "{}-{:016x}",
            source_id.url().host_str().unwrap_or(""),
            hash(&source_id).swap_bytes()
        );
        let get_package_info = |config: &Config| -> Result<_> {
            let lock = config.acquire_package_cache_lock()?;
            let mut registry = PackageRegistry::new(config)?;
            registry.lock_patches();
            let summaries = fetch_candidates(&mut registry, dependency)?;
            drop(lock);
            let pkgids = summaries
                .into_iter()
                .map(|s| s.package_id())
                .collect::<Vec<_>>();
            let pkgid = pkgids.iter().max().ok_or_else(|| {
                format_err!(
                    concat!(
                        "Couldn't find any crate matching {}\n",
                        "Try `debcargo update` to update the crates.io index."
                    ),
                    show_dep(dependency)
                )
            })?;
            let pkgset = registry.get(pkgids.as_slice())?;
            let package = pkgset.get_one(*pkgid)?;
            let manifest = package.manifest();
            for f in dependency.features() {
                // apparently, if offline is set then cargo sometimes selects
                // an offline-available version that doesn't satisfy the
                // requested features. this is dumb. if it happens, then we
                // retry with online allowed.
                if !manifest.summary().features().contains_key(f) {
                    debcargo_bail!(
                        "resolve ({} {}) -> ({}) failed to pick up required feature ({})\n\
                        This can happen with very old or yanked crates. Try patching one of \
                        its dependants, to drop or update the offending dependency.",
                        dependency.package_name(),
                        dependency.version_req(),
                        pkgid,
                        f,
                    )
                }
            }
            let filename = format!("{}-{}.crate", pkgid.name(), pkgid.version());
            let crate_file = config
                .registry_cache_path()
                .join(&registry_name)
                .open_ro(&filename, config, &filename)?;
            Ok((package.clone(), manifest.clone(), crate_file))
        };
        // if update is false but the user never downloaded the crate then the
        // first call will error; re-try with online in that case
        let (package, manifest, crate_file) =
            get_package_info(&config).or_else(|_| get_package_info(&Config::default()?))?;

        Ok(CrateInfo {
            package,
            manifest,
            crate_file,
            config,
            source_id,
            excludes: vec![],
            includes: vec![],
        })
    }

    pub fn crate_name(&self) -> &'static str {
        self.package_id().name().as_str()
    }

    pub fn version(&self) -> &Version {
        self.package_id().version()
    }

    pub fn semver(&self) -> String {
        match *self.package_id().version() {
            Version {
                major: 0, minor, ..
            } => format!("0.{}", minor),
            Version { major, .. } => format!("{}", major),
        }
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn replace_manifest(&mut self, path: &Path) -> Result<&Self> {
        if let (EitherManifest::Real(v), _) = read_manifest(path, self.source_id, &self.config)? {
            self.manifest = v;
        }
        Ok(self)
    }

    pub fn metadata(&self) -> &ManifestMetadata {
        self.manifest.metadata()
    }

    pub fn manifest_path(&self) -> &Path {
        self.package.manifest_path()
    }

    pub fn targets(&self) -> &[Target] {
        self.manifest.targets()
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
        bins.sort_unstable();
        bins
    }

    pub fn summary(&self) -> &Summary {
        self.manifest.summary()
    }

    pub fn checksum(&self) -> Option<&str> {
        self.manifest.summary().checksum()
    }

    pub fn package_id(&self) -> PackageId {
        self.manifest.summary().package_id()
    }

    pub fn crate_file(&self) -> &FileLock {
        &self.crate_file
    }

    pub fn dependencies(&self) -> &[Dependency] {
        self.manifest.dependencies()
    }

    pub fn dev_dependencies(&self) -> Vec<Dependency> {
        use cargo::core::dependency::DepKind;
        let mut deps = vec![];
        for dep in self.dependencies() {
            if dep.kind() == DepKind::Development {
                deps.push(dep.clone())
            }
        }
        deps
    }

    /// Collect information about the dependency structure of features and
    /// their external crate dependencies, in a simple output format.
    pub fn all_dependencies_and_features(&self) -> CrateDepInfo {
        use cargo::core::dependency::DepKind;

        let mut deps_by_name: BTreeMap<&str, Vec<&Dependency>> = BTreeMap::new();
        for dep in self.dependencies() {
            // we treat build-dependencies also as dependencies in Debian
            if dep.kind() != DepKind::Development {
                let s = dep.name_in_toml().as_str();
                deps_by_name.entry(s).or_default().push(dep);
            }
        }
        let deps_by_name = deps_by_name;

        let mut features_with_deps = BTreeMap::new();

        // calculate dependencies of this crate's features
        for (feature, deps) in self.manifest.summary().features() {
            let mut feature_deps: Vec<&'static str> = vec![];
            let mut other_deps: Vec<Dependency> = Vec::new();
            for dep in deps {
                use self::FeatureValue::*;
                match dep {
                    // another feature is a dependency
                    Feature(dep_feature) => {
                        feature_deps.push(InternedString::new(dep_feature).as_str())
                    }
                    // another package is a dependency
                    Dep { dep_name } => {
                        // unwrap is ok, valid Cargo.toml files must have this
                        for &dep in deps_by_name.get(dep_name.as_str()).unwrap() {
                            other_deps.push(dep.clone());
                        }
                    }
                    // another package is a dependency
                    DepFeature {
                        dep_name,
                        dep_feature,
                        ..
                    } => match deps_by_name.get(dep_name.as_str()) {
                        // unwrap is ok, valid Cargo.toml files must have this
                        Some(dd) => {
                            for &dep in dd {
                                let mut dep = dep.clone();
                                dep.set_features(vec![InternedString::new(dep_feature)]);
                                dep.set_default_features(false);
                                other_deps.push(dep);
                            }
                        }
                        None => {
                            let mut expected = false;
                            for dep in self.dependencies() {
                                if dep.kind() == DepKind::Development {
                                    let s = dep.name_in_toml().as_str();
                                    if s == dep_name.as_str() {
                                        expected = true;
                                    }
                                }
                            }
                            if expected {
                                debcargo_warn!(
                                    "Ignoring \"{}\" feature \"{}\" as it depends on a \
                                     dev-dependency \"{}\"",
                                    self.package_id(),
                                    feature,
                                    dep_name
                                );
                            } else {
                                panic!(
                                    "failed to account for dependency \"{}\" of \"{}\" feature \"{}\"",
                                    dep_name, self.package_id(), feature
                                );
                            }
                        }
                    },
                }
            }
            if feature_deps.is_empty() {
                // everything depends on bare library
                feature_deps.push("");
            }
            features_with_deps.insert(feature.as_str(), (feature_deps, other_deps));
        }

        // calculate dependencies of this crate's "optional dependencies", since they are also features
        let mut deps_required: Vec<Dependency> = Vec::new();
        for deps in deps_by_name.values() {
            for &dep in deps {
                if dep.is_optional() {
                    features_with_deps
                        .insert(dep.name_in_toml().as_str(), (vec![""], vec![dep.clone()]));
                } else {
                    deps_required.push(dep.clone())
                }
            }
        }

        // implicit no-default-features
        features_with_deps.insert("", (vec![], deps_required));

        // implicit default feature
        if !features_with_deps.contains_key("default") {
            features_with_deps.insert("default", (vec![""], vec![]));
        }

        features_with_deps
    }

    pub fn get_summary_description(&self) -> (Option<String>, Option<String>) {
        let (summary, description) = if let Some(ref description) = self.metadata().description {
            // Convention these days seems to be to do manual text
            // wrapping in crate descriptions, boo. \n\n is a real line break.
            let mut description = description
                .replace("\n\n", "\r")
                .replace("\n", " ")
                .replace("\r", "\n")
                .trim()
                .to_string();
            // Trim off common prefixes
            let re = Regex::new(&format!(
                r"^(?i)({}|This(\s+\w+)?)(\s*,|\s+is|\s+provides)\s+",
                self.package_id().name()
            ))
            .unwrap();
            description = re.replace(&description, "").to_string();
            let re = Regex::new(r"^(?i)(a|an|the)\s+").unwrap();
            description = re.replace(&description, "").to_string();
            let re =
                Regex::new(r"^(?i)(rust\s+)?(implementation|library|tool|crate)\s+(of|to|for)\s+")
                    .unwrap();
            description = re.replace(&description, "").to_string();

            // https://stackoverflow.com/questions/38406793/why-is-capitalizing-the-first-letter-of-a-string-so-convoluted-in-rust
            description = {
                let mut d = description.chars();
                match d.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().chain(d).collect::<String>(),
                }
            };

            // Use the first sentence or first line, whichever comes first, as the summary.
            let p1 = description.find('\n');
            let p2 = description.find(". ");
            match p1.into_iter().chain(p2.into_iter()).min() {
                Some(p) => {
                    let s = description[..p].trim_end_matches('.').to_string();
                    let d = description[p + 1..].trim();
                    if d.is_empty() {
                        (Some(s), None)
                    } else {
                        (Some(s), Some(d.to_string()))
                    }
                }
                None => (Some(description.trim_end_matches('.').to_string()), None),
            }
        } else {
            (None, None)
        };

        (summary, description)
    }

    /// To be called before extract_crate.
    pub fn set_includes_excludes(
        &mut self,
        excludes: Option<&Vec<String>>,
        includes: Option<&Vec<String>>,
    ) {
        self.excludes = excludes
            .into_iter()
            .flatten()
            .map(|x| Pattern::new(&("*/".to_owned() + x)).unwrap())
            .collect::<Vec<_>>();
        self.includes = includes
            .into_iter()
            .flatten()
            .map(|x| Pattern::new(&("*/".to_owned() + x)).unwrap())
            .collect::<Vec<_>>();
    }

    pub fn filter_path(&self, path: &Path) -> ::std::result::Result<bool, String> {
        if self.excludes.iter().any(|p| p.matches_path(path)) {
            return Ok(true);
        }
        let suspicious = match path.extension() {
            Some(ext) => ext == "c" || ext == "a",
            _ => false,
        };
        if suspicious {
            if self.includes.iter().any(|p| p.matches_path(path)) {
                debcargo_info!("Suspicious file, on whitelist so ignored: {:?}", path);
                Ok(false)
            } else if testing_ignore_debpolv() {
                debcargo_warn!("Suspicious file, ignoring as per override: {:?}", path);
                Ok(false)
            } else {
                Err(format!(
                    "Suspicious file, should probably be excluded: {:?}",
                    path
                ))
            }
        } else {
            Ok(false)
        }
    }

    pub fn extract_crate(&self, path: &Path) -> Result<bool> {
        let mut archive = Archive::new(GzDecoder::new(self.crate_file.file()));
        let tempdir = tempfile::Builder::new()
            .prefix("debcargo")
            .tempdir_in(".")?;
        let mut source_modified = false;
        let mut last_mtime = 0;
        let mut err = vec![];

        for entry in archive.entries()? {
            let mut entry = entry?;
            match self.filter_path(&(entry.path()?)) {
                Err(e) => err.push(e),
                Ok(r) => {
                    if r {
                        source_modified = true;
                        continue;
                    }
                }
            }

            if !entry.unpack_in(tempdir.path())? {
                debcargo_bail!("Crate contained path traversals via '..'");
            }

            if let Ok(mtime) = entry.header().mtime() {
                if mtime > last_mtime {
                    last_mtime = mtime;
                }
            }
        }
        if !err.is_empty() {
            for e in err {
                debcargo_warn!("{}", e);
            }
            debcargo_bail!(
                "Suspicious files detected, aborting. Ask on #debian-rust if you are stuck."
            )
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
            return Err(Error::from(e).context(format!(
                concat!(
                    "Could not create source directory {0}\n",
                    "To regenerate, move or remove {0}"
                ),
                path.display()
            )));
        }

        // Ensure that Cargo.toml is in standard form, e.g. does not contain
        // path dependencies, so can be built standalone (see #4030).
        let toml_path = path.join("Cargo.toml");
        let ws = Workspace::new(&toml_path.canonicalize()?, &self.config)?;
        let registry_toml = self.package.to_registry_toml(&ws)?;
        let mut actual_toml = String::new();
        fs::File::open(&toml_path)?.read_to_string(&mut actual_toml)?;

        if !actual_toml.contains("AUTOMATICALLY GENERATED BY CARGO") {
            // This logic should only fire for old crates, and that's what the
            // if-conditional is supposed to check; modern versions of cargo
            // already do this before uploading the crate and we shouldn't need
            // to handle it specially.
            let old_toml_path = path.join("Cargo.toml.orig");
            fs::copy(&toml_path, &old_toml_path)?;
            fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&toml_path)?
                .write_all(registry_toml.as_bytes())?;
            debcargo_info!(
                "Rewrote {:?} to canonical form\nOld backed up as {:?}",
                &toml_path,
                &old_toml_path,
            );
            source_modified = true;
            // avoid lintian errors about package-contains-ancient-file
            // TODO: do we want to do this for unmodified tarballs? it would
            // force us to modify them, but otherwise we get that ugly warning
            let last_mtime = FileTime::from_unix_time(last_mtime as i64, 0);
            set_file_times(toml_path, last_mtime, last_mtime)?;
        }
        Ok(source_modified)
    }
}

/// Calculate all feature-dependencies and external-dependencies of a given
/// feature, using the information previously generated by
/// `all_dependencies_and_features`.
pub fn transitive_deps<'a>(
    features_with_deps: &'a CrateDepInfo,
    feature: &str,
) -> (Vec<&'a str>, Vec<Dependency>) {
    let mut all_features = Vec::new();
    let mut all_deps = Vec::new();
    let &(ref ff, ref dd) = features_with_deps.get(feature).unwrap();
    all_features.extend(ff.clone());
    all_deps.extend(dd.clone());
    for f in ff {
        let (ff1, dd1) = transitive_deps(features_with_deps, f);
        all_features.extend(ff1);
        all_deps.extend(dd1);
    }
    (all_features, all_deps)
}
