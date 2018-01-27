pub use self::dependency::deb_dep;

use std::fs;
use std::io::{self, Seek, Write as IoWrite};
use std::path::{Path, PathBuf};
use std::os::unix::fs::OpenOptionsExt;

use cargo::util::FileLock;
use tempdir::TempDir;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use tar::{Archive, Builder};


use crates::CrateInfo;
use errors::*;
use overrides::{Overrides, OverrideDefaults};

use self::control::deb_version;
use self::control::{Source, Package};
use self::copyright::debian_copyright;
use self::changelog::Changelog;

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
        let base_pkg_name = format!("{}{}", name_dashed, crate_info.version_suffix());

        let debian_source = format!("rust-{}", base_pkg_name);
        let debver = deb_version(crate_info.version());

        let srcdir = Path::new(&format!("{}-{}", debian_source, debver)).to_owned();
        let orig_tar_gz = Path::new(&format!("{}_{}.orig.tar.gz", debian_source, debver))
            .to_owned();

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
) -> Result<()> {
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
        f.seek(io::SeekFrom::Start(0))?;
        let mut archive = Archive::new(GzDecoder::new(f));
        let mut new_archive =
            Builder::new(GzEncoder::new(create.open(&tarball)?, Compression::best()));
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

pub fn prepare_debian_folder(
    pkgbase: &BaseInfo,
    crate_info: &CrateInfo,
    pkg_lib_binaries: bool,
    bin_name: &str,
    pkg_srcdir: &Path,
    distribution: &str,
    overrides: Option<&Overrides>,
    copyright_guess_harder: bool,
) -> Result<()> {
    let lib = crate_info.is_lib();
    let mut bins = crate_info.get_binary_targets();

    let meta = crate_info.metadata();

    let (default_features, _) = crate_info.default_deps_features();
    let non_default_features = crate_info.non_default_features(&default_features);
    let deps = crate_info.non_dev_dependencies()?;

    let build_deps = if !bins.is_empty() {
        deps.iter()
    } else {
        [].iter()
    };

    if lib && !bins.is_empty() && !pkg_lib_binaries {
        debcargo_warn!(
            "Ignoring binaries from lib crate; pass --bin to package: {}",
            bins.join(", ")
        );
        bins.clear();
    }

    let mut create = fs::OpenOptions::new();
    create.write(true).create_new(true);

    let tempdir = TempDir::new_in(".", "debcargo")?;
    let base_pkgname = pkgbase.package_basename();
    let upstream_name = pkgbase.upstream_name();


    {
        let file = |name: &str| create.open(tempdir.path().join(name));

        // debian/cargo-checksum.json
        let checksum = crate_info.checksum().unwrap_or(
            "Could not get crate checksum",
        );
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
            overrides,
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
        let mut create_exec = create.clone();
        create_exec.mode(0o777);
        let mut rules = create_exec.open(tempdir.path().join("rules"))?;
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
            if let Some(ref home) = meta.homepage {
                home
            } else {
                ""
            },
            &lib,
            build_deps.as_slice(),
        )?;

        // If source overrides are present update related parts.
        if let Some(overrides) = overrides {
            source.apply_overrides(overrides);
        }

        let mut control = io::BufWriter::new(file("control")?);
        write!(control, "{}", source)?;

        // Summary and description generated from Cargo.toml
        let (summary, description) = crate_info.get_summary_description();

        if lib {
            let mut lib_package = Package::new(base_pkgname, upstream_name, crate_info, None)?;

            // Apply overrides if any
            if let Some(overrides) = overrides {
                lib_package.apply_overrides(overrides);
            }
            writeln!(control, "{}", lib_package)?;

            for feature in non_default_features {
                let mut feature_package =
                    Package::new(base_pkgname, upstream_name, &crate_info, Some(feature))?;

                // If any overrides present for this package it will be taken care.
                if let Some(overrides) = overrides {
                    feature_package.apply_overrides(overrides);
                }
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
            if let Some(overrides) = overrides {
                bin_pkg.apply_overrides(overrides);
            }

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
            distribution,
            "medium",
            source.uploader(),
            entries.as_slice(),
        );

        let mut changelog = try!(file("changelog"));
        write!(changelog, "{}", changelog_entries)?;

        fs::rename(tempdir.path(), pkg_srcdir.join("debian"))?;
        Ok(())
    }
}
