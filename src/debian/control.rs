use std::env::{self, VarError};
use std::fmt::{self, Write};

use anyhow::{format_err, Error};
use semver::Version;
use textwrap::fill;

use crate::config::{self, Config, PackageKey};
use crate::errors::*;

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
    crate_name: String,
    requires_root: String,
}

pub struct Package {
    name: String,
    arch: String,
    multi_arch: String,
    section: Option<String>,
    depends: Vec<String>,
    recommends: Vec<String>,
    suggests: Vec<String>,
    provides: Vec<String>,
    summary: Description,
    description: Description,
    extra_lines: Vec<String>,
}

pub struct Description {
    pub prefix: String,
    pub suffix: String,
}

pub struct PkgTest {
    name: String,
    crate_name: String,
    feature: String,
    version: String,
    extra_test_args: Vec<String>,
    depends: Vec<String>,
    extra_restricts: Vec<String>,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Source: {}", self.name)?;
        writeln!(f, "Section: {}", self.section)?;
        writeln!(f, "Priority: {}", self.priority)?;
        writeln!(f, "Build-Depends: {}", self.build_deps.join(",\n "))?;
        writeln!(f, "Maintainer: {}", self.maintainer)?;
        if !self.uploaders.is_empty() {
            writeln!(f, "Uploaders:\n {}", self.uploaders.join(",\n "))?;
        }
        writeln!(f, "Standards-Version: {}", self.standards)?;
        writeln!(f, "Vcs-Git: {}", self.vcs_git)?;
        writeln!(f, "Vcs-Browser: {}", self.vcs_browser)?;

        if !self.homepage.is_empty() {
            writeln!(f, "Homepage: {}", self.homepage)?;
        }

        // We used to set this conditionally, however it's best to do it
        // unconditionally as some crates' names have a number suffix e.g.
        // "utf-8". Without setting X-Cargo-Crate, we cannot distinguish:
        //   a) "utf" crate at version 8 with semver_suffix = true
        //   b) "utf-8" crate at version latest with semver_suffix = false.
        // dh-cargo assumes (a) which is wrong for the "utf-8" crate
        writeln!(f, "X-Cargo-Crate: {}", self.crate_name)?;
        writeln!(f, "Rules-Requires-Root: {}", self.requires_root)?;

        Ok(())
    }
}

impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Package: {}", self.name)?;
        writeln!(f, "Architecture: {}", self.arch)?;
        writeln!(f, "Multi-Arch: {}", self.multi_arch)?;
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

        for line in &self.extra_lines {
            writeln!(f, "{}", line)?;
        }

        self.write_description(f)
    }
}

impl fmt::Display for PkgTest {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(
            f,
            "Test-Command: /usr/share/cargo/bin/cargo-auto-test {} {} --all-targets {}",
            self.crate_name,
            self.version,
            self.extra_test_args.join(" ")
        )?;
        writeln!(f, "Features: test-name={}:{}", &self.name, &self.feature)?;
        // TODO: drop the below workaround when rust-lang/cargo#5133 is fixed.
        // The downside of our present work-around is that more dependencies
        // must be installed, which makes it harder to actually run the tests
        let cargo_bug_fixed = false;
        let default_deps = if cargo_bug_fixed { &self.name } else { "@" };
        if self.depends.is_empty() {
            writeln!(f, "Depends: dh-cargo (>= 18), {}", default_deps)?;
        } else {
            writeln!(
                f,
                "Depends: dh-cargo (>= 18), {}, {}",
                self.depends.join(", "),
                default_deps
            )?;
        }
        if self.extra_restricts.is_empty() {
            writeln!(f, "Restrictions: allow-stderr, skip-not-installable")?;
        } else {
            writeln!(
                f,
                "Restrictions: allow-stderr, skip-not-installable, {}",
                self.extra_restricts.join(", ")
            )?;
        }
        Ok(())
    }
}

