use chrono;
use tempdir;
use cargo::core::Dependency;
use cargo::util::FileLock;
use semver::Version;
use itertools::Itertools;
use semver_parser;
use semver_parser::range::*;
use semver_parser::range::Op::*;
use tempdir::TempDir;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use tar::{Archive, Builder};

use std;
use std::io::{self, Write as IoWrite};
use std::fs;
use std::fmt::{self, Write as FmtWrite};
use std::path::{Path, PathBuf};
use std::collections::HashSet;
use std::os::unix::fs::OpenOptionsExt;

use errors::*;
use crates::CrateInfo;
use copyright::debian_copyright;
use overrides::{parse_overrides, OverrideDefaults, Overrides};

const RUST_MAINT: &'static str = "Rust Maintainers <pkg-rust-maintainers@lists.alioth.debian.org>";
const VCS_GIT: &'static str = "https://anonscm.debian.org/git/pkg-rust/";
const VCS_VIEW: &'static str = "https://anonscm.debian.org/cgit/pkg-rust/";

pub struct Source {
    name: String,
    section: String,
    priority: String,
    maintainer: String,
    uploaders: String,
    standards: String,
    build_deps: String,
    vcs_git: String,
    vcs_browser: String,
    homepage: String,
    x_cargo: String,
    version: String,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Source: {}", self.name)?;
        if !self.section.is_empty() {
            writeln!(f, "Section: {}", self.section)?;
        }

        writeln!(f, "Priority: {}", self.priority)?;
        writeln!(f, "Build-Depends: {}", self.build_deps)?;
        writeln!(f, "Maintainer: {}", self.maintainer)?;
        writeln!(f, "Uploaders: {}", self.uploaders)?;
        writeln!(f, "Standards-Version: {}", self.standards)?;
        writeln!(f, "Vcs-Git: {}", self.vcs_git)?;
        writeln!(f, "Vcs-Browser: {}", self.vcs_browser)?;

        if !self.homepage.is_empty() {
            writeln!(f, "Homepage: {}", self.homepage)?;
        }

        if !self.x_cargo.is_empty() {
            writeln!(f, "X-Cargo-Crate: {}", self.x_cargo)?;
        }

        write!(f, "\n")
    }
}

impl Source {
    pub fn new(pkgbase: &PkgBase, home: &str, lib: &bool, bdeps: &[String]) -> Result<Source> {
        let source = format!("rust-{}", pkgbase.crate_pkg_base);
        let section = if *lib { "rust" } else { "FIXME" };
        let priority = "optional".to_string();
        let maintainer = RUST_MAINT.to_string();
        let uploaders = get_deb_author()?;
        let vcs_git = format!("{}{}.git", VCS_GIT, source);
        let vcs_browser = format!("{}{}.git", VCS_VIEW, source);
        let mut build_deps = vec!["debhelper (>= 10)".to_string(), "dh-cargo".to_string()];
        build_deps.extend_from_slice(bdeps);
        let build_deps = build_deps.iter()
            // .chain(bdeps)
            .join(",\n ");
        let cargo_crate = if pkgbase.crate_name != pkgbase.crate_name.replace('_', "-") {
            pkgbase.crate_name.clone()
        } else {
            "".to_string()
        };
        Ok(Source {
               name: source,
               section: section.to_string(),
               priority: priority,
               maintainer: maintainer,
               uploaders: uploaders,
               standards: "4.0.0".to_string(),
               build_deps: build_deps,
               vcs_git: vcs_git,
               vcs_browser: vcs_browser,
               homepage: home.to_string(),
               x_cargo: cargo_crate,
               version: format!("{}-1", pkgbase.debver),
           })
    }

    pub fn srcname(&self) -> &String {
        &self.name
    }

    pub fn changelog_entry(&self,
                           crate_name: &str,
                           crate_version: &Version,
                           distribution: &str,
                           selfversion: &str)
                           -> String {
        format!(concat!("{} ({}) {}; urgency=medium\n\n",
                        "  * Package {} {} from crates.io with debcargo {}\n\n",
                        " -- {}  {}\n"),
                self.name,
                self.version,
                distribution,
                crate_name,
                crate_version,
                selfversion,
                self.uploaders,
                chrono::Local::now().to_rfc2822())
    }
}

