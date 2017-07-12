#[macro_use]
extern crate debcargo;
extern crate cargo;
#[macro_use]
extern crate clap;
extern crate chrono;
extern crate flate2;
extern crate itertools;
extern crate semver;
extern crate semver_parser;
extern crate tar;
extern crate tempdir;
extern crate ansi_term;
extern crate walkdir;
extern crate regex;

use clap::{App, AppSettings, ArgMatches, SubCommand};
use std::fs;
use std::path::Path;
use std::io::{BufReader, BufRead};


use debcargo::errors::*;
use debcargo::crates::CrateInfo;
use debcargo::debian::{self, BaseInfo};
use debcargo::overrides::parse_overrides;


fn lookup_fixmes(srcdir: &Path) -> Result<Vec<String>> {
    let fixme_regex = regex::Regex::new(r"(?:FIXME)")?;
    let mut fixme_files = Vec::new();
    for entry in walkdir::WalkDir::new(srcdir) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let filename = entry.path().to_str().unwrap();
            let file = fs::File::open(entry.path())?;
            let reader = BufReader::new(file);
            // If we find one FIXME we break the loop and check next file. Idea
            // is only to find files with FIXME strings in it.
            for line in reader.lines() {
                if let Ok(line) = line {
                    if fixme_regex.is_match(&line) {
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
    let crate_name_dashed = crate_name.replace('_', "-");
    let version = matches.value_of("version");
    let package_lib_binaries = matches.is_present("bin") || matches.is_present("bin-name");
    let bin_name = matches.value_of("bin-name").unwrap_or(&crate_name_dashed);
    let distribution = matches.value_of("distribution").unwrap_or("unstable");
    let override_file = matches.value_of("override").unwrap_or("");
    let override_path = Path::new(override_file);

    let overrides = match parse_overrides(&override_path) {
        Ok(o) => Some(o),
        Err(_) => None,
    };


    let crate_info = CrateInfo::new(crate_name, version)?;
    let pkgbase = BaseInfo::new(crate_name, &crate_info, crate_version!());


    let pkg_srcdir = pkgbase.package_source_dir();
    let orig_tar_gz = pkgbase.orig_tarball_path();

    let source_modified = crate_info.extract_crate(pkg_srcdir)?;
    debian::prepare_orig_tarball(crate_info.crate_file(),
                                 orig_tar_gz,
                                 source_modified)?;
    debian::prepare_debian_folder(&pkgbase,
                                  &crate_info,
                                  package_lib_binaries,
                                  bin_name,
                                  distribution,
                                  overrides)?;


    debcargo_info!(concat!("Package Source: {}\n", "Original Tarball for package: {}\n"),
                   pkg_srcdir.to_str().unwrap(),
                   orig_tar_gz.to_str().unwrap());
    let fixmes = lookup_fixmes(pkg_srcdir.join("debian").as_path());
    if let Ok(fixmes) = fixmes {
        if fixmes.len() > 0 {
            debcargo_warn!("Please update the sections marked FIXME in following files.");
            for f in fixmes {
                debcargo_warn!(format!("\t• {}", f));
            }
        }
    }

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
                              .arg_from_usage("--bin 'Package binaries from library crates'")
                              .arg_from_usage("--bin-name [name] 'Set package name for \
                                               binaries (implies --bin)'")
                              .arg_from_usage("--distribution [name] 'Set target distribution \
                                               for package (default: unstable)'")
                              .arg_from_usage("--override [files] 'TOML file providing \
                                               override values for debcargo")])
        .get_matches();
    match m.subcommand() {
        ("package", Some(sm)) => do_package(sm),
        _ => unreachable!(),
    }
}

fn main() {
    if let Err(e) = real_main() {
        println!("{}", e);
        std::process::exit(1);
    }
}
