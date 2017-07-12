use std::fmt;
use chrono;
use itertools::Itertools;
use semver::Version;

use overrides::{Overrides, OverrideDefaults};
use errors::*;

use debian::control::get_deb_author;

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
    pub fn new(upstream_name: &str,
               basename: &str,
               version: &str,
               home: &str,
               lib: &bool,
               bdeps: &[String])
               -> Result<Source> {
        let source = format!("rust-{}", basename);
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
        let cargo_crate = if upstream_name != upstream_name.replace('_', "-") {
            upstream_name.clone()
        } else {
            ""
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
            x_cargo: cargo_crate.to_string(),
            version: format!("{}-1", version),
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
