use anyhow::{format_err, Error};
use cargo::{
    core::manifest::ManifestMetadata,
    core::registry::PackageRegistry,
    core::InternedString,
    core::{
        Dependency, EitherManifest, FeatureValue, Manifest, Package, PackageId, Registry, Source,
        SourceId, Summary, Target, TargetKind, Workspace,
    },
    core::source::MaybePackage,
    ops,
    ops::PackageOpts,
    sources::registry::RegistrySource,
    util::{toml::read_manifest, FileLock},
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
use std::path::{Path, PathBuf};

use crate::errors::*;
use crate::util::vec_opt_iter;

pub struct CrateInfo {
    package: Package,
    manifest: Manifest,
    crate_file: FileLock,
    config: Config,
    source_id: SourceId,
    excludes: Vec<Pattern>,
    includes: Vec<Pattern>,
}

fn hash<H: Hash>(hashable: &H) -> u64 {
    #![allow(deprecated)]
    let mut hasher = std::hash::SipHasher::new();
    hashable.hash(&mut hasher);
    hasher.finish()
}

fn traverse_depth<'a>(map: &BTreeMap<&'a str, Vec<&'a str>>, key: &'a str) -> Vec<&'a str> {
    let mut x = Vec::new();
    if let Some(pp) = (*map).get(key) {
        x.extend(pp);
        for p in pp {
            x.extend(traverse_depth(map, p));
        }
    }
    x
}

fn fetch_candidates(registry: &mut PackageRegistry, dep: &Dependency) -> Result<Vec<Summary>> {
    let mut summaries = registry.query_vec(dep, false)?;
    summaries.sort_by(|a, b| b.package_id().partial_cmp(&a.package_id()).unwrap());
    Ok(summaries)
}

pub fn update_crates_io() -> Result<()> {
    let config = Config::default()?;
    let _lock = config.acquire_package_cache_lock()?;
    let source_id = SourceId::crates_io(&config)?;
    let yanked_whitelist = HashSet::new();
    let mut r = RegistrySource::remote(source_id, &yanked_whitelist, &config);
    r.update()
}

impl CrateInfo {
    pub fn new(crate_name: &str, version: Option<&str>) -> Result<CrateInfo> {
        CrateInfo::new_with_update(crate_name, version, true)
    }

