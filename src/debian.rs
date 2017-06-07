use cargo::core::manifest::ManifestMetadata;
use semver::Version;
use itertools::Itertools;

use std;
use std::fmt::{self, Write as FmtWrite};
use std::path::{Path, PathBuf};
use errors::*;

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
}

pub struct PkgBase {
    crate_name: String,
    crate_pkg_base: String,
    debver: String,
    srcdir: PathBuf,
    orig_tar_gz: PathBuf,
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
    pub fn new(pkgbase: &PkgBase, home: &String, lib: bool, bdeps: &Vec<String>) -> Result<Source> {
        let source = format!("rust-{}", pkgbase.crate_pkg_base);
        let section = if lib { "rust" } else { "" };
        let priority = "optional".to_string();
        let maintainer = RUST_MAINT.to_string();
        let uploaders = get_deb_author()?;
        let vcs_git = format!("{}{}.git", VCS_GIT, source);
        let vcs_browser = format!("{}{}.git", VCS_VIEW, source);
        let build_deps = vec!["debhelper (>= 10)".to_string(), "dh-cargo".to_string()]
            .iter()
            .chain(bdeps)
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
            standards: "3.9.8".to_string(),
            build_deps: build_deps,
            vcs_git: vcs_git,
            vcs_browser: vcs_browser,
            homepage: home.clone(),
            x_cargo: cargo_crate,
        })
    }
}

impl PkgBase {
    pub fn new(crate_name: &String, version_suffix: &String, version: &Version) -> Result<PkgBase> {
        let crate_name_dashed = crate_name.replace('_', "-");
        let crate_pkg_base = format!("{}-{}", crate_name_dashed, version_suffix);

        let debsrcname = format!("rust-{}", crate_pkg_base);
        let debver = deb_version(version);
        let srcdir = Path::new(&format!("{}-{}", debsrcname, debver)).to_owned();
        let orig_tar_gz = Path::new(&format!("{}_{}.orig.tar.gz", debsrcname, debver)).to_owned();

        Ok(PkgBase {
            crate_name: crate_name.clone(),
            crate_pkg_base: crate_pkg_base,
            debver: debver,
            srcdir: srcdir.to_path_buf(),
            orig_tar_gz: orig_tar_gz.to_path_buf(),
        })
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

fn deb_feature_name(name: &str, feature: &str) -> String {
    format!("librust-{}+{}-dev",
            name.replace('_', "-"),
            feature.replace('_', "-"))
}

/// Retrieve one of a series of environment variables, and provide a friendly error message for
/// non-UTF-8 values.
fn get_envs(keys: &[&str]) -> Result<Option<String>> {
    use std::env::{self, VarError};
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
