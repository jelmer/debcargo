extern crate debcargo;
extern crate cargo;
#[macro_use] extern crate clap;
extern crate chrono;
#[macro_use] extern crate error_chain;
extern crate flate2;
extern crate itertools;
extern crate semver;
extern crate semver_parser;
extern crate tar;
extern crate tempdir;

use cargo::core::{Dependency, Source};
use clap::{App, AppSettings, ArgMatches, SubCommand};
use itertools::Itertools;
use semver::Version;
use std::collections::{HashMap, HashSet};
use std::fmt::{self, Write as FmtWrite};
use std::fs;
use std::io::{self, Write as IoWrite};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::iter::FromIterator;

use debcargo::errors::*;
use debcargo::copyright;
use debcargo::crates::CrateInfo;
use debcargo::debian::{PkgBase, Source as ControlSource, Package as
                       ControlPackage};
use debcargo::debian::{get_deb_author, deb_feature_name};

/// Write a Description field with proper formatting.
fn write_description<W: IoWrite>(out: &mut W, summary: &str, longdesc: Option<&String>, boilerplate: Option<&String>) -> Result<()> {
    assert!(!summary.contains('\n'));
    try!(writeln!(out, "Description: {}", summary));
    for (n, ref s) in longdesc.iter().chain(boilerplate.iter()).enumerate() {
        if n != 0 {
            try!(writeln!(out, " ."));
        }
        for line in s.trim().lines() {
            let line = line.trim();
            if line.is_empty() {
                try!(writeln!(out, " ."));
            } else if line.starts_with("- ") {
                try!(writeln!(out, "  {}", line));
            } else {
                try!(writeln!(out, " {}", line));
            }
        }
    }
    Ok(())
}