impl OverrideDefaults for Source {
    fn apply_overrides(&mut self, overrides: &Overrides) {
        if let Some(section) = overrides.section() {
            self.section = section.to_string();
        }

        if let Some(policy) = overrides.policy_version() {
            self.standards = policy.to_string();
        }

        if let Some(bdeps) = overrides.build_depends() {
            let deps = bdeps.iter().join(",\n ");
            self.build_deps.push_str(",\n ");
            self.build_deps.push_str(&deps);
        }

        if let Some(homepage) = overrides.homepage() {
            self.homepage = homepage.to_string();
        }
    }
}

pub struct Package {
    name: String,
    arch: String,
    section: String,
    depends: String,
    suggests: String,
    provides: String,
    summary: String,
    description: String,
    boilerplate: String,
}


impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Package: {}", self.name)?;
        writeln!(f, "Architecture: {}", self.arch)?;

        if !self.section.is_empty() {
            writeln!(f, "Section: {}", self.section)?;
        }

        writeln!(f, "Depends:\n {}", self.depends)?;
        if !self.suggests.is_empty() {
            writeln!(f, "Suggests:\n {}", self.suggests)?;
        }

        if !self.provides.is_empty() {
            writeln!(f, "Provides:\n {}", self.provides)?;
        }

        write_description(f,
                          self.summary.as_str(),
                          if self.description.is_empty() {
                              None
                          } else {
                              Some(&self.description)
                          },
                          Some(&self.boilerplate))
    }
}

impl Package {
    pub fn new(pkgbase: &PkgBase,
               deps: &[String],
               non_default_features: Option<&Vec<&str>>,
               default_features: Option<&HashSet<&str>>,
               summary: &Option<String>,
               description: &Option<String>,
               feature: Option<&str>)
               -> Package {
        let deb_feature = &|f: &str| deb_feature_name(&pkgbase.crate_pkg_base, f);
        let suggests = match non_default_features {
            Some(ndf) => ndf.iter().cloned().map(deb_feature).join(",\n "),
            None => "".to_string(),
        };

        let provides = match default_features {
            Some(df) => {
                df.into_iter()
                    .map(|f| format!("{} (= ${{binary:Version}})", deb_feature(f)))
                    .join(",\n ")
            }
            None => "".to_string(),

        };

        let depends = vec!["${misc:Depends}".to_string()]
            .iter()
            .chain(deps.iter())
            .join(",\n ");

        let short_desc = match *summary {
            None => format!("Source of Rust {} crate", pkgbase.crate_pkg_base),
            Some(ref s) => {
                format!("{} - {}",
                        s,
                        if let Some(f) = feature { f } else { "Source" })
            }
        };

        let name = match feature {
            None => deb_name(pkgbase.crate_pkg_base.as_str()),
            Some(s) => deb_feature(s),
        };

        let long_desc = match *description {
            None => "".to_string(),
            Some(ref s) => s.to_string(),
        };

        let boilerplate = match feature {
            None => {
                format!(concat!("This package contains the source for the",
                                " Rust {} crate,\npackaged for use with",
                                " cargo, debcargo, and dh-cargo."),
                        pkgbase.crate_name)
            }
            Some(f) => {
                format!(concat!("This package enables feature {} for the",
                                " Rust {} crate. Purpose of this package",
                                " is\nto pull the additional dependency",
                                " needed to enable feature {}."),
                        f,
                        pkgbase.crate_name,
                        f)
            }
        };

        Package {
            name: name,
            arch: "all".to_string(),
            section: "".to_string(),
            depends: depends,
            suggests: suggests,
            provides: provides,
            summary: short_desc,
            description: long_desc,
            boilerplate: boilerplate,
        }
    }

    pub fn new_bin(pkgbase: &PkgBase,
                   name: &str,
                   summary: &Option<String>,
                   description: &Option<String>,
                   boilerplate: &str)
                   -> Self {
        let short_desc = match *summary {
            None => format!("Binaries built from the Rust {} crate", pkgbase.crate_name),
            Some(ref s) => s.to_string(),
        };

        let long_desc = match *description {
            None => "".to_string(),
            Some(ref s) => s.to_string(),
        };

        Package {
            name: name.to_string(),
            arch: "any".to_string(),
            section: "misc".to_string(),
            depends: vec!["${misc:Depends}".to_string(),
                          "${shlibs:Depends}".to_string()]
                .iter()
                .join(",\n "),
            suggests: "".to_string(),
            provides: "".to_string(),
            summary: short_desc,
            description: long_desc,
            boilerplate: boilerplate.to_string(),
        }
    }

    pub fn name(&self) -> &String {
        &self.name
    }
}