    pub fn new_with_local_crate(crate_name: &str, version: Option<&str>, crate_path: &Path) -> Result<CrateInfo> {
        let config = Config::default()?;
        let crate_path = crate_path.canonicalize()?;
        let source_id = SourceId::for_path(&crate_path)?;


        let (package, crate_file) = {
            let yanked_whitelist = HashSet::new();

            let mut source = source_id.load(&config, &yanked_whitelist)?;
            source.update()?;

            let package_id = match version {
                None | Some("") => {
                    let dep = Dependency::parse_no_deprecated(crate_name, None, source_id)?;
                    let mut package_id: Option<PackageId> = None;
                    source.query(&dep, &mut |p| package_id = Some(p.package_id()))?;
                    package_id.unwrap()
                },
                Some(version) => PackageId::new(crate_name, version, source_id)?,
            };

            let maybe_package = source.download(package_id)?;
            let package = match maybe_package {
                MaybePackage::Ready(p) => Ok(p),
                _ => Err(format_err!("Failed to 'download' local crate {} from {}",
                        crate_name,
                        crate_path.display()
                    )),
            }?;

            let crate_file = {
                let workspace = Workspace::ephemeral(
                    package.clone(),
                    &config,
                    None,
                    true)?;

                let opts = PackageOpts {
                    config: &config,
                    verify: false,
                    list: false,
                    check_metadata: true,
                    allow_dirty: true,
                    all_features: true,
                    no_default_features: false,
                    jobs: None,
                    target: None,
                    features: Vec::new(),
                };

                // as of cargo 0.41 this returns a FileLock with a temp path, instead of the one
                // it got renamed to
                if ops::package(&workspace, &opts)?.is_none() {
                    return Err(format_err!("Failed to assemble crate file for local crate {} at {}\n",
                        crate_name,
                        crate_path.display()
                    ));
                }
                let filename = format!("{}-{}.crate", crate_name, package_id.version().to_string());
                workspace.target_dir().join("package").open_rw(&filename, &config, "crate file")?
            };

            (package.clone(), crate_file)
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
        let config = Config::default()?;
        let source_id = {
            let source_id = SourceId::crates_io(&config)?;
            if update {
                source_id
            } else {
                // The below is a bit of a hack and depends on some cargo internals
                // but unless we do this, fetch_candidates() will update the index
                // The behaviour is brittle; we really should write a test for it.
                source_id.with_precise(Some("locked".to_string()))
            }
        };

        let version = version.map(|v| {
            if v.starts_with(|c: char| c.is_digit(10)) {
                ["=", v].concat()
            } else {
                v.to_string()
            }
        });

        let dependency = Dependency::parse_no_deprecated(
            crate_name,
            version.as_ref().map(String::as_str),
            source_id,
        )?;

        let registry_name = format!(
            "{}-{:016x}",
            source_id.url().host_str().unwrap_or(""),
            hash(&source_id).swap_bytes()
        );

        let (package, manifest, crate_file) = {
            let lock = config.acquire_package_cache_lock()?;
            let mut registry = PackageRegistry::new(&config)?;
            registry.lock_patches();
            let summaries = fetch_candidates(&mut registry, &dependency)?;
            drop(lock);
            let pkgids = summaries
                .into_iter()
                .map(|s| s.package_id())
                .collect::<Vec<_>>();
            let pkgid = pkgids.iter().max().ok_or_else(|| {
                format_err!(
                    concat!(
                        "Couldn't find any crate matching {} {}\n ",
                        "Try `debcargo update` to update the crates.io index."
                    ),
                    dependency.package_name(),
                    dependency.version_req()
                )
            })?;
            let pkgset = registry.get(pkgids.as_slice())?;
            let package = pkgset.get_one(*pkgid)?;
            let manifest = package.manifest();
            let filename = format!("{}-{}.crate", pkgid.name(), pkgid.version());
            let crate_file = config
                .registry_cache_path()
                .join(&registry_name)
                .open_ro(&filename, &config, &filename)?;
            (package.clone(), manifest.clone(), crate_file)
        };

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

    pub fn targets(&self) -> &[Target] {
        self.manifest.targets()
    }

    pub fn version(&self) -> &Version {
        self.manifest.summary().package_id().version()
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn replace_manifest(&mut self, path: &PathBuf) -> Result<&Self> {
        if let (EitherManifest::Real(v), _) = read_manifest(path, self.source_id, &self.config)? {
            self.manifest = v;
        }
        Ok(self)
    }

    pub fn checksum(&self) -> Option<&str> {
        self.manifest.summary().checksum()
    }

    pub fn package_id(&self) -> PackageId {
        self.manifest.summary().package_id()
    }

    pub fn metadata(&self) -> &ManifestMetadata {
        self.manifest.metadata()
    }

    pub fn summary(&self) -> &Summary {
        self.manifest.summary()
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

    pub fn all_dependencies_and_features(
        &self,
    ) -> BTreeMap<
        &str, // name of feature / optional dependency,
        // or "" for the base package w/ no default features, guaranteed to be in the map
        (
            Vec<&str>, // dependencies: other features (of the current package)
            Vec<Dependency>,
        ),
    > // dependencies: other packages
    {
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
            let mut feature_deps = vec![""];
            // always need "", because in dh-cargo we symlink /usr/share/doc/{$feature => $main} pkg
            let mut other_deps: Vec<Dependency> = Vec::new();
            for dep in deps {
                use self::FeatureValue::*;
                match dep {
                    // another feature is a dependency
                    Feature(dep_feature) => feature_deps.push(dep_feature),
                    // another package is a dependency
                    Crate(dep_name) => {
                        // unwrap is ok, valid Cargo.toml files must have this
                        for &dep in deps_by_name.get(dep_name.as_str()).unwrap() {
                            other_deps.push(dep.clone());
                        }
                    }
                    // another package is a dependency
                    CrateFeature(dep_name, dep_feature) => {
                        // unwrap is ok, valid Cargo.toml files must have this
                        for &dep in deps_by_name.get(dep_name.as_str()).unwrap() {
                            let mut dep = dep.clone();
                            dep.set_features(vec![InternedString::new(dep_feature)]);
                            dep.set_default_features(false);
                            other_deps.push(dep);
                        }
                    }
                }
            }
            features_with_deps.insert(feature.as_str(), (feature_deps, other_deps));
        }

        // calculate dependencies of this crate's "optional dependencies", since they are also features
        let mut deps_required: Vec<Dependency> = Vec::new();
        for deps in deps_by_name.values() {
            for &dep in deps {
                if dep.is_optional() {
                    features_with_deps
                        .insert(&dep.name_in_toml().as_str(), (vec![""], vec![dep.clone()]));
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

    pub fn feature_all_deps<'a>(
        &self,
        features_with_deps: &'a BTreeMap<&str, (Vec<&str>, Vec<Dependency>)>,
        feature: &str,
    ) -> (Vec<&'a str>, Vec<Dependency>) {
        let mut all_features = Vec::new();
        let mut all_deps = Vec::new();
        let &(ref ff, ref dd) = features_with_deps.get(feature).unwrap();
        all_features.extend(ff.clone());
        all_deps.extend(dd.clone());
        for f in ff {
            let (ff1, dd1) = self.feature_all_deps(&features_with_deps, f);
            all_features.extend(ff1);
            all_deps.extend(dd1);
        }
        (all_features, all_deps)
    }

    // Calculate Provides: in an attempt to reduce the number of binaries.
    //
    // Note: this mutates features_with_deps so you MUST run e.g.
    // feature_all_deps *before* calling this.
    //
    // The algorithm is very simple and incomplete. e.g. it does not, yet
    // simplify things like:
    //   f1 depends on f2, f3
    //   f2 depends on f4
    //   f3 depends on f4
    // into
    //   f4 provides f1, f2, f3
    pub fn calculate_provides<'a>(
        &self,
        features_with_deps: &mut BTreeMap<&'a str, (Vec<&'a str>, Vec<Dependency>)>,
    ) -> BTreeMap<&'a str, Vec<&'a str>> {
        // If any features have duplicate dependencies, deduplicate them by
        // making all of the subsequent ones depend on the first one.
        let mut features_rev_deps = BTreeMap::new();
        for (&f, dep) in features_with_deps.iter() {
            if !features_rev_deps.contains_key(dep) {
                features_rev_deps.insert(dep.clone(), vec![]);
            }
            features_rev_deps.get_mut(dep).unwrap().push(f);
        }
        for (_, ff) in features_rev_deps.into_iter() {
            let f0 = ff[0];
            for f in &ff[1..] {
                features_with_deps.insert(f, (vec!["", f0], vec![]));
            }
        }

        // Calculate provides by following 0- or 1-length dependency lists.
        let mut provides = BTreeMap::new();
        let mut provided = Vec::new();
        for (&f, (ref ff, ref dd)) in features_with_deps.iter() {
            //debcargo_info!("provides considering: {:?}", &f);
            if !dd.is_empty() {
                continue;
            }
            assert!(!ff.is_empty() || f == "");
            let k = if ff.len() == 1 {
                // if A depends only on no-default-features (""), then
                // no-default-features provides A.
                assert!(ff[0] == "");
                ff[0]
            } else if ff.len() == 2 {
                // if A depends on a single feature B, then B provides A.
                assert!(ff[0] == "");
                ff[1]
            } else {
                continue;
            };
            //debcargo_info!("provides still considering: {:?}", &f);
            if !provides.contains_key(k) {
                provides.insert(k, vec![]);
            }
            provides.get_mut(k).unwrap().push(f);
            provided.push(f);
        }

        //debcargo_info!("provides-internal: {:?}", &provides);
        //debcargo_info!("provided-internal: {:?}", &provided);
        for p in provided {
            features_with_deps.remove(p);
        }

        features_with_deps
            .keys()
            .map(|k| {
                let mut pp = traverse_depth(&provides, k);
                pp.sort();
                (*k, pp)
            })
            .collect::<BTreeMap<_, _>>()
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

    pub fn semver_suffix(&self) -> String {
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

    pub fn semver_uscan_pattern(&self) -> String {
        // See `man uscan` description of @ANY_VERSION@ on how these
        // regex patterns were built.
        match *self.package_id().version() {
            Version {
                major: 0, minor, ..
            } => format!("[-_]?(0\\.{}\\.\\d[\\-+\\.:\\~\\da-zA-Z]*)", minor),
            Version { major, .. } => format!("[-_]?({}\\.\\d[\\-+\\.:\\~\\da-zA-Z]*)", major),
        }
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

    pub fn set_includes_excludes(
        &mut self,
        excludes: Option<&Vec<String>>,
        includes: Option<&Vec<String>>,
    ) {
        self.excludes = vec_opt_iter(excludes)
            .map(|x| Pattern::new(&("*/".to_owned() + x)).unwrap())
            .collect::<Vec<_>>();
        self.includes = vec_opt_iter(includes)
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
            return Err(Error::from(Error::from(e).context(format!(
                concat!(
                    "Could not create source directory {0}\n",
                    "To regenerate, move or remove {0}"
                ),
                path.display()
            ))));
        }

        // Ensure that Cargo.toml is in standard form, e.g. does not contain
        // path dependencies, so can be built standalone (see #4030).
        let registry_toml = self.package().to_registry_toml(&self.config)?;
        let mut actual_toml = String::new();
        let toml_path = path.join("Cargo.toml");
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