fn do_package(matches: &ArgMatches) -> Result<()> {
    let crate_name = matches.value_of("crate").unwrap();
    let crate_name_dashed = crate_name.replace('_', "-");
    let version = matches.value_of("version");
    let package_lib_binaries = matches.is_present("bin") || matches.is_present("bin-name");
    let bin_name = matches.value_of("bin-name").unwrap_or(&crate_name_dashed);
    let distribution = matches.value_of("distribution").unwrap_or("unstable");

    let deb_author = try!(get_deb_author());


    let crate_info = CrateInfo::new(crate_name, version)?;
    let pkgid = crate_info.package_id();
    let checksum = crate_info.checksum().ok_or("Could not get crate checksum")?;

    let package = crate_info.package();
    let lock = crate_info.crate_file();
    let meta = crate_info.metadata();

    let lib = crate_info.is_lib();
    let mut bins = crate_info.get_binary_targets();

    let (default_features, default_deps) = crate_info.default_deps_features().unwrap();
    let non_default_features = crate_info.non_default_features(&default_features).unwrap();
    let (dev_deps, all_deps, deps) = crate_info.get_dependencies(&default_deps).unwrap();

    let build_deps =  if !bins.is_empty() { deps.iter() } else { [].iter() };


    if lib && !bins.is_empty() && !package_lib_binaries {
        println!("Ignoring binaries from lib crate; pass --bin to package: {}", bins.join(", "));
        bins.clear();
    }

    let version_suffix = match pkgid.version() {
        _ if !lib && !bins.is_empty() => "".to_string(),
        &Version { major: 0, minor, .. } => format!("-0.{}", minor),
        &Version { major, .. } => format!("-{}", major),
    };


    let pkgbase = PkgBase::new(crate_name, &version_suffix, pkgid.version())?;
    let source_section = ControlSource::new(&pkgbase,
                                            if let Some(ref home) = meta.homepage { home.as_str() } else { ""},
                                            &lib,
                                            &build_deps.as_slice())?;


    let mut create = fs::OpenOptions::new();
    create.write(true).create_new(true);
    let mut create_exec = create.clone();
    create_exec.mode(0o777);

    // Filter out static libraries, to avoid needing to patch all the winapi crates to remove
    // import libraries.
    let remove_path = |path: &Path| match path.extension() {
        Some(ext) if ext == "a" => true,
        _ => false,
    };

    let mut archive = tar::Archive::new(try!(flate2::read::GzDecoder::new(lock.file())));
    let tempdir = try!(tempdir::TempDir::new_in(".", "debcargo"));
    let mut source_modified = false;
    for entry in try!(archive.entries()) {
        let mut entry = try!(entry);
        if remove_path(&try!(entry.path())) {
            source_modified = true;
            continue;
        }
        if !try!(entry.unpack_in(tempdir.path())) {
            bail!("Crate contained path traversals via '..'");
        }
    }
    let entries = try!(try!(tempdir.path().read_dir()).collect::<io::Result<Vec<_>>>());
    if entries.len() != 1 || !try!(entries[0].file_type()).is_dir() {
        bail!("{}-{}.crate did not unpack to a single top-level directory",
              pkgid.name(), pkgid.version());
    }
    // If we didn't already have a source directory, assume we can safely overwrite the
    // .orig.tar.gz file.
    if let Err(e) = fs::rename(entries[0].path(), &pkgbase.srcdir) {
        try!(Err(e).chain_err(|| format!("Could not create source directory {0}\nTo regenerate, move or remove {0}", pkgbase.srcdir.display())));
    }

    let temp_archive_path = tempdir.path().join(&pkgbase.orig_tar_gz);
    if source_modified {
        // Generate new .orig.tar.gz without the omitted files.
        let mut f = lock.file();
        use std::io::Seek;
        try!(f.seek(io::SeekFrom::Start(0)));
        let mut archive = tar::Archive::new(try!(flate2::read::GzDecoder::new(f)));
        let mut new_archive = tar::Builder::new(flate2::write::GzEncoder::new(try!(create.open(&temp_archive_path)), flate2::Compression::Best));
        for entry in try!(archive.entries()) {
            let entry = try!(entry);
            if !remove_path(&try!(entry.path())) {
                try!(new_archive.append(&entry.header().clone(), entry));
            }
        }
        try!(new_archive.finish());
        try!(writeln!(io::stderr(), "Filtered out files from .orig.tar.gz"));
    } else {
        try!(fs::copy(lock.path(), &temp_archive_path));
    };
    try!(fs::rename(temp_archive_path, &pkgbase.orig_tar_gz));



    let deb_feature = &|f: &str| deb_feature_name(&pkgbase.crate_pkg_base, f);


    {
        let file = |name| create.open(tempdir.path().join(name));

        let mut cargo_checksum_json = try!(file("cargo-checksum.json"));
        try!(writeln!(cargo_checksum_json, r#"{{"package":"{}","files":{{}}}}"#, checksum));

        let mut changelog = try!(file("changelog"));
        try!(write!(changelog,
                    concat!("{} ({}-1) {}; urgency=medium\n\n",
                            "  * Package {} {} from crates.io with debcargo {}\n\n",
                            " -- {}  {}\n"),
                    source_section.srcname(), pkgbase.debver, distribution,
                    pkgid.name(), pkgid.version(), crate_version!(),
                    deb_author, chrono::Local::now().to_rfc2822()));

        let mut compat = try!(file("compat"));
        try!(writeln!(compat, "10"));


        let mut control = io::BufWriter::new(try!(file("control")));
        write!(control, "{}", source_section)?;
        let (summary, description) = crate_info.get_summary_description();

        if lib {
            let ndf = non_default_features.clone();
            let ndf = if ndf.is_empty() { None } else { Some(&ndf)};

            let df = default_features.clone();
            let df = if df.is_empty() { None } else { Some(&df) };

            let lib_package = ControlPackage::new(&pkgbase, &deps, ndf, df,
                                                  &summary,
                                                  &description,
                                                  None);

            writeln!(control, "{}", lib_package)?;

            for feature in non_default_features {
                let mut feature_deps = vec![
                    format!("{} (= ${{binary:Version}})", lib_package.name()),
                    "${misc:Depends}".to_string()
                ];

                crate_info.get_feature_dependencies(feature,
                                                    deb_feature, &mut feature_deps)?;

                let feature_package = ControlPackage::new(&pkgbase,
                                                          &feature_deps,
                                                          None, None,
                                                          &summary,
                                                          &description,
                                                          Some(feature));
                writeln!(control, "{}", feature_package)?;

            }
        }
        if !bins.is_empty() {
            try!(writeln!(control, "\nPackage: {}", bin_name));
            try!(writeln!(control, "Architecture: any"));
            try!(writeln!(control, "Section: misc"));
            try!(writeln!(control, "Depends: ${{shlibs:Depends}}, ${{misc:Depends}}"));
            let summary = match summary {
                None => format!("Binaries built from the Rust {} crate", crate_name),
                Some(ref s) => s.to_string(),
            };
            let boilerplate = if bins.len() > 1 || bins[0] != bin_name {
                Some(format!("This package contains the following binaries built from the\nRust \"{}\" crate:\n- {}", crate_name, bins.join("\n- ")))
            } else {
                None
            };
            try!(write_description(&mut control, &summary, description.as_ref(), boilerplate.as_ref()));
        }

        let mut copyright = io::BufWriter::new(try!(file("copyright")));
        let deb_copyright = copyright::debian_copyright(&package, &pkgbase.srcdir, crate_info.manifest())?;
        writeln!(copyright, "{}", deb_copyright)?;

        try!(fs::create_dir(tempdir.path().join("source")));
        let mut source_format = try!(file("source/format"));
        try!(writeln!(source_format, "3.0 (quilt)"));

        let mut rules = try!(create_exec.open(tempdir.path().join("rules")));
        try!(write!(rules, concat!("#!/usr/bin/make -f\n",
                                   "%:\n",
                                   "\tdh $@ --buildsystem cargo\n")));
    }

    try!(fs::rename(tempdir.path(), pkgbase.srcdir.join("debian")));
    tempdir.into_path();

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
        .subcommands(vec![
            SubCommand::with_name("cargo-update")
                .about("Update "),
            SubCommand::with_name("package")
                .about("Package a crate from crates.io")
                .arg_from_usage("<crate> 'Name of the crate to package'")
                .arg_from_usage("[version] 'Version of the crate to package; may include dependency operators'")
                .arg_from_usage("--bin 'Package binaries from library crates'")
                .arg_from_usage("--bin-name [name] 'Set package name for binaries (implies --bin)'")
                .arg_from_usage("--distribution [name] 'Set target distribution for package (default: unstable)'"),
        ]).get_matches();
    match m.subcommand() {
        ("cargo-update", _) => do_cargo_update(),
        ("package", Some(ref sm)) => do_package(&sm),
        _ => unreachable!(),
    }
}

fn main() {
    if let Err(e) = real_main() {
        println!("{}", e);
        std::process::exit(1);
    }
}