impl Source {
    pub fn pkg_prefix() -> &'static str {
        if config::testing_ruzt() {
            // avoid accidentally installing official packages during tests
            "ruzt"
        } else {
            "rust"
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        basename: &str,
        name_suffix: Option<&str>,
        crate_name: &str,
        home: &str,
        lib: bool,
        maintainer: String,
        uploaders: Vec<String>,
        build_deps: Vec<String>,
        _requires_root: String,
    ) -> Result<Source> {
        let pkgbase = match name_suffix {
            None => basename.to_string(),
            Some(suf) => format!("{}{}", basename, suf),
        };
        let section = if lib {
            "rust"
        } else {
            "FIXME-IN-THE-SOURCE-SECTION"
        };
        let priority = "optional".to_string();
        let vcs_browser = format!(
            "https://salsa.debian.org/rust-team/debcargo-conf/tree/master/src/{}",
            pkgbase
        );
        let vcs_git = format!(
            "https://salsa.debian.org/rust-team/debcargo-conf.git [src/{}]",
            pkgbase
        );
        Ok(Source {
            name: dsc_name(&pkgbase),
            section: section.to_string(),
            priority,
            maintainer,
            uploaders,
            standards: "4.6.0".to_string(),
            build_deps,
            vcs_git,
            vcs_browser,
            homepage: home.to_string(),
            crate_name: crate_name.to_string(),
            requires_root: "no".to_string(),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn apply_overrides(&mut self, config: &Config) {
        if let Some(section) = config.section() {
            self.section = section.to_string();
        }

        if let Some(policy) = config.policy_version() {
            self.standards = policy.to_string();
        }

        self.build_deps.extend(
            config
                .build_depends()
                .into_iter()
                .flatten()
                .map(String::to_string),
        );
        let bdeps_ex = config
            .build_depends_excludes()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
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

        if let Some(requires_root) = config.requires_root() {
            self.requires_root = requires_root.to_string();
        }
    }
}

impl Package {
    pub fn pkg_prefix() -> &'static str {
        if config::testing_ruzt() {
            // avoid accidentally installing official packages during tests
            "libruzt"
        } else {
            "librust"
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        basename: &str,
        name_suffix: Option<&str>,
        version: &Version,
        summary: Description,
        description: Description,
        feature: Option<&str>,
        f_deps: Vec<&str>,
        o_deps: Vec<String>,
        f_provides: Vec<&str>,
        f_recommends: Vec<&str>,
        f_suggests: Vec<&str>,
    ) -> Result<Package> {
        let pkgbase = match name_suffix {
            None => basename.to_string(),
            Some(suf) => format!("{}{}", basename, suf),
        };
        let deb_feature2 = &|p: &str, f: &str| {
            format!(
                "{} (= ${{binary:Version}})",
                match f {
                    "" => deb_name(p),
                    _ => deb_feature_name(p, f),
                }
            )
        };
        let deb_feature = &|f: &str| deb_feature2(&pkgbase, f);

        let filter_provides = &|x: Vec<&str>| {
            x.into_iter()
                .filter(|f| !f_provides.contains(f))
                .map(deb_feature)
                .collect()
        };
        let (recommends, suggests) = match feature {
            Some(_) => (vec![], vec![]),
            None => (filter_provides(f_recommends), filter_provides(f_suggests)),
        };

        // Provides for all possible versions, see:
        // https://bugs.debian.org/cgi-bin/bugreport.cgi?bug=901827#35
        // https://wiki.debian.org/Teams/RustPackaging/Policy#Package_provides
        let mut provides = vec![];
        let version_suffixes = vec![
            "".to_string(),
            format!("-{}", version.major),
            format!("-{}.{}", version.major, version.minor),
            format!("-{}.{}.{}", version.major, version.minor, version.patch),
        ];
        for suffix in version_suffixes.iter() {
            let p = format!("{}{}", basename, suffix);
            provides.push(deb_feature2(&p, feature.unwrap_or("")));
            provides.extend(f_provides.iter().map(|f| deb_feature2(&p, f)));
        }
        let provides_self = deb_feature(feature.unwrap_or(""));
        // rust dropped Vec::remove_item for annoying reasons, the below is
        // an unofficial recommended replacement from the RFC #40062
        let i = provides.iter().position(|x| *x == *provides_self);
        i.map(|i| provides.remove(i));

        let mut depends = vec!["${misc:Depends}".to_string()];
        if feature.is_some() && !f_deps.contains(&"") {
            // in dh-cargo we symlink /usr/share/doc/{$feature => $main} pkg
            // so we always need this direct dependency, even if the feature
            // only indirectly depends on the bare library via another
            depends.push(deb_feature(""));
        }
        depends.extend(f_deps.into_iter().map(deb_feature));
        depends.extend(o_deps);

        Ok(Package {
            name: match feature {
                None => deb_name(&pkgbase),
                Some(f) => deb_feature_name(&pkgbase, f),
            },
            arch: "any".to_string(),
            // This is the best but not ideal option for us.
            //
            // Currently Debian M-A spec has a deficiency where a package X that
            // build-depends on a (M-A:foreign+arch:all) package that itself
            // depends on an arch:any package Z, will pick up the BUILD_ARCH of
            // package Z instead of the HOST_ARCH. This is because we currently
            // have no way of telling dpkg to use HOST_ARCH when checking that the
            // dependencies of Y are satisfied, which is done at install-time
            // without any knowledge that we're about to do a cross-compile. It
            // is also problematic to tell dpkg to "accept any arch" because of
            // the presence of non-M-A:same packages in the archive, that are not
            // co-installable - different arches of Z might be depended-upon by
            // two conflicting chains. (dpkg has so far chosen not to add an
            // exception for the case where package Z is M-A:same co-installable).
            //
            // The recommended work-around for now from the dpkg developers is to
            // make our packages arch:any M-A:same even though this results in
            // duplicate packages in the Debian archive. For very large crates we
            // will eventually want to make debcargo generate -data packages that
            // are arch:all and have the arch:any -dev packages depend on it.
            multi_arch: "same".to_string(),
            section: None,
            depends,
            recommends,
            suggests,
            provides,
            summary,
            description,
            extra_lines: match (name_suffix, feature) {
                (Some(_), None) => {
                    let fullpkg = format!("{}-{}", basename, version);
                    vec![
                        format!("Replaces: {}", deb_name(&fullpkg)),
                        format!("Breaks: {}", deb_name(&fullpkg)),
                    ]
                }
                (_, _) => vec![],
            },
        })
    }

    pub fn new_bin(
        basename: &str,
        name_suffix: Option<&str>,
        section: Option<&str>,
        summary: Description,
        description: Description,
    ) -> Self {
        let (name, mut provides) = match name_suffix {
            None => (basename.to_string(), vec![]),
            Some(suf) => (
                format!("{}{}", basename, suf),
                vec![format!("{} (= ${{binary:Version}})", basename)],
            ),
        };
        provides.push("${cargo:Provides}".to_string());
        Package {
            name,
            arch: "any".to_string(),
            multi_arch: "allowed".to_string(),
            section: section.map(|s| s.to_string()),
            depends: vec![
                "${misc:Depends}".to_string(),
                "${shlibs:Depends}".to_string(),
                "${cargo:Depends}".to_string(),
            ],
            recommends: vec!["${cargo:Recommends}".to_string()],
            suggests: vec!["${cargo:Suggests}".to_string()],
            provides,
            summary,
            description,
            extra_lines: vec![
                "Built-Using: ${cargo:Built-Using}".to_string(),
                "XB-X-Cargo-Built-Using: ${cargo:X-Cargo-Built-Using}".to_string(),
            ],
        }
    }

    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    fn write_description(&self, out: &mut fmt::Formatter) -> fmt::Result {
        writeln!(out, "Description: {}", &self.summary)?;
        let description = format!("{}", &self.description);
        for line in fill(description.trim(), 79).lines() {
            let line = line.trim_end();
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

    #[allow(clippy::result_unit_err)]
    pub fn summary_check_len(&self) -> std::result::Result<(), ()> {
        if self.summary.prefix.len() <= 80 {
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn apply_overrides(&mut self, config: &Config, key: PackageKey, f_provides: Vec<&str>) {
        if let Some(section) = config.package_section(key) {
            self.section = Some(section.to_string());
        }
        self.summary
            .apply_overrides(&config.summary, config.package_summary(key));
        self.description
            .apply_overrides(&config.description, config.package_description(key));

        self.depends.extend(config::package_field_for_feature(
            &|x| config.package_depends(x),
            key,
            &f_provides,
        ));
        self.recommends.extend(config::package_field_for_feature(
            &|x| config.package_recommends(x),
            key,
            &f_provides,
        ));
        self.suggests.extend(config::package_field_for_feature(
            &|x| config.package_suggests(x),
            key,
            &f_provides,
        ));
        self.provides.extend(config::package_field_for_feature(
            &|x| config.package_provides(x),
            key,
            &f_provides,
        ));

        self.extra_lines.extend(
            config
                .package_extra_lines(key)
                .into_iter()
                .flatten()
                .map(|s| s.to_string()),
        );
    }
}

impl Description {
    fn apply_overrides(&mut self, global: &Option<String>, per_package: Option<&str>) {
        if let Some(per_package) = per_package {
            self.prefix = per_package.to_string();
            self.suffix = "".to_string();
        } else if let Some(global) = &global {
            self.prefix = global.into();
        }
    }
}
impl fmt::Display for Description {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}{}", &self.prefix, self.suffix)
    }
}

impl PkgTest {
    pub fn new(
        name: &str,
        crate_name: &str,
        feature: &str,
        version: &str,
        extra_test_args: Vec<&str>,
        depends: &[String],
        extra_restricts: Vec<&str>,
    ) -> Result<PkgTest> {
        Ok(PkgTest {
            name: name.to_string(),
            crate_name: crate_name.to_string(),
            feature: feature.to_string(),
            version: version.to_string(),
            extra_test_args: extra_test_args.iter().map(|x| x.to_string()).collect(),
            depends: depends.to_vec(),
            extra_restricts: extra_restricts.iter().map(|x| x.to_string()).collect(),
        })
    }
}

/// Translates a semver into a Debian-format upstream version.
/// Omits the build metadata, and uses a ~ before the prerelease version so it
/// compares earlier than the subsequent release.
pub fn deb_upstream_version(v: &Version) -> String {
    let mut s = format!("{}.{}.{}", v.major, v.minor, v.patch);
    if !v.pre.is_empty() {
        write!(s, "~{}", v.pre.as_str()).unwrap();
    }
    s
}

pub fn base_deb_name(crate_name: &str) -> String {
    crate_name.replace('_', "-").to_lowercase()
}

pub fn dsc_name(name: &str) -> String {
    format!("{}-{}", Source::pkg_prefix(), base_deb_name(name))
}

pub fn deb_name(name: &str) -> String {
    format!("{}-{}-dev", Package::pkg_prefix(), base_deb_name(name))
}

pub fn deb_feature_name(name: &str, feature: &str) -> String {
    format!(
        "{}-{}+{}-dev",
        Package::pkg_prefix(),
        base_deb_name(name),
        base_deb_name(feature)
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
                return Err(Error::from(e)
                    .context(format!("Environment variable ${} not valid UTF-8", key)));
            }
            Err(VarError::NotPresent) => {}
        }
    }
    Ok(None)
}

/// Determine a name and email address from environment variables.
pub fn get_deb_author() -> Result<String> {
    let name = get_envs(&["DEBFULLNAME", "NAME"])?.ok_or_else(|| {
        format_err!("Unable to determine your name; please set $DEBFULLNAME or $NAME")
    })?;
    let email = get_envs(&["DEBEMAIL", "EMAIL"])?.ok_or_else(|| {
        format_err!("Unable to determine your email; please set $DEBEMAIL or $EMAIL")
    })?;
    Ok(format!("{} <{}>", name, email))
}
