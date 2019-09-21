use std::collections::BTreeMap;
use std::fs;
use std::io::{self, ErrorKind, Read, Seek, Write as IoWrite};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;

use chrono::{self, Datelike};
use failure::format_err;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use regex::Regex;
use tar::{Archive, Builder};
use tempfile;

use crate::config::{package_field_for_feature, Config, PackageKey};
use crate::crates::CrateInfo;
use crate::errors::*;
use crate::util::{self, copy_tree, expect_success, vec_opt_iter};

use self::changelog::{ChangelogEntry, ChangelogIterator};
use self::control::deb_version;
use self::control::{Package, PkgTest, Source};
use self::copyright::debian_copyright;
pub use self::dependency::{deb_dep_add_nocheck, deb_deps};

pub mod changelog;
pub mod control;
pub mod copyright;
mod dependency;

pub struct BaseInfo {
    upstream_name: String,
    base_package_name: String,
    name_suffix: Option<String>,
    uscan_version_pattern: Option<String>,
    package_name: String,
    debian_version: String,
    debcargo_version: String,
    package_source_dir: String,
    orig_tarball_path: String,
}

impl BaseInfo {
    pub fn new(
        name: &str,
        crate_info: &CrateInfo,
        debcargo_version: &str,
        semver_suffix: bool,
    ) -> Self {
        let upstream = name.to_string();
        let name_dashed = upstream.replace('_', "-");
        let base_package_name = name_dashed.to_lowercase();
        let (name_suffix, uscan_version_pattern, package_name) = if semver_suffix {
            (
                Some(crate_info.semver_suffix()),
                Some(crate_info.semver_uscan_pattern()),
                format!("{}{}", base_package_name, crate_info.semver_suffix()),
            )
        } else {
            (None, None, base_package_name.clone())
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
            base_package_name,
            name_suffix,
            uscan_version_pattern,
            package_name,
            debian_version,
            debcargo_version: debcargo_version.to_string(),
            package_source_dir,
            orig_tarball_path,
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

fn traverse_depth_2<'a, T>(
    map: &BTreeMap<&'a str, (Vec<&'a str>, T)>,
    key: &'a str,
) -> Vec<&'a str> {
    let mut x = Vec::new();
    if let Some((pp, _)) = (*map).get(key) {
        x.extend(pp);
        for p in pp {
            x.extend(traverse_depth_2(map, p));
        }
    }
    x
}