impl OverrideDefaults for Package {
    fn apply_overrides(&mut self, overrides: &Overrides) {
        if let Some((s, d)) = overrides.summary_description_for(&self.name) {
            if !s.is_empty() {
                self.summary = s.to_string();
            }

            if !d.is_empty() {
                self.description = d.to_string();
            }
        }
    }
}

pub struct PkgBase {
    pub crate_name: String,
    pub crate_pkg_base: String,
    pub debver: String,
    pub srcdir: PathBuf,
    pub orig_tar_gz: PathBuf,
    pub debcargo_version: String,
}

impl PkgBase {
    pub fn new(crate_name: &str,
               version_suffix: &str,
               version: &Version,
               debcargo_version: &str)
               -> Result<PkgBase> {
        let crate_name_dashed = crate_name.replace('_', "-");
        let crate_pkg_base = format!("{}{}", crate_name_dashed, version_suffix);

        let debsrcname = format!("rust-{}", crate_pkg_base);
        let debver = deb_version(version);
        let srcdir = Path::new(&format!("{}-{}", debsrcname, debver)).to_owned();
        let orig_tar_gz = Path::new(&format!("{}_{}.orig.tar.gz", debsrcname, debver)).to_owned();

        Ok(PkgBase {
               crate_name: crate_name.to_string(),
               crate_pkg_base: crate_pkg_base,
               debver: debver,
               srcdir: srcdir.to_path_buf(),
               orig_tar_gz: orig_tar_gz.to_path_buf(),
               debcargo_version: debcargo_version.to_string(),
           })
    }
}

#[derive(PartialEq)]
enum V {
    M(u64),
    MM(u64, u64),
    MMP(u64, u64, u64),
}

impl V {
    fn new(p: &Predicate, dep: &str) -> Result<Self> {
        use self::V::*;
        let mmp = match (p.minor, p.patch) {
            (None, None) => M(p.major),
            (Some(minor), None) => MM(p.major, minor),
            (Some(minor), Some(patch)) => MMP(p.major, minor, patch),
            (None, Some(_)) => panic!("semver had patch without minor"),
        };
        if mmp == M(0) && p.op != Gt {
            debcargo_bail!("Unrepresentable dependency version predicate: {} {:?}",
                           dep,
                           p);
        }

        Ok(mmp)
    }

    fn inclast(&self) -> V {
        use self::V::*;
        match *self {
            M(major) => M(major + 1),
            MM(major, minor) => MM(major, minor + 1),
            MMP(major, minor, patch) => MMP(major, minor, patch + 1),
        }
    }
}

impl fmt::Display for V {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::V::*;
        match *self {
            M(major) => write!(f, "{}", major),
            MM(major, minor) => write!(f, "{}.{}", major, minor),
            MMP(major, minor, patch) => write!(f, "{}.{}.{}", major, minor, patch),
        }
    }
}


/// Translates a semver into a Debian version. Omits the build metadata, and uses a ~ before the
/// prerelease version so it compares earlier than the subsequent release.
fn deb_version(v: &Version) -> String {
    let mut s = format!("{}.{}.{}", v.major, v.minor, v.patch);
    for (n, id) in v.pre.iter().enumerate() {
        write!(s, "{}{}", if n == 0 { '~' } else { '.' }, id).unwrap();
    }
    s
}

fn deb_name(name: &str) -> String {
    format!("librust-{}-dev", name.replace('_', "-"))
}

pub fn deb_feature_name(name: &str, feature: &str) -> String {
    format!("librust-{}+{}-dev",
            name.replace('_', "-"),
            feature.replace('_', "-"))
}

/// Retrieve one of a series of environment variables, and provide a friendly error message for
/// non-UTF-8 values.
fn get_envs(keys: &[&str]) -> Result<Option<String>> {
    use std::env::VarError;
    for key in keys {
        match std::env::var(key) {
            Ok(val) => {
                return Ok(Some(val));
            }
            Err(e @ VarError::NotUnicode(_)) => {
                return Err(e)
                    .chain_err(|| format!("Environment variable ${} not valid UTF-8", key));
            }
            Err(VarError::NotPresent) => {}
        }
    }
    Ok(None)
}

/// Determine a name and email address from environment variables.
pub fn get_deb_author() -> Result<String> {
    let name = try!(try!(get_envs(&["DEBFULLNAME", "NAME"]))
                        .ok_or("Unable to determine your name; please set $DEBFULLNAME or $NAME"));
    let email = try!(try!(get_envs(&["DEBEMAIL", "EMAIL"]))
                         .ok_or("Unable to determine your email; please set $DEBEMAIL or $EMAIL"));
    Ok(format!("{} <{}>", name, email))
}

