use std::env;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use ansi_term::Colour::Red;
use anyhow::Context;
use structopt::{
    clap::{crate_version, AppSettings},
    StructOpt,
};

use debcargo::config::{parse_config, Config};
use debcargo::crates::{update_crates_io, CrateInfo};
use debcargo::debian::{
    self,
    build_order::{build_order, ResolveType},
    DebInfo,
};
use debcargo::errors::Result;
use debcargo::util;
use debcargo::{debcargo_info, debcargo_warn};

fn lookup_fixmes(srcdir: &Path) -> Result<Vec<PathBuf>> {
    let mut fixme_files = Vec::new();
    for entry in walkdir::WalkDir::new(srcdir) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let file = fs::File::open(entry.path())?;
            let reader = BufReader::new(file);
            // If we find one FIXME we break the loop and check next file. Idea
            // is only to find files with FIXME strings in it.
            for line in reader.lines().flatten() {
                if line.contains("FIXME") {
                    fixme_files.push(entry.path().to_path_buf());
                    break;
                }
            }
        }
    }

    Ok(fixme_files)
}

fn rel_p<'a>(path: &'a Path, base: &'a Path) -> &'a str {
    path.strip_prefix(base).unwrap_or(path).to_str().unwrap()
}

fn do_package(
    crate_name: &str,
    version: Option<&str>,
    directory: Option<&str>,
    changelog_ready: bool,
    copyright_guess_harder: bool,
    no_overlay_write_back: bool,
    config: Option<&str>,
) -> Result<()> {
    let (config_path, config) = match config {
        Some(p) => {
            let path = Path::new(p);
            let config = parse_config(path).context("failed to parse debcargo.toml")?;
            (Some(path), config)
        }
        None => (None, Config::default()),
    };
    let overlay_write_back = !no_overlay_write_back;
    let crate_path = config.crate_src_path(config_path);

    log::info!("preparing crate info");
    let mut crate_info = match crate_path {
        Some(p) => CrateInfo::new_with_local_crate(crate_name, version, &p)?,
        None => CrateInfo::new(crate_name, version)?,
    };
    crate_info.set_includes_excludes(config.orig_tar_excludes(), config.orig_tar_whitelist());
    let deb_info = DebInfo::new(
        crate_name,
        &crate_info,
        crate_version!(),
        config.semver_suffix,
    );
    let pkg_srcdir = Path::new(directory.unwrap_or_else(|| deb_info.package_source_dir()));
    log::info!("extracting crate");
    let source_modified = crate_info.extract_crate(pkg_srcdir)?;

    let orig_tar_gz = pkg_srcdir
        .parent()
        .unwrap()
        .join(deb_info.orig_tarball_path());
    log::info!("preparing orig tarball");
    debian::prepare_orig_tarball(&crate_info, &orig_tar_gz, source_modified, pkg_srcdir)?;
    log::info!("preparing debian folder");
    debian::prepare_debian_folder(
        &deb_info,
        &mut crate_info,
        pkg_srcdir,
        config_path,
        &config,
        changelog_ready,
        copyright_guess_harder,
        overlay_write_back,
    )?;

    let curdir = env::current_dir()?;
    debcargo_info!(
        concat!("Package Source: {}\n", "Original Tarball for package: {}\n"),
        rel_p(pkg_srcdir, &curdir),
        rel_p(&orig_tar_gz, &curdir)
    );
    let fixmes = lookup_fixmes(pkg_srcdir.join("debian").as_path());
    if let Ok(fixmes) = fixmes {
        if !fixmes.is_empty() {
            debcargo_warn!("FIXME found in the following files.");
            for f in fixmes {
                if util::is_hint_file(&f) {
                    debcargo_warn!("\t(•) {}", rel_p(&f, &curdir));
                } else {
                    debcargo_warn!("\t •  {}", rel_p(&f, &curdir));
                }
            }
            debcargo_warn!("");
            debcargo_warn!("To fix, try combinations of the following: ");
            match config_path {
                None => debcargo_warn!("\t •  Write a config file and use it with --config"),
                Some(c) => {
                    debcargo_warn!("\t •  Add or edit overrides in your config file:");
                    debcargo_warn!("\t    {}", rel_p(c, &curdir));
                }
            };
            match config.overlay_dir(config_path) {
                None => debcargo_warn!("\t •  Create an overlay directory and add it to your config file with overlay = \"/path/to/overlay\""),
                Some(p) => {
                    debcargo_warn!("\t •  Add or edit files in your overlay directory:");
                    debcargo_warn!("\t    {}", rel_p(&p, &curdir));
                }
            }
        }
    }

    Ok(())
}