pub fn prepare_orig_tarball(
    crate_info: &CrateInfo,
    tarball: &Path,
    src_modified: bool,
    pkg_srcdir: &Path,
) -> Result<()> {
    let crate_file = crate_info.crate_file();
    let tempdir = tempfile::Builder::new()
        .prefix("debcargo")
        .tempdir_in(".")?;
    let temp_archive_path = tempdir.path().join(tarball);

    let mut create = fs::OpenOptions::new();
    create.write(true).create_new(true);

    if src_modified {
        debcargo_info!("crate tarball was modified; repacking for debian");
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
            } else {
                match crate_info.filter_path(&entry.path()?) {
                    Err(e) => debcargo_bail!(e),
                    Ok(r) => {
                        if !r {
                            new_archive.append(&entry.header().clone(), entry)?;
                        } else {
                            writeln!(
                                io::stderr(),
                                "Filtered out files from .orig.tar.gz: {:?}",
                                &entry.path()?
                            )?;
                        }
                    }
                }
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
    crate_info: &mut CrateInfo,
    pkg_srcdir: &Path,
    config_path: Option<&Path>,
    config: &Config,
    changelog_ready: bool,
    copyright_guess_harder: bool,
    overlay_write_back: bool,
) -> Result<()> {
    let tempdir = tempfile::Builder::new()
        .prefix("debcargo")
        .tempdir_in(".")?;
    let overlay = config.overlay_dir(config_path);
    if let Some(p) = overlay.as_ref() {
        copy_tree(p.as_path(), tempdir.path()).unwrap();
    }
    if tempdir.path().join("control").exists() {
        debcargo_warn!(
            "Most of the time you shouldn't overlay debian/control, \
             it's a maintenance burden. Use debcargo.toml instead."
        )
    }
    if tempdir.path().join("patches").join("series").exists() {
        // apply patches to Cargo.toml in case they exist, and re-read it
        let pkg_srcdir = &fs::canonicalize(&pkg_srcdir)?;
        expect_success(
            Command::new("quilt")
                .current_dir(&pkg_srcdir)
                .env("QUILT_PATCHES", tempdir.path().join("patches"))
                .args(&["push", "--quiltrc=-", "-a"]),
            "failed to apply patches",
        );
        crate_info.replace_manifest(&pkg_srcdir.join("Cargo.toml"))?;
        expect_success(
            Command::new("quilt")
                .current_dir(&pkg_srcdir)
                .env("QUILT_PATCHES", tempdir.path().join("patches"))
                .args(&["pop", "--quiltrc=-", "-a"]),
            "failed to unapply patches",
        );
    }

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
    let dev_depends = deb_deps(config, &crate_info.dev_dependencies())?;
    /*debcargo_info!("features_with_deps: {:?}", features_with_deps
    .iter()
    .map(|(&f, &(ref ff, ref dd))| {
        (f, (ff, deb_deps(config, dd).unwrap()))
    }).collect::<Vec<_>>());*/

    let mut create = fs::OpenOptions::new();
    create.write(true).create_new(true);

    let crate_name = crate_info.package_id().name();
    let crate_version = crate_info.package_id().version();
    let base_pkgname = pkgbase.base_package_name();
    let name_suffix = pkgbase.name_suffix();
    let upstream_name = pkgbase.upstream_name();

    let mut new_hints = vec![];
    {
        let mut file = |name: &str| {
            let path = tempdir.path();
            let f = path.join(name);
            fs::create_dir_all(f.parent().unwrap())?;
            create.open(&f).or_else(|e| match e.kind() {
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
        let uploaders: Vec<&str> = vec_opt_iter(config.uploaders())
            .map(String::as_str)
            .collect();
        let mut copyright = io::BufWriter::new(file("copyright")?);
        let year_range = if changelog_ready {
            // if changelog is ready, unconditionally read the year range from it
            changelog_first_last(tempdir.path())?
        } else {
            // otherwise use the first date if it exists
            let last = chrono::Local::now().year();
            match changelog_first_last(tempdir.path()) {
                Ok((first, _)) => (first, last),
                Err(_) => (last, last),
            }
        };
        let dep5_copyright = debian_copyright(
            crate_info.package(),
            pkg_srcdir,
            crate_info.manifest(),
            &uploaders,
            year_range,
            copyright_guess_harder,
        )?;
        write!(copyright, "{}", dep5_copyright)?;

        // debian/watch
        let mut watch = file("watch")?;
        let uscan_version_pattern = pkgbase
            .uscan_version_pattern
            .as_ref()
            .map_or_else(|| "@ANY_VERSION@".to_string(), |ref s| s.to_string());
        let uscan_lines = &[
            "version=4".into(),
            format!(
                r"opts=filenamemangle=s/.*\/(.*)\/download/{name}-$1\.tar\.gz/g,\",
                name = upstream_name
            ),
            r"uversionmangle=s/(\d)[_\.\-\+]?((RC|rc|pre|dev|beta|alpha)\d*)$/$1~$2/ \".into(),
            format!(
                "https://qa.debian.org/cgi-bin/fakeupstream.cgi?upstream=crates.io/{name} \
                 .*/crates/{name}/{version_pattern}/download",
                name = upstream_name,
                version_pattern = uscan_version_pattern
            ),
        ];
        for line in uscan_lines {
            writeln!(watch, "{}", line)?;
        }

        // debian/source/format
        fs::create_dir_all(tempdir.path().join("source"))?;
        let mut source_format = file("source/format")?;
        writeln!(source_format, "3.0 (quilt)")?;

        let (all_features_test_broken, broken_tests) = {
            let is_broken = |f: &str| {
                config
                    .package_test_is_broken(PackageKey::feature(f))
                    .unwrap_or(false)
            };
            let mut broken_tests: BTreeMap<&str, bool> = BTreeMap::new();
            let mut any_test_broken = false;
            for (feature, _) in features_with_deps.iter() {
                let broken = is_broken(feature);
                if broken {
                    any_test_broken = true;
                }
                let all_deps = traverse_depth_2(&features_with_deps, feature);
                broken_tests.insert(feature, broken || all_deps.iter().any(|f| is_broken(f)));
            }
            (is_broken("@") || any_test_broken, broken_tests)
        };
        let test_is_broken_for = |f: &str| *broken_tests.get(f).unwrap();

        // debian/rules
        let mut rules = file("rules")?;
        rules.set_permissions(fs::Permissions::from_mode(0o777))?;
        if !dev_depends.is_empty() {
            write!(
                rules,
                "{}",
                concat!(
                    "#!/usr/bin/make -f\n",
                    "%:\n",
                    "\tdh $@ --buildsystem cargo\n"
                )
            )?;
        } else {
            write!(
                rules,
                "{}{}",
                concat!(
                    "#!/usr/bin/make -f\n",
                    "%:\n",
                    "\tdh $@ --buildsystem cargo\n",
                    "\n",
                    "override_dh_auto_test:\n",
                ),
                // TODO: this logic is slightly brittle if another feature
                // "provides" the default feature. In this case, you need to
                // set test_is_broken explicitly on package."lib+default" and
                // not package."lib+theotherfeature".
                if test_is_broken_for("default") {
                    "\tdh_auto_test -- test --all || true\n"
                } else {
                    "\tdh_auto_test -- test --all\n"
                },
            )?;
        }

        // debian/control
        let build_deps = {
            let build_deps = ["debhelper (>= 11)", "dh-cargo (>= 18)"]
                .iter()
                .map(|x| x.to_string());
            let (default_features, default_deps) =
                crate_info.feature_all_deps(&features_with_deps, "default");
            //debcargo_info!("default_features: {:?}", default_features);
            //debcargo_info!("default_deps: {:?}", deb_deps(config, &default_deps)?);
            let extra_override_deps = package_field_for_feature(
                &|x| config.package_depends(x),
                PackageKey::feature("default"),
                &default_features,
            );
            let build_deps_extra = ["cargo:native", "rustc:native", "libstd-rust-dev"]
                .iter()
                .map(|s| s.to_string())
                .chain(deb_deps(config, &default_deps)?)
                .chain(extra_override_deps);
            if !bins.is_empty() {
                build_deps.chain(build_deps_extra).collect()
            } else {
                assert!(lib);
                build_deps
                    .chain(build_deps_extra.map(|d| deb_dep_add_nocheck(&d)))
                    .collect()
            }
        };
        let mut source = Source::new(
            base_pkgname,
            name_suffix,
            upstream_name,
            if let Some(ref home) = meta.homepage {
                home
            } else {
                ""
            },
            lib,
            uploaders.iter().map(|s| s.to_string()).collect(),
            build_deps,
        )?;

        // If source overrides are present update related parts.
        source.apply_overrides(config);

        let mut control = io::BufWriter::new(file("control")?);
        write!(control, "{}", source)?;

        // Summary and description generated from Cargo.toml
        let (summary, description) = crate_info.get_summary_description();
        let summary = if !config.summary.is_empty() {
            Some(config.summary.as_str())
        } else {
            if let Some(summary) = summary.as_ref() {
                if summary.len() > 80 {
                    writeln!(
                        control,
                        "\n{}",
                        concat!(
                            "# FIXME (packages.\"(name)\".section) debcargo ",
                            "auto-generated summaries are very long, consider overriding"
                        )
                    )?;
                }
            }
            summary.as_ref().map(String::as_str)
        };
        let description = if config.description.is_empty() {
            description.as_ref().map(String::as_str)
        } else {
            Some(config.description.as_str())
        };

        if lib {
            // debian/tests/control
            let mut testctl = io::BufWriter::new(file("tests/control")?);
            write!(
                testctl,
                "{}",
                PkgTest::new(
                    "@",
                    &crate_name,
                    &crate_version,
                    vec!["--all-features"],
                    &dev_depends,
                    if all_features_test_broken {
                        vec!["flaky"]
                    } else {
                        vec![]
                    },
                )?
            )?;

            let mut provides = crate_info.calculate_provides(&mut features_with_deps);
            //debcargo_info!("provides: {:?}", provides);
            let mut recommends = vec![];
            let mut suggests = vec![];
            for (&feature, features) in provides.iter() {
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
                let mut package = Package::new(
                    base_pkgname,
                    name_suffix,
                    &crate_info.version(),
                    upstream_name,
                    summary,
                    description,
                    if feature == "" { None } else { Some(feature) },
                    f_deps,
                    deb_deps(config, &o_deps)?,
                    f_provides.clone(),
                    if feature == "" {
                        recommends.clone()
                    } else {
                        vec![]
                    },
                    if feature == "" {
                        suggests.clone()
                    } else {
                        vec![]
                    },
                )?;

                let test_is_broken =
                    test_is_broken_for(feature) || f_provides.iter().any(|f| test_is_broken_for(f));

                // If any overrides present for this package it will be taken care.
                package.apply_overrides(config, PackageKey::feature(feature), f_provides);
                write!(control, "\n{}", package)?;

                let pkgtest = PkgTest::new(
                    package.name(),
                    &crate_name,
                    &crate_version,
                    if feature == "" {
                        vec!["--no-default-features"]
                    } else {
                        vec!["--features", feature]
                    },
                    &dev_depends,
                    if test_is_broken {
                        vec!["flaky"]
                    } else {
                        vec![]
                    },
                )?;
                write!(testctl, "\n{}", pkgtest)?;
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
                if !lib {
                    None
                } else {
                    Some("FIXME-(packages.\"(name)\".section)")
                },
                summary,
                description,
                match boilerplate {
                    Some(ref s) => s,
                    None => "",
                },
            );

            // Binary package overrides.
            bin_pkg.apply_overrides(config, PackageKey::Bin, vec![]);
            write!(control, "\n{}", bin_pkg)?;
        }

        // debian/changelog
        if !changelog_ready {
            let author = control::get_deb_author()?;
            let autogenerated_item = format!(
                "  * Package {} {} from crates.io using debcargo {}",
                &crate_name,
                &crate_version,
                pkgbase.debcargo_version()
            );
            let autogenerated_re =
                Regex::new(r"^  \* Package (.*) (.*) from crates.io using debcargo (.*)$").unwrap();

            // Special-case d/changelog:
            // - Always prepend to any existing file from the overlay.
            // - If the first entry is changelog::DEFAULT_DIST then write over that, smartly
            let (mut changelog, changelog_data) = changelog_or_new(tempdir.path())?;
            let (changelog_old, mut changelog_items, deb_version_suffix) =
                match ChangelogIterator::from(&changelog_data).next() {
                    Some(x) => {
                        if x.contains(changelog::DEFAULT_DIST) {
                            let mut e = ChangelogEntry::from_str(x)?;
                            let (ups, suf) = e.version_parts();
                            if author == e.maintainer {
                                if let Some(pos) =
                                    e.items.iter().position(|x| autogenerated_re.is_match(x))
                                {
                                    e.items[pos] = autogenerated_item;
                                } else {
                                    e.items.push(autogenerated_item);
                                }
                            } else {
                                // If unreleased changelog is by someone else, preserve their entries
                                e.items.insert(0, autogenerated_item);
                                e.items.insert(1, "".to_string());
                                let ename = e.maintainer_name();
                                e.items.insert(2, format!("  [ {} ]", ename));
                            }
                            (
                                &changelog_data[x.len()..],
                                e.items,
                                if ups == pkgbase.debian_version() {
                                    suf
                                } else {
                                    "1".to_string()
                                },
                            )
                        } else {
                            let e = ChangelogEntry::from_str(x)?;
                            let (ups, _suf) = e.version_parts();
                            (
                                changelog_data.as_str(),
                                vec![autogenerated_item],
                                if ups == pkgbase.debian_version() {
                                    e.deb_version_suffix_bump()
                                } else {
                                    "1".to_string()
                                },
                            )
                        }
                    }
                    None => (
                        changelog_data.as_str(),
                        vec![autogenerated_item],
                        "1".to_string(),
                    ),
                };

            let source_deb_version =
                format!("{}-{}", pkgbase.debian_version(), &deb_version_suffix);
            if !uploaders.contains(&author.as_str()) {
                debcargo_warn!(
                    "You ({}) are not in Uploaders; adding \"Team upload\" to d/changelog",
                    author
                );
                if !changelog_items.contains(&changelog::COMMENT_TEAM_UPLOAD.to_string()) {
                    changelog_items.insert(0, changelog::COMMENT_TEAM_UPLOAD.to_string());
                }
            }
            let changelog_new_entry = ChangelogEntry::new(
                source.srcname().to_string(),
                source_deb_version,
                changelog::DEFAULT_DIST.to_string(),
                "urgency=medium".to_string(),
                author,
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

    if overlay_write_back {
        if let Some(p) = overlay.as_ref() {
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
        }
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

fn changelog_first_last(tempdir: &Path) -> Result<(i32, i32)> {
    let mut changelog = fs::File::open(tempdir.join("changelog"))?;
    let mut changelog_data = String::new();
    changelog.read_to_string(&mut changelog_data)?;
    let mut last = None;
    let mut first = None;
    for x in ChangelogIterator::from(&changelog_data) {
        let e = ChangelogEntry::from_str(x)?;
        if None == last {
            last = Some(e.date.year());
        }
        first = Some(e.date.year());
    }
    if None == last {
        Err(format_err!("changelog had no entries"))
    } else {
        Ok((first.unwrap(), last.unwrap()))
    }
}