fn write_description(out: &mut fmt::Formatter,
                     summary: &str,
                     longdesc: Option<&String>,
                     boilerplate: Option<&String>)
                     -> fmt::Result {
    writeln!(out, "Description: {}", summary)?;
    for (n, s) in longdesc.iter().chain(boilerplate.iter()).enumerate() {
        if n != 0 {
            writeln!(out, " .")?;
        }
        for line in s.trim().lines() {
            let line = line.trim();
            if line.is_empty() {
                writeln!(out, " .")?;
            } else if line.starts_with("- ") {
                writeln!(out, "  {}", line)?;
            } else {
                writeln!(out, " {}", line)?;
            }
        }
    }
    write!(out, "")
}

/// Translates a Cargo dependency into a Debian package dependency.
pub fn deb_dep(dep: &Dependency) -> Result<String> {
    use self::V::*;
    let dep_dashed = dep.name().replace('_', "-");
    let mut suffixes = Vec::new();
    if dep.uses_default_features() {
        suffixes.push("+default-dev".to_string());
    }
    for feature in dep.features() {
        suffixes.push(format!("+{}-dev", feature.replace('_', "-")));
    }
    if suffixes.is_empty() {
        suffixes.push("-dev".to_string());
    }
    let req = semver_parser::range::parse(&dep.version_req().to_string()).unwrap();
    let mut deps = Vec::new();
    for suffix in suffixes {
        let pkg = |v: &V| {
            let (major, minor) = match *v {
                M(major) => (major, 0),
                MM(major, minor) |
                MMP(major, minor, _) => (major, minor),
            };
            if major == 0 {
                format!("librust-{}-{}.{}{}", dep_dashed, major, minor, suffix)
            } else {
                format!("librust-{}-{}{}", dep_dashed, major, suffix)
            }
        };

        for p in &req.predicates {
            // Cargo/semver and Debian handle pre-release versions quite
            // differently, so a versioned Debian dependency cannot properly
            // handle pre-release crates. Don't package pre-release crates or
            // crates that depend on pre-release crates.
            if !p.pre.is_empty() {
                debcargo_bail!("Dependency on prerelease version: {} {:?}", dep.name(), p);
            }

            let mmp = V::new(p, dep.name())?;

            match (&p.op, &mmp) {
                (&Ex, &M(..)) => deps.push(pkg(&mmp)),
                (&Ex, &MM(..)) => deps.push(format!("{} (>= {})", pkg(&mmp), mmp)),
                (&Ex, &MMP(..)) => {
                    deps.push(format!("{} (>= {})", pkg(&mmp), mmp));
                    deps.push(format!("{} (<< {})", pkg(&mmp), mmp.inclast()));
                }
                // We can't represent every major version that satisfies an
                // inequality, because each major version has a different
                // package name, so we only allow the first major version that
                // satisfies the inequality. This may result in a stricter
                // dependency, but will never result in a looser one. We could
                // represent some dependency ranges (such as >= x and < y)
                // better with a disjunction on multiple package names, but that
                // would break when depending on multiple features.
                (&Gt, &M(_)) | (&Gt, &MM(0, _)) => deps.push(pkg(&mmp.inclast())),
                (&Gt, _) => deps.push(format!("{} (>> {})", pkg(&mmp), mmp)),
                (&GtEq, &M(_)) |
                (&GtEq, &MM(0, _)) => deps.push(pkg(&mmp)),
                (&GtEq, _) => deps.push(format!("{} (>= {})", pkg(&mmp), mmp)),
                (&Lt, &M(major)) => deps.push(pkg(&M(major - 1))),
                (&Lt, &MM(0, 0)) => {
                    debcargo_bail!("Unrepresentable dependency version predicate: {} {:?}",
                                   dep.name(),
                                   p)
                }
                (&Lt, &MM(0, minor)) => deps.push(pkg(&MM(0, minor - 1))),
                (&Lt, _) => deps.push(format!("{} (<< {})", pkg(&mmp), mmp)),
                (&LtEq, &M(_)) |
                (&LtEq, &MM(0, _)) => deps.push(pkg(&mmp)),
                (&LtEq, _) => deps.push(format!("{} (<< {})", pkg(&mmp), mmp.inclast())),
                (&Tilde, &M(_)) |
                (&Tilde, &MM(0, _)) |
                (&Tilde, &MMP(0, _, 0)) => deps.push(pkg(&mmp)),
                (&Tilde, &MM(..)) |
                (&Tilde, &MMP(0, _, _)) => deps.push(format!("{} (>= {})", pkg(&mmp), mmp)),
                (&Tilde, &MMP(major, minor, _)) => {
                    deps.push(format!("{} (>= {})", pkg(&mmp), mmp));
                    deps.push(format!("{} (<< {})", pkg(&mmp), MM(major, minor + 1)));
                }
                (&Compatible, &MMP(0, 0, _)) => {
                    deps.push(format!("{} (>= {})", pkg(&mmp), mmp));
                    deps.push(format!("{} (<< {})", pkg(&mmp), mmp.inclast()));
                }
                (&Compatible, &M(_)) |
                (&Compatible, &MM(0, _)) |
                (&Compatible, &MM(_, 0)) |
                (&Compatible, &MMP(0, _, 0)) => deps.push(pkg(&mmp)),
                (&Compatible, &MM(..)) |
                (&Compatible, &MMP(..)) => deps.push(format!("{} (>= {})", pkg(&mmp), mmp)),
                (&Wildcard(WildcardVersion::Major), _) => {
                    debcargo_bail!("Unrepresentable dependency wildcard: {} = \"{:?}\"",
                                   dep.name(),
                                   p)
                }
                (&Wildcard(WildcardVersion::Minor), _) => deps.push(pkg(&mmp)),
                (&Wildcard(WildcardVersion::Patch), _) => {
                    deps.push(format!("{} (>= {})", pkg(&mmp), mmp))
                }
            }
        }
    }
    Ok(deps.join(", "))
}

