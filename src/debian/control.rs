use std::fmt::{self, Write};
use std::env::{self, VarError};

use failure::Error;
use itertools::Itertools;
use semver::Version;
use textwrap::fill;

use cargo::core::Dependency;
use config::{Config, OverrideDefaults};
use debian::dependency::deb_deps;
use errors::*;
use util::vec_opt_iter;

const RUST_MAINT: &'static str = "Rust Maintainers <pkg-rust-maintainers@alioth-lists.debian.net>";
const VCS_ALL: &'static str = "https://salsa.debian.org/rust-team/debcargo-conf";

pub struct Source {
    name: String,
    section: String,
    priority: String,
    maintainer: String,
    uploaders: Vec<String>,
    standards: String,
    build_deps: Vec<String>,
    vcs_git: String,
    vcs_browser: String,
    homepage: String,
    x_cargo: String,
    version: String,
}

pub struct Package {
    name: String,
    arch: String,
    section: Option<String>,
    depends: Vec<String>,
    recommends: Vec<String>,
    suggests: Vec<String>,
    provides: Vec<String>,
    summary: String,
    description: String,
    boilerplate: String,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Source: {}", self.name)?;
        writeln!(f, "Section: {}", self.section)?;
        writeln!(f, "Priority: {}", self.priority)?;
        writeln!(f, "Build-Depends: {}", self.build_deps.join(",\n "))?;
        writeln!(f, "Maintainer: {}", self.maintainer)?;
        writeln!(f, "Uploaders: {}", self.uploaders.join(",\n "))?;
        writeln!(f, "Standards-Version: {}", self.standards)?;
        writeln!(f, "Vcs-Git: {}", self.vcs_git)?;
        writeln!(f, "Vcs-Browser: {}", self.vcs_browser)?;

        if !self.homepage.is_empty() {
            writeln!(f, "Homepage: {}", self.homepage)?;
        }

        if !self.x_cargo.is_empty() {
            writeln!(f, "X-Cargo-Crate: {}", self.x_cargo)?;
        }

        Ok(())
    }
}

impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Package: {}", self.name)?;
        writeln!(f, "Architecture: {}", self.arch)?;
        if let Some(section) = &self.section {
            writeln!(f, "Section: {}", section)?;
        }

        if !self.depends.is_empty() {
            writeln!(f, "Depends:\n {}", self.depends.join(",\n "))?;
        }
        if !self.recommends.is_empty() {
            writeln!(f, "Recommends:\n {}", self.recommends.join(",\n "))?;
        }
        if !self.suggests.is_empty() {
            writeln!(f, "Suggests:\n {}", self.suggests.join(",\n "))?;
        }
        if !self.provides.is_empty() {
            writeln!(f, "Provides:\n {}", self.provides.join(",\n "))?;
        }

        self.write_description(f)
    }
}

impl Source {
    pub fn new(
        upstream_name: &str,
        basename: &str,
        version: &str,
        home: &str,
        lib: bool,
        b_deps: Vec<String>,
    ) -> Result<Source> {
        let source = format!("rust-{}", basename);
        let section = if lib { "rust" } else { "FIXME-(source.section)" };
        let priority = "optional".to_string();
        let maintainer = RUST_MAINT.to_string();
        let uploaders = vec![get_deb_author()?];
        let vcs_browser = VCS_ALL.to_string();
        let vcs_git = format!("{}.git", vcs_browser);

        let mut build_deps = vec![
            "debhelper (>= 11)".to_string(),
            "dh-cargo (>= 3)".to_string(),
        ];
        build_deps.extend(b_deps);
        let cargo_crate = if upstream_name != upstream_name.replace('_', "-") {
            upstream_name.to_string()
        } else {
            "".to_string()
        };
        Ok(Source {
            name: source,
            section: section.to_string(),
            priority: priority,
            maintainer: maintainer,
            uploaders: uploaders,
            standards: "4.1.4".to_string(),
            build_deps: build_deps,
            vcs_git: vcs_git,
            vcs_browser: vcs_browser,
            homepage: home.to_string(),
            x_cargo: cargo_crate,
            version: format!("{}-1", version),
        })
    }

    pub fn srcname(&self) -> &String {
        &self.name
    }

    pub fn version(&self) -> &String {
        &self.version
    }

    pub fn main_uploader(&self) -> &str {
        &self.uploaders[0]
    }
}

impl OverrideDefaults for Source {
    fn apply_overrides(&mut self, config: &Config) {
        if let Some(section) = config.section() {
            self.section = section.to_string();
        }

        if let Some(policy) = config.policy_version() {
            self.standards = policy.to_string();
        }

        self.build_deps.extend(vec_opt_iter(config.build_depends()).map(String::to_string));
        let bdeps_ex = config.build_depends_excludes().map(Vec::as_slice).unwrap_or(&[]);
        self.build_deps.retain(|x| !bdeps_ex.contains(x));

        if let Some(homepage) = config.homepage() {
            self.homepage = homepage.to_string();
        }

        if let Some(vcs_git) = config.vcs_git() {
            self.vcs_git = vcs_git.to_string();
        }

        if let Some(vcs_browser) = config.vcs_browser() {
            self.vcs_browser = vcs_browser.to_string();
        }
    }
}

