use std::env;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use ansi_term::Colour::Red;
use clap::{crate_authors, crate_version, App, AppSettings, ArgMatches, SubCommand};
use failure::ResultExt;

use debcargo::config::{parse_config, Config};
use debcargo::crates::{update_crates_io, CrateInfo};
use debcargo::debian::{self, BaseInfo};
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
            for line in reader.lines() {
                if let Ok(line) = line {
                    if line.contains("FIXME") {
                        fixme_files.push(entry.path().to_path_buf());
                        break;
                    }
                }
            }
        }
    }

    Ok(fixme_files)
}

fn rel_p<'a>(path: &'a Path, base: &'a Path) -> &'a str {
    path.strip_prefix(base).unwrap_or(path).to_str().unwrap()
}

fn do_package(matches: &ArgMatches) -> Result<()> {
    let crate_name = matches.value_of("crate").unwrap();
    let version = matches.value_of("version");
    let directory = matches.value_of("directory");
    let (config_path, config) = match matches.value_of("config") {
        Some(p) => {
            let path = Path::new(p);
            let config = parse_config(path).context("failed to parse debcargo.toml")?;
            (Some(path), config)
        }
        None => (None, Config::default()),
    };
    let changelog_ready = matches.is_present("changelog-ready");
    let overlay_write_back = !matches.is_present("no-overlay-write-back");
    let copyright_guess_harder = matches.is_present("copyright-guess-harder");

    let crate_path = config.crate_src_path(config_path);

    let mut crate_info = match crate_path {
        Some(p) => CrateInfo::new_with_local_crate(crate_name, version, &p)?,
        None => CrateInfo::new(crate_name, version)?,
    };
    let pkgbase = BaseInfo::new(
        crate_name,
        &crate_info,
        crate_version!(),
        config.semver_suffix,
    );

    let pkg_srcdir = Path::new(directory.unwrap_or_else(|| pkgbase.package_source_dir()));
    let orig_tar_gz = pkg_srcdir
        .parent()
        .unwrap()
        .join(pkgbase.orig_tarball_path());
    crate_info.set_includes_excludes(config.orig_tar_excludes(), config.orig_tar_whitelist());
    let source_modified = crate_info.extract_crate(pkg_srcdir)?;
    debian::prepare_orig_tarball(&crate_info, &orig_tar_gz, source_modified, pkg_srcdir)?;
    debian::prepare_debian_folder(
        &pkgbase,
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
                    debcargo_warn!("\t    {}", rel_p(&c, &curdir));
                }
            };
            match config.overlay {
                None => debcargo_warn!("\t •  Create an overlay directory and add it to your config file with overlay = \"/path/to/overlay\""),
                Some(_) => {
                    debcargo_warn!("\t •  Add or edit files in your overlay directory:");
                    debcargo_warn!("\t    {}", rel_p(&config.overlay_dir(config_path).unwrap(), &curdir));
                }
            }
        }
    }

    Ok(())
}

fn do_deb_src_name(matches: &ArgMatches) -> Result<()> {
    let crate_name = matches.value_of("crate").unwrap();
    let version = matches.value_of("version");

    let crate_info = CrateInfo::new_with_update(crate_name, version, false)?;
    let pkgbase = BaseInfo::new(crate_name, &crate_info, crate_version!(), version.is_some());

    println!("{}", pkgbase.package_name());
    Ok(())
}

fn do_extract(matches: &ArgMatches) -> Result<()> {
    let crate_name = matches.value_of("crate").unwrap();
    let version = matches.value_of("version");
    let directory = matches.value_of("directory");

    let crate_info = CrateInfo::new(crate_name, version)?;
    let pkgbase = BaseInfo::new(crate_name, &crate_info, crate_version!(), false);
    let pkg_srcdir = Path::new(directory.unwrap_or_else(|| pkgbase.package_source_dir()));

    crate_info.extract_crate(pkg_srcdir)?;
    Ok(())
}

fn do_update() -> Result<()> {
    update_crates_io()
}

fn real_main() -> Result<()> {
    let m = App::new("debcargo")
        .author(crate_authors!())
        .version(crate_version!())
        .global_setting(AppSettings::ColoredHelp)
        .global_setting(AppSettings::UnifiedHelpMessage)
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .subcommands(vec![SubCommand::with_name("package")
                              .about("Package a crate from crates.io")
                              .arg_from_usage("<crate> 'Name of the crate to package'")
                              .arg_from_usage("[version] 'Version of the crate to package; may \
                                               include dependency operators'")
                              .arg_from_usage("--directory [directory] 'Output directory.'")
                              .arg_from_usage("--changelog-ready 'Assume the changelog is already bumped, and leave it alone.'")
                              .arg_from_usage("--copyright-guess-harder 'Guess extra values for d/copyright. Might be slow.'")
                              .arg_from_usage("--no-overlay-write-back 'Don\'t write back hint files or d/changelog to the source overlay directory.'")
                              .arg_from_usage("--config [file] 'TOML file providing additional \
                                               package-specific options.'")
                     ])
        .subcommands(vec![SubCommand::with_name("deb-src-name")
                              .about("Prints the Debian package name for a crate")
                              .arg_from_usage("<crate> 'Name of the crate to package'")
                              .arg_from_usage("[version] 'Version of the crate to package; may \
                                               include dependency operators'")
                     ])
        .subcommands(vec![SubCommand::with_name("extract")
                              .about("Extract only a crate, without any other transformations.")
                              .arg_from_usage("<crate> 'Name of the crate to package'")
                              .arg_from_usage("[version] 'Version of the crate to package; may \
                                               include dependency operators'")
                              .arg_from_usage("--directory [directory] 'Output directory.'")
                     ])
        .subcommands(vec![SubCommand::with_name("update")
                              .about("Update the crates.io index, outside of a workspace.")
                     ])
        .get_matches();
    match m.subcommand() {
        ("package", Some(sm)) => do_package(sm),
        ("deb-src-name", Some(sm)) => do_deb_src_name(sm),
        ("extract", Some(sm)) => do_extract(sm),
        ("update", Some(_)) => do_update(),
        _ => unreachable!(),
    }
}

fn main() {
    if let Err(e) = real_main() {
        eprintln!("{}", Red.bold().paint(format!("Something failed: {:?}", e)));
        std::process::exit(1);
    }
}