pub fn prepare_orig_tarball(crate_file: &FileLock,
                            tarball: &Path,
                            src_modified: bool)
                            -> Result<()> {
    let tempdir = TempDir::new_in(".", "debcargo")?;
    let temp_archive_path = tempdir.path().join(tarball);

    let mut create = fs::OpenOptions::new();
    create.write(true).create_new(true);

    // Filter out static libraries, to avoid needing to patch all the winapi crates to remove
    // import libraries.
    let remove_path = |path: &Path| match path.extension() {
        Some(ext) if ext == "a" => true,
        _ => false,
    };

    if src_modified {
        let mut f = crate_file.file();
        use std::io::Seek;
        f.seek(io::SeekFrom::Start(0))?;
        let mut archive = Archive::new(GzDecoder::new(f)?);
        let mut new_archive = Builder::new(GzEncoder::new(create.open(&tarball)?,
                                                          Compression::Best));
        for entry in archive.entries()? {
            let entry = entry?;
            if !remove_path(&entry.path()?) {
                new_archive.append(&entry.header().clone(), entry)?;
            }
        }

        new_archive.finish()?;
        writeln!(io::stderr(), "Filtered out files from .orig.tar.gz")?;
    } else {
        fs::copy(crate_file.path(), &temp_archive_path)?;
    }

    fs::rename(temp_archive_path, &tarball)?;
    Ok(())
}