impl Package {
    pub fn new(
        basename: &str,
        upstream_name: &str,
        summary: Option<&String>,
        description: Option<&String>,
        feature: Option<&str>,
        f_deps: Vec<&str>,
        o_deps: Vec<Dependency>,
        f_provides: Vec<&str>,
        f_recommends: Vec<&str>,
        f_suggests: Vec<&str>,
    ) -> Result<Package> {
        let deb_feature = &|f: &str| {
            format!("{} (= ${{binary:Version}})", if f == "" {
                deb_name(basename)
            } else {
                deb_feature_name(basename, f)
            })
        };

        let (recommends, suggests) = if let None = feature {
            (f_recommends.into_iter().filter(|f| !f_provides.contains(f)).map(deb_feature).collect(),
             f_suggests.into_iter().filter(|f| !f_provides.contains(f)).map(deb_feature).collect())
        } else {
            (vec![], vec![])
        };
        let provides = f_provides.into_iter().map(deb_feature).collect();
        let mut depends = vec!["${misc:Depends}".to_string()];
        depends.extend(f_deps.into_iter().map(deb_feature));
        depends.extend(deb_deps(&o_deps)?);

        let short_desc = match summary {
            None => format!("Rust source code for crate \"{}\"", basename),
            Some(ref s) => if let Some(f) = feature {
                format!("{} - feature \"{}\"", s, f)
            } else {
                format!("{} - Rust source code", s)
            },
        };

        let name = match feature {
            None => deb_name(basename),
            Some(f) => deb_feature_name(basename, f),
        };

        let long_desc = match description {
            None => "".to_string(),
            Some(ref s) => s.to_string(),
        };

        let boilerplate = match feature {
            None => format!(
                concat!(
                    "This package contains the source for the ",
                    "Rust {} crate, packaged by debcargo for use ",
                    "with cargo and dh-cargo."
                ),
                upstream_name
            ),
            Some(f) => format!(
                concat!(
                    "This metapackage enables feature {} for the ",
                    "Rust {} crate, by pulling in any additional ",
                    "dependencies needed by that feature."
                ),
                f,
                upstream_name
            ),
        };

        Ok(Package {
            name: name,
            arch: "all".to_string(),
            section: None,
            depends: depends,
            recommends: recommends,
            suggests: suggests,
            provides: provides,
            summary: short_desc,
            description: fill(&long_desc, 79),
            boilerplate: fill(&boilerplate, 79),
        })
    }

    pub fn new_bin(
        upstream_name: &str,
        name: &str,
        section: Option<&str>,
        summary: &Option<String>,
        description: &Option<String>,
        boilerplate: &str,
    ) -> Self {
        let short_desc = match *summary {
            None => format!("Binaries built from the Rust {} crate", upstream_name),
            Some(ref s) => s.to_string(),
        };

        let long_desc = match *description {
            None => "".to_string(),
            Some(ref s) => s.to_string(),
        };

        Package {
            name: name.to_string(),
            arch: "any".to_string(),
            section: section.map(|s| s.to_string()),
            depends: vec![
                "${misc:Depends}".to_string(),
                "${shlibs:Depends}".to_string(),
            ],
            recommends: vec![],
            suggests: vec![],
            provides: vec![],
            summary: short_desc,
            description: long_desc,
            boilerplate: boilerplate.to_string(),
        }
    }

    pub fn name(&self) -> &String {
        &self.name
    }

    fn write_description(&self, out: &mut fmt::Formatter) -> fmt::Result {
        writeln!(out, "Description: {}", self.summary)?;
        let description = [&self.description, &self.boilerplate].iter().filter_map(|x| {
            let x = x.trim();
            if x.is_empty() { None } else { Some(x) }
        }).join("\n\n");
        for line in description.trim().lines() {
            let line = line.trim();
            if line.is_empty() {
                writeln!(out, " .")?;
            } else if line.starts_with("- ") {
                writeln!(out, "  {}", line)?;
            } else {
                writeln!(out, " {}", line)?;
            }
        }
        Ok(())
    }
}

impl OverrideDefaults for Package {
    fn apply_overrides(&mut self, config: &Config) {
        if let Some(section) = config.package_section(&self.name) {
            self.section = Some(section.to_string());
        }

        if let Some((s, d)) = config.package_summary(&self.name) {
            if !s.is_empty() {
                self.summary = s.to_string();
            }

            if !d.is_empty() {
                self.description = d.to_string();
            }
        }

        self.depends.extend(vec_opt_iter(config.package_depends(&self.name)).map(String::to_string));
    }
}

/// Translates a semver into a Debian version. Omits the build metadata, and uses a ~ before the
/// prerelease version so it compares earlier than the subsequent release.
pub fn deb_version(v: &Version) -> String {
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
    format!(
        "librust-{}+{}-dev",
        name.replace('_', "-"),
        feature.replace('_', "-").to_lowercase()
    )
}

/// Retrieve one of a series of environment variables, and provide a friendly error message for
/// non-UTF-8 values.
fn get_envs(keys: &[&str]) -> Result<Option<String>> {
    for key in keys {
        match env::var(key) {
            Ok(val) => {
                return Ok(Some(val));
            }
            Err(e @ VarError::NotUnicode(_)) => {
                return Err(Error::from(
                    Error::from(e)
                        .context(format!("Environment variable ${} not valid UTF-8", key)),
                ));
            }
            Err(VarError::NotPresent) => {}
        }
    }
    Ok(None)
}

/// Determine a name and email address from environment variables.
pub fn get_deb_author() -> Result<String> {
    let name = get_envs(&["DEBFULLNAME", "NAME"])?.ok_or(format_err!(
        "Unable to determine your name; please set $DEBFULLNAME or $NAME"
    ))?;
    let email = get_envs(&["DEBEMAIL", "EMAIL"])?.ok_or(format_err!(
        "Unable to determine your email; please set $DEBEMAIL or $EMAIL"
    ))?;
    Ok(format!("{} <{}>", name, email))
}