fn do_deb_src_name(crate_name: &str, version: Option<&str>) -> Result<()> {
    let crate_info = CrateInfo::new_with_update(crate_name, version, false)?;
    let deb_info = DebInfo::new(crate_name, &crate_info, crate_version!(), version.is_some());
    println!("{}", deb_info.package_name());
    Ok(())
}

fn do_extract(crate_name: &str, version: Option<&str>, directory: Option<&str>) -> Result<()> {
    let crate_info = CrateInfo::new(crate_name, version)?;
    let deb_info = DebInfo::new(crate_name, &crate_info, crate_version!(), false);
    let pkg_srcdir = Path::new(directory.unwrap_or_else(|| deb_info.package_source_dir()));
    crate_info.extract_crate(pkg_srcdir)?;
    Ok(())
}

fn do_build_order(
    crate_name: &str,
    version: Option<&str>,
    resolve_type: ResolveType,
    emulate_collapse_features: bool,
) -> Result<()> {
    let build_order = build_order(crate_name, version, resolve_type, emulate_collapse_features)?;
    for v in &build_order {
        println!("{}", v);
    }
    Ok(())
}

fn do_update() -> Result<()> {
    update_crates_io()
}

#[derive(Debug, StructOpt)]
#[structopt(name = "debcargo", about = "Package Rust crates for Debian.")]
enum Opt {
    /// Package a Rust crate for Debian.
    Package {
        /// Name of the crate to package.
        crate_name: String,
        /// Version of the crate to package; may contain dependency operators.
        version: Option<String>,
        /// Output directory.
        #[structopt(long)]
        directory: Option<String>,
        /// Assume the changelog is already bumped, and leave it alone.
        #[structopt(long)]
        changelog_ready: bool,
        /// Guess extra values for d/copyright. Might be slow.
        #[structopt(long)]
        copyright_guess_harder: bool,
        /// Don't write back hint files or d/changelog to the source overlay directory.
        #[structopt(long)]
        no_overlay_write_back: bool,
        /// TOML file providing additional package-specific options.
        #[structopt(long)]
        config: Option<String>,
    },
    /// Print the Debian package name for a crate.
    DebSrcName {
        /// Name of the crate to package.
        crate_name: String,
        /// Version of the crate to package; may contain dependency operators.
        version: Option<String>,
    },
    /// Extract only a crate, without any other transformations.
    Extract {
        /// Name of the crate to package.
        crate_name: String,
        /// Version of the crate to package; may contain dependency operators.
        version: Option<String>,
        /// Output directory.
        #[structopt(long)]
        directory: Option<String>,
    },
    /// Print the transitive dependencies of a package in topological order.
    BuildOrder {
        /// Name of the crate to package.
        crate_name: String,
        /// Version of the crate to package; may contain dependency operators.
        version: Option<String>,
        /// Resolution type, one of BinaryForDebianUnstable | SourceForDebianTesting
        #[structopt(long, default_value = "BinaryForDebianUnstable")]
        resolve_type: ResolveType,
        /// Emulate resolution as if every package were built with --collapse-features.
        #[structopt(long)]
        emulate_collapse_features: bool,
    },
    /// Update the user's default crates.io index, outside of a workspace.
    Update,
}

fn real_main() -> Result<()> {
    let m = Opt::clap()
        .global_setting(AppSettings::ColoredHelp)
        .get_matches();
    use Opt::*;
    match Opt::from_clap(&m) {
        Package {
            crate_name,
            version,
            directory,
            changelog_ready,
            copyright_guess_harder,
            no_overlay_write_back,
            config,
        } => do_package(
            &crate_name,
            version.as_deref(),
            directory.as_deref(),
            changelog_ready,
            copyright_guess_harder,
            no_overlay_write_back,
            config.as_deref(),
        ),
        DebSrcName {
            crate_name,
            version,
        } => do_deb_src_name(&crate_name, version.as_deref()),
        Extract {
            crate_name,
            version,
            directory,
        } => do_extract(&crate_name, version.as_deref(), directory.as_deref()),
        BuildOrder {
            crate_name,
            version,
            resolve_type,
            emulate_collapse_features,
        } => do_build_order(
            &crate_name,
            version.as_deref(),
            resolve_type,
            emulate_collapse_features,
        ),
        Update => do_update(),
    }
}

fn main() {
    env_logger::init();
    if let Err(e) = real_main() {
        eprintln!("{}", Red.bold().paint(format!("debcargo failed: {:?}", e)));
        std::process::exit(1);
    }
}
