use std::fs;
use std::io::{self, ErrorKind, Read, Seek, Write as IoWrite};
use std::path::Path;
use std::os::unix::fs::PermissionsExt;
use std::str::FromStr;

use cargo::util::FileLock;
use glob::Pattern;
use tempdir::TempDir;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use regex::Regex;
use tar::{Archive, Builder};

use crates::CrateInfo;
use errors::*;
use config::{Config, package_key_for_feature, PACKAGE_KEY_FOR_BIN};
use util::{self, copy_tree, vec_opt_iter};

use self::control::deb_version;
use self::control::{Package, Source};
use self::copyright::debian_copyright;
use self::changelog::{ChangelogEntry, ChangelogIterator};
pub use self::dependency::{deb_deps, deb_dep_add_nocheck};

pub mod control;
mod dependency;
pub mod copyright;
pub mod changelog;

pub struct BaseInfo {
    upstream_name: String,
    base_package_name: String,
    name_suffix: Option<String>,
    package_name: String,
    debian_version: String,
    debcargo_version: String,
    package_source_dir: String,
    orig_tarball_path: String,
}

impl BaseInfo {
    pub fn new(name: &str, crate_info: &CrateInfo,
               debcargo_version: &str, semver_suffix: bool) -> Self {
        let upstream = name.to_string();
        let name_dashed = upstream.replace('_', "-");
        let base_package_name = name_dashed.to_lowercase();
        let (name_suffix, package_name) = if semver_suffix {
            (Some(crate_info.semver_suffix()),
             format!("{}{}", base_package_name, crate_info.semver_suffix()))
        } else {
            (None, base_package_name.clone())
        };
        let debian_version = deb_version(crate_info.version());
        let debian_source = match name_suffix {
            Some(ref suf) => format!("rust-{}{}", base_package_name, suf),
            None => format!("rust-{}", base_package_name),
        };
        let package_source_dir = format!("{}-{}", debian_source, debian_version);
        let orig_tarball_path = format!("{}_{}.orig.tar.gz", debian_source, debian_version);

        BaseInfo {
            upstream_name: upstream,
            base_package_name: base_package_name,
            name_suffix: name_suffix,
            package_name: package_name,
            debian_version: debian_version,
            debcargo_version: debcargo_version.to_string(),
            package_source_dir: package_source_dir,
            orig_tarball_path: orig_tarball_path,
        }
    }

    pub fn upstream_name(&self) -> &str {
        self.upstream_name.as_str()
    }

    pub fn base_package_name(&self) -> &str {
        self.base_package_name.as_str()
    }

    pub fn name_suffix(&self) -> Option<&str> {
        self.name_suffix.as_ref().map(|s| s.as_str())
    }

    pub fn package_name(&self) -> &str {
        self.package_name.as_str()
    }

    pub fn debian_version(&self) -> &str {
        self.debian_version.as_str()
    }

    pub fn debcargo_version(&self) -> &str {
        self.debcargo_version.as_str()
    }

    pub fn package_source_dir(&self) -> &str {
        self.package_source_dir.as_str()
    }

    pub fn orig_tarball_path(&self) -> &str {
        self.orig_tarball_path.as_str()
    }

}

pub fn prepare_orig_tarball(
    crate_file: &FileLock,
    tarball: &Path,
    src_modified: bool,
    pkg_srcdir: &Path,
    excludes: &Vec<Pattern>,
) -> Result<()> {
    let tempdir = TempDir::new_in(".", "debcargo")?;
    let temp_archive_path = tempdir.path().join(tarball);

    let mut create = fs::OpenOptions::new();
    create.write(true).create_new(true);

    if src_modified {
        let mut f = crate_file.file();
        f.seek(io::SeekFrom::Start(0))?;
        let mut archive = Archive::new(GzDecoder::new(f));
        let mut new_archive = Builder::new(GzEncoder::new(
            create.open(&temp_archive_path)?,
            Compression::best(),
        ));

        for entry in archive.entries()? {
            let entry = entry?;
            let path = entry.path()?.into_owned();
            if path.ends_with("Cargo.toml") && path.iter().count() == 2 {
                // Put the rewritten and original Cargo.toml back into the orig tarball
                let mut new_archive_append = |name: &str| {
                    let mut header = entry.header().clone();
                    let srcpath = pkg_srcdir.join(name);
                    header.set_path(path.parent().unwrap().join(name))?;
                    header.set_size(fs::metadata(&srcpath)?.len());
                    header.set_cksum();
                    new_archive.append(&header, fs::File::open(&srcpath)?)
                };
                new_archive_append("Cargo.toml")?;
                new_archive_append("Cargo.toml.orig")?;
                writeln!(
                    io::stderr(),
                    "Rewrote {:?} to canonical form.",
                    &entry.path()?
                )?;
            } else if !CrateInfo::filter_path(excludes, &entry.path()?) {
                new_archive.append(&entry.header().clone(), entry)?;
            } else {
                writeln!(
                    io::stderr(),
                    "Filtered out files from .orig.tar.gz: {:?}",
                    &entry.path()?
                )?;
            }
        }

        new_archive.finish()?;
    } else {
        fs::copy(crate_file.path(), &temp_archive_path)?;
    }

    fs::rename(temp_archive_path, &tarball)?;
    Ok(())
}

