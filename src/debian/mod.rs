pub use self::dependency::deb_dep;
pub use self::dependency::deb_deps;

use std::fs;
use std::io::{self, ErrorKind, Read, Seek, Write as IoWrite};
use std::path::{Path, PathBuf};
use std::os::unix::fs::PermissionsExt;

use cargo::util::FileLock;
use glob::Pattern;
use tempdir::TempDir;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use tar::{Archive, Builder};

use crates::CrateInfo;
use errors::*;
use config::{Config, OverrideDefaults};
use util::{self, copy_tree};

use self::control::deb_version;
use self::control::{Package, Source};
use self::copyright::debian_copyright;
use self::changelog::{Changelog, ChangelogIterator};

pub mod control;
mod dependency;
pub mod copyright;
pub mod changelog;

pub struct BaseInfo {
    upstream_name: String,
    base_package_name: String,
    package_source: PathBuf,
    debian_version: String,
    original_source_archive: PathBuf,
    debcargo_version: String,
}

impl BaseInfo {
    pub fn new(name: &str, crate_info: &CrateInfo, debcargo_version: &str) -> Self {
        let upstream = name.to_string();
        let name_dashed = upstream.replace('_', "-");
        let base_pkg_name = format!("{}{}", name_dashed.to_lowercase(), crate_info.version_suffix());

        let debian_source = format!("rust-{}", base_pkg_name);
        let debver = deb_version(crate_info.version());

        let srcdir = Path::new(&format!("{}-{}", debian_source, debver)).to_owned();
        let orig_tar_gz =
            Path::new(&format!("{}_{}.orig.tar.gz", debian_source, debver)).to_owned();

        BaseInfo {
            upstream_name: upstream,
            base_package_name: base_pkg_name,
            debian_version: debver,
            original_source_archive: orig_tar_gz,
            package_source: srcdir,
            debcargo_version: debcargo_version.to_string(),
        }
    }

    pub fn upstream_name(&self) -> &str {
        self.upstream_name.as_str()
    }

    pub fn package_source_dir(&self) -> &Path {
        self.package_source.as_path()
    }

    pub fn orig_tarball_path(&self) -> &Path {
        self.original_source_archive.as_path()
    }

    pub fn package_basename(&self) -> &str {
        self.base_package_name.as_str()
    }

    pub fn debian_version(&self) -> &str {
        self.debian_version.as_str()
    }

    pub fn debcargo_version(&self) -> &str {
        self.debcargo_version.as_str()
    }
}

pub fn prepare_orig_tarball(
    crate_file: &FileLock,
    tarball: &Path,
    src_modified: bool,
    pkg_srcdir: &Path,
    excludes: Option<&Vec<Pattern>>,
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

    if lib && !bins.is_empty() && !config.bin {
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
            (f, (ff, deb_deps(dd).unwrap()))
        }).collect::<Vec<_>>());*/
    let default_deps = crate_info.feature_all_deps(&features_with_deps, "default");
    //debcargo_info!("default_deps: {:?}", deb_deps(&default_deps)?);
    let provides = crate_info.calculate_provides(&mut features_with_deps);
    //debcargo_info!("provides: {:?}", provides);

    let mut create = fs::OpenOptions::new();
    create.write(true).create_new(true);

    let tempdir = TempDir::new_in(".", "debcargo")?;
    let base_pkgname = pkgbase.package_basename();
    let upstream_name = pkgbase.upstream_name();

    let overlay = config
        .overlay
        .as_ref()
        .map(|p| config_path.unwrap().parent().unwrap().join(p));

    overlay.as_ref().map(|p| {
        copy_tree(p.as_path(), tempdir.path()).unwrap();
    });

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
        writeln!(compat, "10")?;

        // debian/copyright
        let mut copyright = io::BufWriter::new(file("copyright")?);
        let dep5_copyright = debian_copyright(
            crate_info.package(),
            pkg_srcdir,
            crate_info.manifest(),
            copyright_guess_harder,
        )?;
        writeln!(copyright, "{}", dep5_copyright)?;

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
        let mut source = Source::new(
            upstream_name,
            base_pkgname,
            pkgbase.debian_version(),
            if let Some(ref home) = meta.homepage { home } else { "" },
            lib,
            &default_deps,
        )?;

        // If source overrides are present update related parts.
        source.apply_overrides(config);

        let mut control = io::BufWriter::new(file("control")?);
        write!(control, "{}", source)?;

        // Summary and description generated from Cargo.toml
        let (summary, description) = crate_info.get_summary_description();

        if lib {
            for (&feature, &(ref f_deps, ref o_deps)) in features_with_deps.iter() {
                let mut feature_package =
                    Package::new(base_pkgname, upstream_name, crate_info,
                        if feature == "" { None } else { Some(feature) },
                        f_deps, o_deps, provides.get(feature).unwrap_or(&vec![]))?;

                // If any overrides present for this package it will be taken care.
                feature_package.apply_overrides(config);
                writeln!(control, "{}", feature_package)?;
            }
        }

        if !bins.is_empty() {
            let boilerplate = if bins.len() > 1 || bins[0] != bin_name {
                Some(format!(
                    "This package contains the following binaries built
        from \
                              the\nRust \"{}\" crate:\n- {}",
                    upstream_name,
                    bins.join("\n- ")
                ))
            } else {
                None
            };

            let mut bin_pkg = Package::new_bin(
                upstream_name,
                bin_name,
                &summary,
                &description,
                match boilerplate {
                    Some(ref s) => s,
                    None => "",
                },
            );

            // Binary package overrides.
            bin_pkg.apply_overrides(config);
            writeln!(control, "{}", bin_pkg)?;
        }

        // debian/changelog
        let entries = vec![
            format!(
                "Package {} {} from crates.io using debcargo {}",
                crate_info.package_id().name(),
                crate_info.package_id().version(),
                pkgbase.debcargo_version()
            ),
        ];

        let changelog_entries = Changelog::new(
            source.srcname(),
            source.version(),
            changelog::DEFAULT_DIST,
            "medium",
            source.uploader(),
            entries.as_slice(),
        );

        if !changelog_ready {
            // Special-case d/changelog:
            // - Always prepend to any existing file from the overlay.
            // - If the first entry is changelog::DEFAULT_DIST then write over that.
            let mut changelog = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(tempdir.path().join("changelog"))?;
            let mut changelog_data = String::new();
            changelog.read_to_string(&mut changelog_data)?;
            let changelog_old = match ChangelogIterator::from(&changelog_data).next() {
                Some(x) => if x.contains(changelog::DEFAULT_DIST) {
                    &changelog_data[x.len()..]
                } else {
                    &changelog_data
                },
                None => &changelog_data,
            };
            changelog.seek(io::SeekFrom::Start(0))?;
            write!(changelog, "{}{}", changelog_entries, changelog_old)?;
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
