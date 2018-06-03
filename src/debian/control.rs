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

const RUST_MAINT: &'static str = "Rust Maintainers <pkg-rust-maintainers@alioth-lists.debian.net>";
const VCS_ALL: &'static str = "https://salsa.debian.org/rust-team/debcargo-conf";

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

        Ok(())
    }
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
        t_deps: &Vec<Dependency>,
    ) -> Result<Source> {
        let source = format!("rust-{}", basename);
        let section = if lib { "rust" } else { "FIXME" };
        let priority = "optional".to_string();
        let maintainer = RUST_MAINT.to_string();
        let uploaders = get_deb_author()?;
        let vcs_browser = VCS_ALL.to_string();
        let vcs_git = format!("{}.git", vcs_browser);

        let mut build_deps = vec![
            "debhelper (>= 11)".to_string(),
            "dh-cargo (>= 3)".to_string(),
        ];
        build_deps.extend(deb_deps(t_deps)?.iter().map(|x| {
            x.to_string().split("|").map(|x| {
                x.trim_right().to_string() + " <!nocheck> "
            }).join("|").trim_right().to_string()
        }));
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
            build_deps: build_deps.iter().join(",\n "),
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

    pub fn uploader(&self) -> &str {
        &self.uploaders
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

        if let Some(bdeps) = config.build_depends() {
            let deps = bdeps.iter().join(",\n ");
            self.build_deps.push_str(",\n ");
            self.build_deps.push_str(&deps);
        }

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
        f_deps: &Vec<&str>,
        o_deps: &Vec<Dependency>,
        f_provides: &Vec<&str>,
    ) -> Result<Package> {
        let deb_feature = &|f: &str| {
            if f == "" {
                deb_name(basename)
            } else {
                deb_feature_name(basename, f)
            }
        };

        let deps = f_deps
            .iter()
            .map(|f| format!("{} (= ${{binary:Version}})", deb_feature(f)))
            .collect::<Vec<_>>();

        // Suggests is needed only for main package and not feature package.
        // TODO: FIXME later
        let suggests = if false { "".to_string() /*feature == "" {
            non_default_features
                .iter()
                .cloned()
                .map(deb_feature)
                .join(",\n ")*/
        } else {
            "".to_string()
        };

        let provides = f_provides
                .iter()
                .map(|f| format!("{} (= ${{binary:Version}})", deb_feature(f)))
                .join(",\n ");

        let depends = vec!["${misc:Depends}".to_string()]
            .iter()
            .chain(deps.iter())
            .chain(deb_deps(&o_deps)?.iter())
            .join(",\n ");

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
            Some(s) => deb_feature(s),
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
            section: "".to_string(),
            depends: depends,
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
            section: "misc".to_string(),
            depends: vec![
                "${misc:Depends}".to_string(),
                "${shlibs:Depends}".to_string(),
            ].iter()
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
        if let Some((s, d)) = config.package_summary(&self.name) {
            if !s.is_empty() {
                self.summary = s.to_string();
            }

            if !d.is_empty() {
                self.description = d.to_string();
            }
        }

        if let Some(depends) = config.package_depends(&self.name) {
            let deps = depends.iter().join(",\n ");
            self.depends.push_str(",\n ");
            self.depends.push_str(&deps);
        }
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
