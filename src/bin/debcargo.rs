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
    let summary = crate_info.summary();
    let pkgid = crate_info.package_id();
    let checksum = crate_info.checksum().ok_or("Could not get crate checksum")?;

    let package = crate_info.package();
    let lock = crate_info.crate_file();

    let mut lib = false;
    let mut bins = Vec::new();
    for target in crate_info.targets() {
        match target.kind() {
            &TargetKind::Lib(_) => {
                lib = true;
            }
            &TargetKind::Bin => {
                bins.push(target.name());
            }
            _ => continue,
        }
    }
    bins.sort();
    if lib && !bins.is_empty() && !package_lib_binaries {
        println!("Ignoring binaries from lib crate; pass --bin to package: {}", bins.join(", "));
        bins.clear();
    }

    let version_suffix = match pkgid.version() {
        _ if !lib && !bins.is_empty() => "".to_string(),
        &Version { major: 0, minor, .. } => format!("-0.{}", minor),
        &Version { major, .. } => format!("-{}", major),
    };
    let crate_pkg_base = format!("{}{}", crate_name_dashed, version_suffix);
    let debsrcname = format!("rust-{}", crate_pkg_base);
    let debver = deb_version(pkgid.version());
    let debsrcdir = Path::new(&format!("{}-{}", debsrcname, debver)).to_owned();
    let orig_tar_gz = Path::new(&format!("{}_{}.orig.tar.gz", debsrcname, debver)).to_owned();

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
    if let Err(e) = fs::rename(entries[0].path(), &debsrcdir) {
        try!(Err(e).chain_err(|| format!("Could not create source directory {0}\nTo regenerate, move or remove {0}", debsrcdir.display())));
    }

    let temp_archive_path = tempdir.path().join(&orig_tar_gz);
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
    try!(fs::rename(temp_archive_path, &orig_tar_gz));

    let (default_features, default_deps) = crate_info.default_deps_features().unwrap();
    let non_default_features = crate_info.non_default_features(&default_features).unwrap();

    let deb_feature = &|f: &str| deb_feature_name(&crate_pkg_base, f);

    let mut deps = Vec::new();
    let mut all_deps = HashMap::new();
    let mut dev_deps = HashSet::new();
    for dep in crate_info.dependencies().iter() {
        if dep.kind() == cargo::core::dependency::Kind::Development {
            dev_deps.insert(dep.name());
            continue;
        }
        if dep.kind() != cargo::core::dependency::Kind::Build {
            if all_deps.insert(dep.name(), dep).is_some() {
                bail!("Duplicate dependency for {}", dep.name());
            }
        }
        if !dep.is_optional() || default_deps.contains(dep.name()) {
            deps.push(try!(deb_dep(dep)));
        }
    }
    deps.sort();
    deps.dedup();

    {
        let file = |name| create.open(tempdir.path().join(name));

        let mut cargo_checksum_json = try!(file("cargo-checksum.json"));
        try!(writeln!(cargo_checksum_json, r#"{{"package":"{}","files":{{}}}}"#, checksum));

        let mut changelog = try!(file("changelog"));
        try!(write!(changelog,
                    concat!("{} ({}-1) {}; urgency=medium\n\n",
                            "  * Package {} {} from crates.io with debcargo {}\n\n",
                            " -- {}  {}\n"),
                    debsrcname, debver, distribution,
                    pkgid.name(), pkgid.version(), crate_version!(),
                    deb_author, chrono::Local::now().to_rfc2822()));

        let mut compat = try!(file("compat"));
        try!(writeln!(compat, "10"));

        let meta = crate_info.metadata();
        let mut control = io::BufWriter::new(try!(file("control")));
        try!(writeln!(control, "Source: {}", debsrcname));
        if crate_name != crate_name_dashed {
            try!(writeln!(control, "X-Cargo-Crate: {}", crate_name));
        }
        if lib {
            try!(writeln!(control, "Section: rust"));
        }
        try!(writeln!(control, "Priority: optional"));
        try!(writeln!(control, "Maintainer: {}", RUST_MAINT));
        try!(writeln!(control, "Uploaders: {}", deb_author));
        let build_deps = if !bins.is_empty() { deps.iter() } else { [].iter() };
        try!(writeln!(control, "Build-Depends:\n {}", vec!["debhelper (>= 10)".to_string(), "dh-cargo".to_string()].iter().chain(build_deps).join(",\n ")));
        try!(writeln!(control, "Standards-Version: 3.9.8"));
        if let Some(ref homepage) = meta.homepage {
            assert!(!homepage.contains('\n'));
            try!(writeln!(control, "Homepage: {}", homepage));
        }
        let (summary, description) = if let Some(ref description) = meta.description {
            let mut description = description.trim();
            for article in ["a ", "A ", "an ", "An ", "the ", "The "].iter() {
                description = description.trim_left_matches(article);
            }
            let p1 = description.find('\n');
            let p2 = description.find(". ");
            match p1.into_iter().chain(p2.into_iter()).min() {
                Some(p) => {
                    let s = description[..p].trim_right_matches('.').to_string();
                    let d = description[p+1..].trim();
                    if d.is_empty() {
                        (Some(s), None)
                    } else {
                        (Some(s), Some(d.to_string()))
                    }
                }
                None => (Some(description.trim_right_matches('.').to_string()), None),
            }
        } else {
            (None, None)
        };
        if lib {
            let deb_lib_name = deb_name(&crate_pkg_base);
            try!(writeln!(control, "\nPackage: {}", deb_lib_name));
            try!(writeln!(control, "Architecture: all"));
            try!(writeln!(control, "Depends:\n {}", vec!["${misc:Depends}".to_string()].iter().chain(deps.iter()).join(",\n ")));
            if !non_default_features.is_empty() {
                try!(writeln!(control, "Suggests:\n {}", non_default_features.iter().cloned().map(deb_feature).join(",\n ")));
            }
            if !default_features.is_empty() {
                let default_features = default_features.iter().cloned().sorted();
                try!(writeln!(control, "Provides:\n {}", default_features.into_iter().map(|f| format!("{} (= ${{binary:Version}})", deb_feature(f))).join(",\n ")));
            }
            let lib_summary = match summary {
                None => format!("Source of the Rust {} crate", crate_name),
                Some(ref s) => format!("{} - Source", s),
            };
            let boilerplate = format!(
                concat!("This package contains the source for the Rust {} crate,\n",
                        "packaged for use with cargo, debcargo, and dh-cargo."),
                crate_name);
            try!(write_description(&mut control, &lib_summary, description.as_ref(), Some(&boilerplate)));

            for feature in non_default_features {
                try!(writeln!(control, "\nPackage: {}", deb_feature(feature)));
                try!(writeln!(control, "Architecture: all"));
                let mut feature_deps = vec![
                    format!("{} (= ${{binary:Version}})", deb_lib_name),
                    "${misc:Depends}".to_string()
                ];
                // Track the (possibly empty) additional features required for each dep, to call
                // deb_dep once for all of them.
                let mut deps_features = HashMap::new();
                let features = crate_info.summary().features();
                for dep_str in features.get(feature).unwrap() {
                    let mut dep_tokens = dep_str.splitn(2, '/');
                    let dep_name = dep_tokens.next().unwrap();
                    match dep_tokens.next() {
                        None if features.contains_key(dep_name) => {
                            if !default_features.contains(dep_name) {
                                feature_deps.push(format!("{} (= ${{binary:Version}})", deb_feature(dep_name)));
                            }
                        }
                        opt_dep_feature => {
                            deps_features.entry(dep_name).or_insert(vec![]).extend(opt_dep_feature.into_iter().map(String::from));
                        }
                    }
                }
                for (dep_name, dep_features) in deps_features.into_iter().sorted() {
                    if let Some(&dep_dependency) = all_deps.get(dep_name) {
                        if dep_features.is_empty() {
                            feature_deps.push(try!(deb_dep(dep_dependency)));
                        } else {
                            let inner = dep_dependency.clone_inner().set_features(dep_features);
                            feature_deps.push(try!(deb_dep(&inner.into_dependency())));
                        }
                    } else if dev_deps.contains(dep_name) {
                        continue;
                    } else {
                        bail!("Feature {} depended on non-existent dep {}", feature, dep_name);
                    };
                }
                try!(writeln!(control, "Depends:\n {}", feature_deps.into_iter().join(",\n ")));
                let feature_summary = match summary {
                    None => format!("Rust {} crate - {} feature", crate_name, feature),
                    Some(ref s) => format!("{} - {} feature", s, feature),
                };
                let boilerplate = format!(
                    concat!("This dependency package depends on the additional crates required for the\n",
                            "{} feature of the Rust {} crate."),
                    feature, crate_name);
                try!(write_description(&mut control, &feature_summary, description.as_ref(), Some(&boilerplate)));
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
        let deb_copyright = copyright::debian_copyright(&package, &debsrcdir, crate_info.manifest())?;
        writeln!(copyright, "{}", deb_copyright)?;

        try!(fs::create_dir(tempdir.path().join("source")));
        let mut source_format = try!(file("source/format"));
        try!(writeln!(source_format, "3.0 (quilt)"));

        let mut rules = try!(create_exec.open(tempdir.path().join("rules")));
        try!(write!(rules, concat!("#!/usr/bin/make -f\n",
                                   "%:\n",
                                   "\tdh $@ --buildsystem cargo\n")));
    }

    try!(fs::rename(tempdir.path(), debsrcdir.join("debian")));
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
