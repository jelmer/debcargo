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
extern crate termcolor;

use cargo::core::Source;
use clap::{App, AppSettings, ArgMatches, SubCommand};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};
use std::fs;
use std::io::{self, Write as IoWrite};
use std::os::unix::fs::OpenOptionsExt;

use debcargo::errors::*;
use debcargo::copyright;
use debcargo::crates::CrateInfo;
use debcargo::debian::{self, PkgBase, Source as ControlSource, Package as ControlPackage};
use debcargo::debian::deb_feature_name;


fn do_package(matches: &ArgMatches) -> Result<()> {
    let crate_name = matches.value_of("crate").unwrap();
    let crate_name_dashed = crate_name.replace('_', "-");
    let version = matches.value_of("version");
    let package_lib_binaries = matches.is_present("bin") || matches.is_present("bin-name");
    let bin_name = matches.value_of("bin-name").unwrap_or(&crate_name_dashed);
    let distribution = matches.value_of("distribution").unwrap_or("unstable");


    let crate_info = CrateInfo::new(crate_name, version)?;
    let pkgid = crate_info.package_id();
    let checksum = crate_info.checksum().ok_or("Could not get crate checksum")?;

    let package = crate_info.package();
    let meta = crate_info.metadata();

    let lib = crate_info.is_lib();
    let mut bins = crate_info.get_binary_targets();

    let (default_features, _) = crate_info.default_deps_features().unwrap();
    let non_default_features = crate_info.non_default_features(&default_features).unwrap();
    let deps = crate_info.non_dev_dependencies()?;

    let build_deps = if !bins.is_empty() {
        deps.iter()
    } else {
        [].iter()
    };


    if lib && !bins.is_empty() && !package_lib_binaries {
        debcargo_info!("Ignoring binaries from lib crate; pass --bin to package: {}",
                 bins.join(", "));
        bins.clear();
    }

    let version_suffix = crate_info.version_suffix();
    let pkgbase = PkgBase::new(crate_name, &version_suffix, pkgid.version())?;
    let source_section = ControlSource::new(&pkgbase,
                                            if let Some(ref home) = meta.homepage {
                                                home.as_str()
                                            } else {
                                                ""
                                            },
                                            &lib,
                                            &build_deps.as_slice())?;


    let mut create = fs::OpenOptions::new();
    create.write(true).create_new(true);
    let mut create_exec = create.clone();
    create_exec.mode(0o777);

    let source_modified = crate_info.extract_crate(pkgbase.srcdir.as_path())?;
    debian::prepare_orig_tarball(crate_info.crate_file(),
                                 pkgbase.orig_tar_gz.as_path(),
                                 source_modified)?;
    let tempdir = tempdir::TempDir::new_in(".", "debcargo")?;

    let deb_feature = &|f: &str| deb_feature_name(&pkgbase.crate_pkg_base, f);


    {
        let file = |name| create.open(tempdir.path().join(name));

        let mut cargo_checksum_json = try!(file("cargo-checksum.json"));
        try!(writeln!(cargo_checksum_json,
                      r#"{{"package":"{}","files":{{}}}}"#,
                      checksum));

        let mut changelog = try!(file("changelog"));
        write!(changelog,
               "{}",
               source_section.changelog_entry(pkgid.name(),
                                              pkgid.version(),
                                              distribution,
                                              crate_version!()))?;

        let mut compat = try!(file("compat"));
        try!(writeln!(compat, "10"));


        let mut control = io::BufWriter::new(try!(file("control")));
        write!(control, "{}", source_section)?;
        let (summary, description) = crate_info.get_summary_description();

        if lib {
            let ndf = non_default_features.clone();
            let ndf = if ndf.is_empty() { None } else { Some(&ndf) };

            let df = default_features.clone();
            let df = if df.is_empty() { None } else { Some(&df) };

            let lib_package =
                ControlPackage::new(&pkgbase, &deps, ndf, df, &summary, &description, None);

            writeln!(control, "{}", lib_package)?;

            for feature in non_default_features {
                let mut feature_deps = vec![format!("{} (= ${{binary:Version}})",
                                                    lib_package.name()),
                                            "${misc:Depends}".to_string()];

                crate_info.get_feature_dependencies(feature, deb_feature, &mut feature_deps)?;

                let feature_package = ControlPackage::new(&pkgbase,
                                                          &feature_deps,
                                                          None,
                                                          None,
                                                          &summary,
                                                          &description,
                                                          Some(feature));
                writeln!(control, "{}", feature_package)?;

            }
        }

        if !bins.is_empty() {
            let boilerplate = if bins.len() > 1 || bins[0] != bin_name {
                Some(format!("This package contains the following binaries built
        from the\nRust \"{}\" crate:\n- {}",
                             crate_name,
                             bins.join("\n- ")))
            } else {
                None
            };

            let bin_pkg = ControlPackage::new_bin(&pkgbase,
                                                  bin_name,
                                                  &summary,
                                                  &description,
                                                  match boilerplate {
                                                      Some(ref s) => s,
                                                      None => "",
                                                  });

            writeln!(control, "{}", bin_pkg)?;
        }

        let mut copyright = io::BufWriter::new(try!(file("copyright")));
        let deb_copyright =
            copyright::debian_copyright(&package, &pkgbase.srcdir, crate_info.manifest())?;
        writeln!(copyright, "{}", deb_copyright)?;

        try!(fs::create_dir(tempdir.path().join("source")));
        let mut source_format = try!(file("source/format"));
        try!(writeln!(source_format, "3.0 (quilt)"));

        let mut rules = try!(create_exec.open(tempdir.path().join("rules")));
        try!(write!(rules,
                    concat!("#!/usr/bin/make -f\n",
                            "%:\n",
                            "\tdh $@ --buildsystem cargo\n")));

        let mut watch = create.open(tempdir.path().join("watch"))?;
        write!(watch,
               "{}",
               format!(concat!("version=4\n",
                               "opts=filenamemangle=s/.*\\/(.*)\\/download/{}-$1\\.tar\\.\
                                gz/g\\ \n",
                               " https://qa.debian.org/cgi-bin/fakeupstream.\
                                cgi?upstream=crates.io/{} ",
                               ".*/crates/{}/@ANY_VERSION@/download\n"),
                       crate_name,
                       crate_name,
                       crate_name))?;
    }

    try!(fs::rename(tempdir.path(), pkgbase.srcdir.join("debian")));
    tempdir.into_path();


    debcargo_info!(concat!("Package Source: {}\n",
                   "Original Tarball for package: {}\n"),
                   pkgbase.srcdir.to_str().unwrap(), pkgbase.orig_tar_gz.to_str().unwrap());
    debcargo_highlight!("Please update the sections marked FIXME in files inside Debian folder\n");

    Ok(())
}