pub fn prepare_debian_folder(
    pkgbase: &BaseInfo,
    crate_info: &CrateInfo,
    pkg_srcdir: &Path,
    config_path: Option<&Path>,
    config: &Config,
    changelog_ready: bool,
    copyright_guess_harder: bool,
) -> Result<()> {
    let lib = crate_info.is_lib();
    let mut bins = crate_info.get_binary_targets();
    let meta = crate_info.metadata();

    if lib && !bins.is_empty() && !config.build_bin_package() {
        bins.clear();
    }
    let default_bin_name = crate_info.package().name().to_string().replace('_', "-");
    let bin_name = if config.bin_name.eq(&Config::default().bin_name) {
        if !bins.is_empty() {
            debcargo_info!(
                "Generate binary crate with default name '{}', set bin_name to override or bin = false to disable.",
                &default_bin_name
            );
        }
        &default_bin_name
    } else {
        &config.bin_name
    };

    let mut features_with_deps = crate_info.all_dependencies_and_features();
    /*debcargo_info!("features_with_deps: {:?}", features_with_deps
        .iter()
        .map(|(&f, &(ref ff, ref dd))| {
            (f, (ff, deb_deps(config, dd).unwrap()))
        }).collect::<Vec<_>>());*/

    let mut create = fs::OpenOptions::new();
    create.write(true).create_new(true);

    let tempdir = TempDir::new_in(".", "debcargo")?;
    let base_pkgname = pkgbase.base_package_name();
    let name_suffix = pkgbase.name_suffix();
    let upstream_name = pkgbase.upstream_name();

    let overlay = config.overlay_dir(config_path);

    overlay.as_ref().map(|p| {
        copy_tree(p.as_path(), tempdir.path()).unwrap();
    });
    if tempdir.path().join("control").exists() {
        debcargo_warn!("Most of the time you shouldn't overlay debian/control, \
                        it's a maintenance burden. Use debcargo.toml instead.")
    }

    let mut new_hints = vec![];
    {
        let mut file = |name: &str| {
            let path = tempdir.path();
            create.open(path.join(name)).or_else(|e| match e.kind() {
                ErrorKind::AlreadyExists => {
                    let hintname = name.to_owned() + util::HINT_SUFFIX;
                    let hint = path.join(&hintname);
                    if hint.exists() {
                        fs::remove_file(&hint)?;
                    }
                    new_hints.push(hintname);
                    create.open(&hint)
                }
                _ => Err(e),
            })
        };

        // debian/cargo-checksum.json
        let checksum = crate_info
            .checksum()
            .unwrap_or("Could not get crate checksum");
        let mut cargo_checksum_json = file("cargo-checksum.json")?;
        writeln!(
            cargo_checksum_json,
            r#"{{"package":"{}","files":{{}}}}"#,
            checksum
        )?;

        // debian/compat
        let mut compat = file("compat")?;
        writeln!(compat, "11")?;

        // debian/copyright
        let mut copyright = io::BufWriter::new(file("copyright")?);
        let dep5_copyright = debian_copyright(
            crate_info.package(),
            pkg_srcdir,
            crate_info.manifest(),
            copyright_guess_harder,
        )?;
        write!(copyright, "{}", dep5_copyright)?;

        // debian/watch
        let mut watch = file("watch")?;
        writeln!(
            watch,
            "{}",
            format!(
                concat!(
                    "version=4\n",
                    "opts=filenamemangle=s/.*\\/(.*)\\/download/{name}-$1\\.\
                     tar\\.gz/g\\ \n",
                    " https://qa.debian.org/cgi-bin/fakeupstream.\
                     cgi?upstream=crates.io/{name} ",
                    ".*/crates/{name}/@ANY_VERSION@/download\n"
                ),
                name = upstream_name
            )
        )?;

        // debian/source/format
        fs::create_dir(tempdir.path().join("source"))?;
        let mut source_format = file("source/format")?;
        writeln!(source_format, "3.0 (quilt)")?;

        // debian/rules
        let mut rules = file("rules")?;
        rules.set_permissions(fs::Permissions::from_mode(0o777))?;
        write!(
            rules,
            "{}",
            concat!(
                "#!/usr/bin/make -f\n",
                "%:\n",
                "\tdh $@ --buildsystem cargo\n"
            )
        )?;

        // debian/control
        let build_deps = {
            let build_deps = [
                "debhelper (>= 11)",
                "dh-cargo (>= 6)"
                ].iter().map(|x| x.to_string());
            let (default_features, default_deps) = crate_info.feature_all_deps(&features_with_deps, "default");
            //debcargo_info!("default_features: {:?}", default_features);
            //debcargo_info!("default_deps: {:?}", deb_deps(config, &default_deps)?);
            let extra_override_deps = config.package_depends_for_feature("default", default_features);
            let build_deps_extra = [
                "cargo:native",
                "rustc:native",
                "libstd-rust-dev",
                ].iter().map(|s| s.to_string())
                .chain(deb_deps(config, &default_deps)?)
                .chain(extra_override_deps.map(|s| s.to_string()));
            if !bins.is_empty() {
                build_deps.chain(build_deps_extra).collect()
            } else {
                assert!(lib);
                build_deps.chain(build_deps_extra.map(|d| deb_dep_add_nocheck(&d))).collect()
            }
        };
        let uploaders = if changelog_ready {
            // Use changelog maintainer as uploader if --changelog-ready
            // helps to give contributors more visibility in sponsored packages
            let (_changelog, changelog_data) = changelog_or_new(tempdir.path())?;
            match ChangelogIterator::from(&changelog_data).next() {
                Some(x) => {
                    let e = ChangelogEntry::from_str(x)?;
                    vec![e.maintainer]
                },
                None => vec![control::get_deb_author()?]
            }
        } else {
            vec![control::get_deb_author()?]
        };
        let mut source = Source::new(
            base_pkgname,
            name_suffix,
            upstream_name,
            if let Some(ref home) = meta.homepage { home } else { "" },
            lib,
            uploaders,
            build_deps
        )?;

        // If source overrides are present update related parts.
        source.apply_overrides(config);

        let mut control = io::BufWriter::new(file("control")?);
        write!(control, "{}", source)?;

        // Summary and description generated from Cargo.toml
        let (summary, description) = crate_info.get_summary_description();
        let summary = if !config.summary.is_empty() {
            Some(&config.summary)
        } else {
            if let Some(summary) = summary.as_ref() {
                if summary.len() > 80 {
                    writeln!(control, "\n{}", concat!(
                        "# FIXME (packages.\"(name)\".section) debcargo ",
                        "auto-generated summaries are very long, consider overriding"))?;
                }
            }
            summary.as_ref()
        };

        if lib {
            let mut provides = crate_info.calculate_provides(&mut features_with_deps);
            //debcargo_info!("provides: {:?}", provides);
            let mut recommends = vec![];
            let mut suggests = vec![];
            for (&feature, &ref features) in provides.iter() {
                if feature == "" {
                    continue;
                } else if feature == "default" || features.contains(&"default") {
                    recommends.push(feature);
                } else {
                    suggests.push(feature);
                }
            }
            for (feature, (f_deps, o_deps)) in features_with_deps.into_iter() {
                let f_provides = provides.remove(feature).unwrap();
                let mut package =
                    Package::new(base_pkgname, name_suffix, &crate_info.version(), upstream_name,
                        summary, description.as_ref(),
                        if feature == "" { None } else { Some(feature) },
                        f_deps, deb_deps(config, &o_deps)?,
                        f_provides.clone(),
                        if feature == "" { recommends.clone() } else { vec![] },
                        if feature == "" { suggests.clone() } else { vec![] })?;

                // If any overrides present for this package it will be taken care.
                package.apply_overrides(config, &package_key_for_feature(feature),
                    config.package_depends_for_feature(feature, f_provides));
                write!(control, "\n{}", package)?;
            }
            assert!(provides.is_empty());
            // features_with_deps consumed by into_iter, no longer usable
        }

        if !bins.is_empty() {
            let boilerplate = Some(format!(
                "This package contains the following binaries built from the Rust crate\n\"{}\":\n - {}",
                upstream_name,
                bins.join("\n - ")
            ));

            let mut bin_pkg = Package::new_bin(
                bin_name,
                name_suffix,
                upstream_name,
                // if not-a-lib then Source section is already FIXME
                if !lib { None } else { Some("FIXME-(packages.\"(name)\".section)") },
                summary,
                description.as_ref(),
                match boilerplate {
                    Some(ref s) => s,
                    None => "",
                },
            );

            // Binary package overrides.
            bin_pkg.apply_overrides(config, PACKAGE_KEY_FOR_BIN,
                vec_opt_iter(config.package_depends(PACKAGE_KEY_FOR_BIN)));
            write!(control, "\n{}", bin_pkg)?;
        }

        // debian/changelog
        if !changelog_ready {
            let autogenerated_item = format!(
                "  * Package {} {} from crates.io using debcargo {}",
                crate_info.package_id().name(),
                crate_info.package_id().version(),
                pkgbase.debcargo_version()
            );
            let autogenerated_re = Regex::new(
                r"^  \* Package (.*) (.*) from crates.io using debcargo (.*)$"
            ).unwrap();

            // Special-case d/changelog:
            // - Always prepend to any existing file from the overlay.
            // - If the first entry is changelog::DEFAULT_DIST then write over that, smartly
            let (mut changelog, changelog_data) = changelog_or_new(tempdir.path())?;
            let (changelog_old, changelog_items, deb_version_suffix) = match ChangelogIterator::from(&changelog_data).next() {
                Some(x) => if x.contains(changelog::DEFAULT_DIST) {
                    let mut e = ChangelogEntry::from_str(x)?;
                    let (ups, suf) = e.version_parts();
                    if let Some(pos) = e.items.iter().position(|x| autogenerated_re.is_match(x)) {
                        e.items[pos] = autogenerated_item;
                    } else {
                        e.items.push(autogenerated_item);
                    }
                    (&changelog_data[x.len()..],
                     e.items,
                     if ups == pkgbase.debian_version() { suf } else { "1".to_string() })
                } else {
                    let e = ChangelogEntry::from_str(x)?;
                    let (ups, _suf) = e.version_parts();
                    (changelog_data.as_str(),
                     vec![autogenerated_item],
                     if ups == pkgbase.debian_version() { e.deb_version_suffix_bump() } else { "1".to_string() })
                },
                None => {
                    (changelog_data.as_str(),
                     vec![autogenerated_item],
                     "1".to_string())
                }
            };

            let source_deb_version = format!("{}-{}", pkgbase.debian_version(), &deb_version_suffix);
            let changelog_new_entry = ChangelogEntry::new(
                source.srcname().to_string(),
                source_deb_version,
                changelog::DEFAULT_DIST.to_string(),
                "urgency=medium".to_string(),
                source.main_uploader().to_string(),
                changelog::local_now(),
                changelog_items,
            );

            changelog.seek(io::SeekFrom::Start(0))?;
            if changelog_old.is_empty() {
                write!(changelog, "{}", changelog_new_entry)?;
            } else {
                write!(changelog, "{}\n{}", changelog_new_entry, changelog_old)?;
            }
            // the new file might be shorter, truncate it to the current cursor position
            let pos = changelog.seek(io::SeekFrom::Current(0))?;
            changelog.set_len(pos)?;
        }
    }

    if config.overlay_write_back {
        overlay.as_ref().map(|p| {
            if !changelog_ready {
                // Special-case d/changelog:
                // Always write it back, this is safe because of our prepending logic
                new_hints.push("changelog".to_string());
            }
            for hint in &new_hints {
                let newpath = tempdir.path().join(hint);
                let oldpath = p.join(hint);
                fs::copy(newpath, oldpath).expect("could not write back");
                debcargo_info!("Wrote back file to overlay: {}", hint);
            }
        });
    }

    fs::rename(tempdir.path(), pkg_srcdir.join("debian"))?;
    Ok(())
}

fn changelog_or_new(tempdir: &Path) -> Result<(fs::File, String)> {
    let mut changelog = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(tempdir.join("changelog"))?;
    let mut changelog_data = String::new();
    changelog.read_to_string(&mut changelog_data)?;
    Ok((changelog, changelog_data))
}
