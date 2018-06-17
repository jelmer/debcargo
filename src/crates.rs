use cargo::{Config,
            core::{Dependency, Package, PackageId, Registry, Source, SourceId, Summary,
                   TargetKind},
            sources::RegistrySource};
use cargo::util::FileLock;
use cargo::core::manifest;
use failure::Error;
use filetime::{set_file_times, FileTime};
use glob::Pattern;
use semver::Version;
use flate2::read::GzDecoder;
use tar::Archive;
use tempdir::TempDir;
use regex::Regex;

use std;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::io::{self, Read, Write};
use std::fs;

use errors::*;

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

fn traverse_depth<'a>(map: &BTreeMap<&'a str, Vec<&'a str>>, key: &'a str) -> Vec<&'a str> {
    let mut x = Vec::new();
    if let Some (pp) = (*map).get(key) {
        x.extend(pp);
        for p in pp {
            x.extend(traverse_depth(map, p));
        }
    }
    x
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
                self.create_dependency(&s.name(), Some(format!("{}", s.version()).as_str()))
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

    pub fn all_dependencies_and_features(&self) ->
        BTreeMap<&str,                  // name of feature / optional dependency,
                                        // or "" for the base package w/ no default features, guaranteed to be in the map
                (Vec<&str>,             // dependencies: other features (of the current package)
                 Vec<Dependency>)>      // dependencies: other packages
    {
        use cargo::core::dependency::Kind;

        let deps_by_name : BTreeMap<&str, &Dependency> = self.dependencies().iter().filter_map(|dep| {
            // we treat build-dependencies also as dependencies in Debian
            if dep.kind() == Kind::Development { None } else { Some((dep.name().to_inner(), dep)) }
        }).collect();

        // calculate dependencies of features from other crates
        let mut features_with_deps = BTreeMap::new();
        let features = self.summary.features();
        for (feature, deps) in features {
            let mut feature_deps = vec![""];
            // always need "", because in dh-cargo we symlink /usr/share/doc/{$feature => $main} pkg
            let mut other_deps = Vec::new();
            for dep_str in deps {
                let mut dep_tokens = dep_str.splitn(2, '/');
                let dep_name = dep_tokens.next().unwrap();
                match dep_tokens.next() {
                    None => {
                        // another feature is a dependency
                        feature_deps.push(dep_name);
                    }
                    Some(dep_feature) => {
                        // another package is a dependency
                        let &dep = deps_by_name.get(dep_name).unwrap(); // valid Cargo.toml files must have this
                        let mut dep = dep.clone();
                        dep.set_features(vec![dep_feature.to_string()]);
                        dep.set_default_features(false);
                        other_deps.push(dep);
                    }
                }
            }
            features_with_deps.insert(feature.as_str(), (feature_deps, other_deps));
        }

        // calculate dependencies of "optional dependencies" that are also features
        let deps_required : Vec<Dependency> = deps_by_name.iter().filter_map(|(_, &dep)| {
            if dep.is_optional() {
                features_with_deps.insert(&dep.name().to_inner(), (vec![""], vec![dep.clone()]));
                None
            } else {
                Some(dep.clone())
            }
        }).collect();

        // implicit no-default-features
        features_with_deps.insert("", (vec![], deps_required));

        // implicit default feature
        if !features_with_deps.contains_key("default") {
            features_with_deps.insert("default", (vec![""], vec![]));
        }
        features_with_deps
    }

    pub fn feature_all_deps(&self,
            features_with_deps: &BTreeMap<&str, (Vec<&str>, Vec<Dependency>)>,
            feature: &str)
            -> Vec<Dependency> {
        let mut all_deps = Vec::new();
        let &(ref ff, ref dd) = features_with_deps.get(feature).unwrap();
        all_deps.extend(dd.clone());
        for f in ff {
            all_deps.extend(self.feature_all_deps(&features_with_deps, f));
        };
        all_deps
    }

    // Note: this mutates features_with_deps so you need to run e.g.
    // feature_all_deps before calling this.
    pub fn calculate_provides<'a>(&self,
            features_with_deps: &mut BTreeMap<&'a str, (Vec<&'a str>, Vec<Dependency>)>)
            -> BTreeMap<&'a str, Vec<&'a str>> {
        let mut provides = BTreeMap::new();
        let mut provided = Vec::new();
        // the below is very simple and incomplete. e.g. it does not,
        // but could be improved to, simplify things like:
        // f1 depends on f2, f3
        // f2 depends on f4
        // f3 depends on f4
        for (&f, &(ref ff, ref dd)) in features_with_deps.iter() {
            if !dd.is_empty() {
                continue;
            }
            assert!(!ff.is_empty() || f == "");
            let k = if ff.len() == 1 {
                *ff.get(0).unwrap() // it's just ""
            } else if ff.len() == 2 {
                *ff.get(1).unwrap()
            } else {
                continue;
            };
            if !provides.contains_key(k) {
                provides.insert(k, vec![]);
            }
            provides.get_mut(k).unwrap().push(f);
            provided.push(f);
        }

        for p in provided {
            features_with_deps.remove(p);
        }

        features_with_deps.keys().map(|k| {
            let mut pp = traverse_depth(&provides, k);
            pp.sort();
            (*k, pp)
        }).collect::<BTreeMap<_, _>>()
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

    pub fn get_summary_description(&self) -> (Option<String>, Option<String>) {
        let (summary, description) = if let Some(ref description) = self.metadata().description {
            // Convention these days seems to be to do manual text
            // wrapping in crate descriptions, boo. \n\n is a real line break.
            let mut description = description
                .replace("\n\n", "\r")
                .replace("\n", " ")
                .replace("\r", "\n")
                .trim().to_string();
            // Trim off common prefixes
            let re = Regex::new(&format!(r"^(?i)({}|This(\s+\w+)?)(\s*,|\s+is|\s+provides)\s+",
                self.package_id().name())).unwrap();
            description = re.replace(&description, "").to_string();
            let re = Regex::new(r"^(?i)(a|an|the)\s+").unwrap();
            description = re.replace(&description, "").to_string();
            let re = Regex::new(r"^(?i)(rust\s+)?(implementation|library|tool|crate)\s+(of|to|for)\s+").unwrap();
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

    pub fn filter_path(excludes: &Vec<Pattern>, path: &Path) -> bool {
        // Filter out static libraries, to avoid needing to patch all the winapi crates to remove
        // import libraries.
        if excludes.iter().any(|p| p.matches_path(path)) {
            return true
        }
        match path.extension() {
            Some(ext) => if ext == "a" {
                true
            } else {
                if ext == "c" {
                    debcargo_warn!("Suspicious file, maybe should be excluded: {:?}", path);
                }
                false
            },
            _ => false,
        }
    }

    pub fn extract_crate(&self, path: &Path, excludes: &Vec<Pattern>) -> Result<bool> {
        let mut archive = Archive::new(GzDecoder::new(self.crate_file.file()));
        let tempdir = TempDir::new_in(".", "debcargo")?;
        let mut source_modified = false;
        let mut last_mtime = 0;

        for entry in archive.entries()? {
            let mut entry = entry?;
            if Self::filter_path(excludes, &(entry.path()?)) {
                source_modified = true;
                continue;
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
        let registry_toml = self.package().to_registry_toml(&Config::default()?)?;
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
            // avoid lintian errors about package-contains-ancient-file
            // TODO: do we want to do this for unmodified tarballs? it would
            // force us to modify them, but otherwise we get that ugly warning
            let last_mtime = FileTime::from_unix_time(last_mtime as i64, 0);
            set_file_times(toml_path, last_mtime, last_mtime)?;
        }
        Ok(source_modified)
    }
}