fn do_cargo_update() -> Result<()> {
    let config = try!(cargo::Config::default());
    let crates_io = try!(cargo::core::SourceId::crates_io(&config));
    let mut registry = cargo::sources::RegistrySource::remote(&crates_io, &config);
    try!(registry.update());
    Ok(())
}

fn real_main() -> Result<()> {
    let m = App::new("debcargo")
        .author(crate_authors!())
        .version(crate_version!())
        .global_setting(AppSettings::ColoredHelp)
        .global_setting(AppSettings::UnifiedHelpMessage)
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .subcommands(vec![SubCommand::with_name("cargo-update").about("Update "),
                          SubCommand::with_name("package")
                              .about("Package a crate from crates.io")
                              .arg_from_usage("<crate> 'Name of the crate to package'")
                              .arg_from_usage("[version] 'Version of the crate to package; may \
                                               include dependency operators'")
                              .arg_from_usage("--bin 'Package binaries from library crates'")
                              .arg_from_usage("--bin-name [name] 'Set package name for \
                                               binaries (implies --bin)'")
                              .arg_from_usage("--distribution [name] 'Set target distribution \
                                               for package (default: unstable)'")])
        .get_matches();
    match m.subcommand() {
        ("cargo-update", _) => do_cargo_update(),
        ("package", Some(ref sm)) => do_package(sm),
        _ => unreachable!(),
    }
}

fn main() {
    if let Err(e) = real_main() {
        println!("{}", e);
        std::process::exit(1);
    }
}
