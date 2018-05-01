extern crate ansi_term;
extern crate cargo;
extern crate chrono;
#[macro_use]
extern crate clap;
#[macro_use]
extern crate debcargo;
extern crate flate2;
extern crate itertools;
extern crate semver;
extern crate semver_parser;
extern crate tar;
extern crate tempdir;
extern crate walkdir;

use clap::{App, AppSettings, ArgMatches, SubCommand};
use std::fs;
use std::path::Path;
use std::io::{BufRead, BufReader};

use debcargo::errors::*;
use debcargo::crates::CrateInfo;
use debcargo::debian::{self, BaseInfo};
use debcargo::config::{parse_config, Config};
use debcargo::util;

fn lookup_fixmes(srcdir: &Path) -> Result<Vec<String>> {
    let mut fixme_files = Vec::new();
    for entry in walkdir::WalkDir::new(srcdir) {
        let entry = entry?;
        if entry.file_type().is_file() && !util::is_hint_file(entry.file_name()) {
            let filename = entry.path().to_str().unwrap();
            let file = fs::File::open(entry.path())?;
            let reader = BufReader::new(file);
            // If we find one FIXME we break the loop and check next file. Idea
            // is only to find files with FIXME strings in it.
            for line in reader.lines() {
                if let Ok(line) = line {
                    if line.contains("FIXME") {
                        fixme_files.push(filename.to_string());
                        break;
                    }
                }
            }
        }
    }

    Ok(fixme_files)
}

fn do_package(matches: &ArgMatches) -> Result<()> {
    let crate_name = matches.value_of("crate").unwrap();
    let version = matches.value_of("version");
    let directory = matches.value_of("directory");
    let (config_path, config) = matches
        .value_of("config")
        .map(|p| {
            debcargo_warn!("--config is not yet stable, follow the mailing list for changes.");
            let path = Path::new(p);
            (Some(path), parse_config(path).unwrap())
        })
        .unwrap_or((None, Config::default()));
    let changelog_ready = matches.is_present("changelog-ready");
    let copyright_guess_harder = matches.is_present("copyright-guess-harder");

    let crate_info = CrateInfo::new(crate_name, version)?;
    let pkgbase = BaseInfo::new(crate_name, &crate_info, crate_version!());

    let pkg_srcdir = directory
        .map(|s| Path::new(s))
        .unwrap_or(pkgbase.package_source_dir());
    let orig_tar_gz = pkgbase.orig_tarball_path();

    let source_modified = crate_info.extract_crate(pkg_srcdir)?;
    debian::prepare_orig_tarball(
        crate_info.crate_file(),
        orig_tar_gz,
        source_modified,
        pkg_srcdir,
    )?;
    debian::prepare_debian_folder(
        &pkgbase,
        &crate_info,
        pkg_srcdir,
        config_path,
        &config,
        changelog_ready,
        copyright_guess_harder,
    )?;

    debcargo_info!(
        concat!("Package Source: {}\n", "Original Tarball for package: {}\n"),
        pkg_srcdir.to_str().unwrap(),
        orig_tar_gz.to_str().unwrap()
    );
    let fixmes = lookup_fixmes(pkg_srcdir.join("debian").as_path());
    if let Ok(fixmes) = fixmes {
        if !fixmes.is_empty() {
            debcargo_warn!("FIXME found in the following files.");
            debcargo_warn!("Either supply an overlay or add extra overrides to your config file.");
            for f in fixmes {
                debcargo_warn!(format!("\t• {}", f));
            }
        }
    }

    Ok(())
}

fn do_deb_src_name(matches: &ArgMatches) -> Result<()> {
    let crate_name = matches.value_of("crate").unwrap();
    let version = matches.value_of("version");

    let crate_info = CrateInfo::new(crate_name, version)?;
    let pkgbase = BaseInfo::new(crate_name, &crate_info, crate_version!());
    println!("{}", pkgbase.package_basename());

    Ok(())
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
                              .arg_from_usage("--config [file] 'TOML file providing additional \
                                               package-specific options.'")
                     ])
        .subcommands(vec![SubCommand::with_name("deb-src-name")
                              .about("Prints the Debian package name for a crate")
                              .arg_from_usage("<crate> 'Name of the crate to package'")
                              .arg_from_usage("[version] 'Version of the crate to package; may \
                                               include dependency operators'")
                     ])
        .get_matches();
    match m.subcommand() {
        ("package", Some(sm)) => do_package(sm),
        ("deb-src-name", Some(sm)) => do_deb_src_name(sm),
        _ => unreachable!(),
    }
}

fn main() {
    if let Err(e) = real_main() {
        println!("Something failed: {:?}", e);
        std::process::exit(1);
    }
}