pub fn prepare_debian_folder(pkgbase: &PkgBase,
                             crate_info: &CrateInfo,
                             pkg_lib_binaries: bool,
                             bin_name: &str,
                             distribution: &str,
                             overrides: Option<Overrides>)
                             -> Result<()> {
    let lib = crate_info.is_lib();
    let mut bins = crate_info.get_binary_targets();

    let meta = crate_info.metadata();

    let (default_features, _) = crate_info.default_deps_features().unwrap();
    let non_default_features = crate_info.non_default_features(&default_features).unwrap();
    let deps = crate_info.non_dev_dependencies()?;

    let build_deps = if !bins.is_empty() {
        deps.iter()
    } else {
        [].iter()
    };

    if lib && !bins.is_empty() && !pkg_lib_binaries {
        debcargo_info!("Ignoring binaries from lib crate; pass --bin to package: {}",
                       bins.join(", "));
        bins.clear();
    }

    let mut create = fs::OpenOptions::new();
    create.write(true).create_new(true);

    let tempdir = tempdir::TempDir::new_in(".", "debcargo")?;
    let deb_feature = &|f: &str| deb_feature_name(&pkgbase.crate_pkg_base, f);


    {
        let file = |name: &str| create.open(tempdir.path().join(name));

        // debian/cargo-checksum.json
        let checksum = crate_info
            .checksum()
            .unwrap_or("Could not get crate checksum");
        let mut cargo_checksum_json = file("cargo-checksum.json")?;
        writeln!(cargo_checksum_json,
                 r#"{{"package":"{}","files":{{}}}}"#,
                 checksum)?;

        // debian/compat
        let mut compat = file("compat")?;
        writeln!(compat, "10")?;

        // debian/copyright
        let mut copyright = io::BufWriter::new(file("copyright")?);
        let dep5_copyright =
            debian_copyright(crate_info.package(), &pkgbase.srcdir, crate_info.manifest())?;
        writeln!(copyright, "{}", dep5_copyright)?;

        // debian/watch
        let mut watch = file("watch")?;
        writeln!(watch,
                 "{}",
                 format!(concat!("version=4\n",
                                 "opts=filenamemangle=s/.*\\/(.*)\\/download/{}-$1\\.tar\\.\
                                  gz/g\\ \n",
                                 " https://qa.debian.org/cgi-bin/fakeupstream.\
                                  cgi?upstream=crates.io/{} ",
                                 ".*/crates/{}/@ANY_VERSION@/download\n"),
                         pkgbase.crate_name,
                         pkgbase.crate_name,
                         pkgbase.crate_name))?;

        // debian/source/format
        fs::create_dir(tempdir.path().join("source"))?;
        let mut source_format = file("source/format")?;
        writeln!(source_format, "3.0 (quilt)")?;

        // debian/rules
        let mut create_exec = create.clone();
        create_exec.mode(0o777);
        let mut rules = create_exec.open(tempdir.path().join("rules"))?;
        write!(rules,
               "{}",
               concat!("#!/usr/bin/make -f\n",
                       "%:\n",
                       "\tdh $@ --buildsystem cargo\n"))?;

        // debian/control
        let mut source = Source::new(&pkgbase,
                                     if let Some(ref home) = meta.homepage {
                                         home
                                     } else {
                                         ""
                                     },
                                     &lib,
                                     build_deps.as_slice())?;

        // If source overrides are present update related parts.
        if let Some(ref overrides) = overrides {
            source.apply_overrides(overrides);
        }

        let mut control = io::BufWriter::new(file("control")?);
        write!(control, "{}", source)?;

        // Summary and description generated from Cargo.toml
        let (summary, description) = crate_info.get_summary_description();

        if lib {
            let ndf = non_default_features.clone();
            let ndf = if ndf.is_empty() { None } else { Some(&ndf) };

            let df = default_features.clone();
            let df = if df.is_empty() { None } else { Some(&df) };

            let mut lib_package =
                Package::new(pkgbase, &deps, ndf, df, &summary, &description, None);

            // Apply overrides if any
            if let Some(ref overrides) = overrides {
                lib_package.apply_overrides(&overrides);
            }
            writeln!(control, "{}", lib_package)?;

            for feature in non_default_features {
                let mut feature_deps = vec![format!("{} (= ${{binary:Version}})",
                                                    lib_package.name())];

                crate_info
                    .get_feature_dependencies(feature, deb_feature, &mut feature_deps)?;

                let mut feature_package = Package::new(&pkgbase,
                                                       &feature_deps,
                                                       None,
                                                       None,
                                                       &summary,
                                                       &description,
                                                       Some(feature));

                // If any overrides present for this package it will be taken care.
                if let Some(ref overrides) = overrides {
                    feature_package.apply_overrides(&overrides);
                }
                writeln!(control, "{}", feature_package)?;
            }
        }

        if !bins.is_empty() {
            let boilerplate = if bins.len() > 1 || bins[0] != bin_name {
                Some(format!(
                    "This package contains the following binaries built
        from the\nRust \"{}\" crate:\n- {}",
                    pkgbase.crate_name,
                    bins.join("\n- ")
                ))
            } else {
                None
            };

            let mut bin_pkg = Package::new_bin(&pkgbase,
                                               bin_name,
                                               &summary,
                                               &description,
                                               match boilerplate {
                                                   Some(ref s) => s,
                                                   None => "",
                                               });

            // Binary package overrides.
            if let Some(ref overrides) = overrides {
                bin_pkg.apply_overrides(&overrides);
            }

            writeln!(control, "{}", bin_pkg)?;
        }

        // debian/changelog
        let mut changelog = try!(file("changelog"));
        let pkgid = crate_info.package_id();
        write!(changelog,
               "{}",
               source.changelog_entry(pkgid.name(),
                                      pkgid.version(),
                                      distribution,
                                      &pkgbase.debcargo_version))?;

    }

    fs::rename(tempdir.path(), pkgbase.srcdir.join("debian"))?;
    tempdir.into_path();
    Ok(())
}
