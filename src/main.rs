extern crate cargo;
#[macro_use] extern crate clap;
extern crate chrono;
#[macro_use] extern crate error_chain;
extern crate flate2;
extern crate itertools;
extern crate regex;
extern crate semver;
extern crate semver_parser;
extern crate tar;
extern crate tempdir;
extern crate walkdir;

use cargo::core::{Dependency, Registry, Source, TargetKind};
use clap::{App, AppSettings, SubCommand};
use itertools::Itertools;
use regex::Regex;
use semver::Version;
use std::collections::{HashMap, HashSet};
use std::fmt::{self, Write as FmtWrite};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, Read, Write as IoWrite};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

error_chain! {
    foreign_links {
        Io(io::Error);
        Cargo(Box<cargo::CargoError>);
        Regex(regex::Error);
        WalkDir(walkdir::Error);
    }
}

const RUST_MAINT: &'static str = "Rust Maintainers <pkg-rust-maintainers@lists.alioth.debian.org>";

fn hash<H: Hash>(hashable: &H) -> u64 {
    #![allow(deprecated)]
    let mut hasher = std::hash::SipHasher::new();
    hashable.hash(&mut hasher);
    hasher.finish()
}

/// Translates a semver into a Debian version. Omits the build metadata, and uses a ~ before the
/// prerelease version so it compares earlier than the subsequent release.
fn deb_version(v: &Version) -> String {
    let mut s = format!("{}.{}.{}", v.major, v.minor, v.patch);
    for (n, id) in v.pre.iter().enumerate() {
        write!(s, "{}{}", if n == 0 { '~' } else { '.' }, id).unwrap();
    }
    s
}

enum V { M(u64), MM(u64, u64), MMP(u64, u64, u64) }

impl V {
    fn inclast(&self) -> V {
        use V::*;
        match *self {
            M(major) => M(major+1),
            MM(major, minor) => MM(major, minor+1),
            MMP(major, minor, patch) => MMP(major, minor, patch+1),
        }
    }
}

impl fmt::Display for V {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use V::*;
        match *self {
            M(major) => write!(f, "{}", major),
            MM(major, minor) => write!(f, "{}.{}", major, minor),
            MMP(major, minor, patch) => write!(f, "{}.{}.{}", major, minor, patch),
        }
    }
}

fn deb_name(name: &str) -> String {
    format!("librust-{}-dev", name.replace('_', "-"))
}

fn deb_feature_name(name: &str, feature: &str) -> String {
    format!("librust-{}+{}-dev", name.replace('_', "-"), feature.replace('_', "-"))
}

/// Translates a Cargo dependency into a Debian package dependency.
fn deb_dep(dep: &Dependency) -> String {
    use semver_parser::range::*;
    use semver_parser::range::Op::*;
    use self::V::*;
    let mut packages = Vec::new();
    if dep.uses_default_features() {
        packages.push(deb_feature_name(dep.name(), "default"));
    }
    for feature in dep.features() {
        packages.push(deb_feature_name(dep.name(), feature));
    }
    if packages.is_empty() {
        packages.push(deb_name(dep.name()));
    }
    let req = semver_parser::range::parse(&dep.version_req().to_string()).unwrap();
    let mut deps = Vec::new();
    for name in packages {
        for p in &req.predicates {
            // Cargo/semver and Debian handle pre-release versions quite differently, so a versioned
            // Debian dependency cannot properly handle pre-release crates.  Don't package pre-release
            // crates or crates that depend on pre-release crates.
            if !p.pre.is_empty() {
                writeln!(io::stderr(), "Warning: dependency on prerelease version: {} {:?}", dep.name(), p).unwrap();
            }
            let mmp = match (p.minor, p.patch) {
                (None, None) => M(p.major),
                (Some(minor), None) => MM(p.major, minor),
                (Some(minor), Some(patch)) => MMP(p.major, minor, patch),
                (None, Some(_)) => panic!("semver had patch without minor"),
            };
            match &p.op {
                &Ex => {
                    deps.push(format!("{} (>= {})", name, mmp));
                    deps.push(format!("{} (<< {})", name, mmp.inclast()));
                }
                &Gt => deps.push(format!("{} (>> {})", name, mmp)),
                &GtEq => deps.push(format!("{} (>= {})", name, mmp)),
                &Lt => deps.push(format!("{} (<< {})", name, mmp)),
                &LtEq => deps.push(format!("{} (<< {})", name, mmp.inclast())),
                &Tilde => {
                    deps.push(format!("{} (>= {})", name, mmp));
                    if let MMP(major, minor, _) = mmp {
                        deps.push(format!("{} (<< {})", name, MM(major, minor+1)));
                    } else {
                        deps.push(format!("{} (<< {})", name, mmp.inclast()));
                    }
                }
                &Compatible => {
                    deps.push(format!("{} (>= {})", name, mmp));
                    match mmp {
                        M(_) => {
                            deps.push(format!("{} (<< {})", name, mmp.inclast()));
                        }
                        MM(0, minor) | MMP(0, minor, _) => {
                            deps.push(format!("{} (<< {})", name, MM(0, minor+1)));
                        }
                        MM(major, _) | MMP(major, _, _) => {
                            deps.push(format!("{} (<< {})", name, M(major+1)));
                        }
                    }
                }
                &Wildcard(WildcardVersion::Major) => {
                    deps.push(format!("{}", name));
                }
                &Wildcard(_) => {
                    deps.push(format!("{} (>= {})", name, mmp));
                    deps.push(format!("{} (<< {})", name, mmp.inclast()));
                }
            }
        }
    }
    deps.join(", ")
}

/// Retrieve one of a series of environment variables, and provide a friendly error message for
/// non-UTF-8 values.
fn get_envs(keys: &[&str]) -> Result<Option<String>> {
    use std::env::VarError;
    for key in keys {
        match std::env::var(key) {
            Ok(val) => { return Ok(Some(val)); }
            Err(e@VarError::NotUnicode(_)) => {
                return Err(e).chain_err(|| format!("Environment variable ${} not valid UTF-8", key));
            }
            Err(VarError::NotPresent) => {},
        }
    }
    Ok(None)
}

/// Determine a name and email address from environment variables.
fn get_deb_author() -> Result<String> {
    let name = try!(try!(get_envs(&["DEBFULLNAME", "NAME"]))
                    .ok_or("Unable to determine your name; please set $DEBFULLNAME or $NAME"));
    let email = try!(try!(get_envs(&["DEBEMAIL", "EMAIL"]))
                     .ok_or("Unable to determine your email; please set $DEBEMAIL or $EMAIL"));
    Ok(format!("{} <{}>", name, email))
}

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

fn real_main() -> Result<()> {
    let matches = App::new("debcargo")
        .author(crate_authors!())
        .version(crate_version!())
        .global_setting(AppSettings::ColoredHelp)
        .global_setting(AppSettings::UnifiedHelpMessage)
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .subcommand(SubCommand::with_name("package")
            .about("Package a crate from crates.io")
            .arg_from_usage("<crate> 'Name of the crate to package'")
            .arg_from_usage("[version] 'Version of the crate to package; may include dependency operators'")
            .arg_from_usage("--bin 'Package binaries from library crates'")
            .arg_from_usage("--bin-name [name] 'Set package name for binaries (implies --bin)'")
        ).get_matches();
    let matches = matches.subcommand_matches("package").unwrap();
    let crate_name = matches.value_of("crate").unwrap();
    let crate_name_dashed = crate_name.replace('_', "-");
    let version = matches.value_of("version");
    let package_lib_binaries = matches.is_present("bin") || matches.is_present("bin-name");
    let bin_name = matches.value_of("bin-name").unwrap_or(&crate_name_dashed);

    let deb_author = try!(get_deb_author());

    // Default to an exact match if no operator specified
    let version = version.map(|v| if v.starts_with(|c: char| c.is_digit(10)) { ["=", v].concat() } else { v.to_string() });

    let config = try!(cargo::Config::default());
    let crates_io = try!(cargo::core::SourceId::crates_io(&config));
	let mut registry = cargo::sources::RegistrySource::remote(&crates_io, &config);
    let dependency = try!(cargo::core::Dependency::parse_no_deprecated(&crate_name, version.as_ref().map(String::as_str), &crates_io));
    let summaries = try!(registry.query(&dependency));
    let summary = try!(summaries.iter().max_by_key(|s| s.package_id())
                     .ok_or_else(|| format!("Couldn't find any package matching {} {}",
                                            dependency.name(), dependency.version_req())));
    let pkgid = summary.package_id();
    let checksum = try!(summary.checksum().ok_or("Could not get crate checksum"));
    let package = try!(registry.download(&pkgid));
    let registry_name = format!("{}-{:016x}", crates_io.url().host_str().unwrap_or(""), hash(&crates_io).swap_bytes());
    let crate_filename = format!("{}-{}.crate", pkgid.name(), pkgid.version());
    let lock = try!(config.registry_cache_path().join(&registry_name).open_ro(&crate_filename, &config, &crate_filename));

    let manifest = package.manifest();

    let mut lib = false;
    let mut bins = Vec::new();
    for target in manifest.targets() {
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

    let debsrcname = format!("rust-{}", crate_name_dashed);
    let debver = deb_version(pkgid.version());
    let debsrcdir = Path::new(&format!("{}-{}", debsrcname, debver)).to_owned();
    let orig_tar_gz = Path::new(&format!("{}_{}.orig.tar.gz", debsrcname, debver)).to_owned();
    if orig_tar_gz.exists() {
        bail!(format!("File already exists: {}", orig_tar_gz.display()));
    }
    fs::copy(lock.path(), &orig_tar_gz).unwrap();

    let mut archive = tar::Archive::new(try!(flate2::read::GzDecoder::new(lock.file())));
    let tempdir = try!(tempdir::TempDir::new_in(".", "debcargo"));
    for entry in try!(archive.entries()) {
        let mut entry = try!(entry);
        // Filter out static libraries, to avoid needing to patch all the winapi crates to remove
        // import libraries.
        match try!(entry.path()).extension() {
            Some(ext) if ext == "a" => continue,
            _ => {}
        }
        if !try!(entry.unpack_in(tempdir.path())) {
            bail!("Crate contained path traversals via '..'");
        }
    }
    let entries = try!(try!(tempdir.path().read_dir()).collect::<io::Result<Vec<_>>>());
    if entries.len() != 1 || !try!(entries[0].file_type()).is_dir() {
        bail!(format!("{} did not unpack to a single top-level directory", crate_filename));
    }
    try!(fs::rename(entries[0].path(), &debsrcdir));

    let mut copyright_notices = HashSet::new();
    let copyright_notice_re = try!(Regex::new(r"(?:[Cc]opyright|©)(?:\s|[©:,()Cc<])*\b\d{4}\b.*$"));
    for entry in walkdir::WalkDir::new(&debsrcdir) {
        let entry = try!(entry);
        if entry.file_type().is_file() {
            let copying_file = entry.file_name().to_string_lossy().starts_with("COPYING");
            let file = try!(fs::File::open(entry.path()));
            let reader = io::BufReader::new(file);
            for line in reader.lines() {
                if let Ok(line) = line {
                    if let Some((start, end)) = copyright_notice_re.find(&line) {
                        let notice = line[start..end].trim_right().trim_right_matches(". See the COPYRIGHT").to_string();
                        // Skip the copyright notices from the GPL/LGPL itself.
                        if copying_file && notice.contains("Free Software Foundation, Inc.") {
                            continue;
                        }
                        copyright_notices.insert(notice);
                    }
                } else {
                    break;
                }
            }
        }
    }
    let copyright_notices = copyright_notices.into_iter().sorted();

    let features = summary.features();
    let mut default_features = HashSet::new();
    let mut default_deps = HashSet::new();
    let mut defaults = Vec::new();
    defaults.push("default");
    default_features.insert("default");
    while let Some(feature) = defaults.pop() {
        match features.get(feature) {
            Some(l) => {
                default_features.insert(feature);
                for f in l {
                    defaults.push(f);
                }
            }
            None => { default_deps.insert(feature); }
        }
    }
    for (feature, deps) in features {
        if deps.is_empty() {
            default_features.insert(feature.as_str());
        }
    }
    let non_default_features = features.keys().map(String::as_str).filter(|f| !default_features.contains(f)).sorted();

    let deb_feature = &|f: &str| deb_feature_name(&crate_name_dashed, f);

    let mut deps = Vec::new();
    let mut all_deps = HashMap::new();
    let mut dev_deps = HashSet::new();
    for dep in manifest.dependencies().iter() {
        if dep.kind() == cargo::core::dependency::Kind::Development {
            dev_deps.insert(dep.name());
            continue;
        }
        if dep.kind() != cargo::core::dependency::Kind::Build {
            if all_deps.insert(dep.name(), dep).is_some() {
                bail!(format!("Duplicate dependency for {}", dep.name()));
            }
        }
        if !dep.is_optional() || default_deps.contains(dep.name()) {
            deps.push(deb_dep(dep));
        }
    }

    let mut create = fs::OpenOptions::new();
    create.write(true).create_new(true);
    let mut create_exec = create.clone();
    create_exec.mode(0o777);

    {
        let file = |name| create.open(tempdir.path().join(name));

        let mut cargo_checksum_json = try!(file("cargo-checksum.json"));
        try!(writeln!(cargo_checksum_json, r#"{{"package":"{}","files":{{}}}}"#, checksum));

        let mut changelog = try!(file("changelog"));
        try!(write!(changelog,
                    concat!("{} ({}-1) unstable; urgency=medium\n\n",
                            "  * Package {} {} from crates.io with debcargo {}\n\n",
                            " -- {}  {}\n"),
                    debsrcname, debver,
                    pkgid.name(), pkgid.version(), crate_version!(),
                    deb_author, chrono::Local::now().to_rfc2822()));

        let mut compat = try!(file("compat"));
        try!(writeln!(compat, "10"));

        let meta = manifest.metadata();
        let mut control = io::BufWriter::new(try!(file("control")));
        try!(writeln!(control, "Source: {}", debsrcname));
        if crate_name != crate_name_dashed {
            try!(writeln!(control, "X-Cargo-Crate: {}", crate_name));
        }
        try!(writeln!(control, "Section: libdevel"));
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
            let deb_lib_name = deb_name(crate_name);
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
                            feature_deps.push(deb_dep(dep_dependency));
                        } else {
                            let inner = dep_dependency.clone_inner().set_features(dep_features);
                            feature_deps.push(deb_dep(&inner.into_dependency()));
                        }
                    } else if dev_deps.contains(dep_name) {
                        continue;
                    } else {
                        bail!(format!("Feature {} depended on non-existent dep {}", feature, dep_name));
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
        try!(writeln!(copyright, "Downloaded from the crate \"{}\" on crates.io via Cargo.\n", crate_name));
        if !meta.authors.is_empty() {
            try!(writeln!(copyright, "Upstream authors:"));
            for author in meta.authors.iter() {
                try!(writeln!(copyright, "- {}", author));
            }
            try!(writeln!(copyright, ""));
        }
        if !copyright_notices.is_empty() {
            try!(writeln!(copyright, "Copyright notices:"));
            for notice in copyright_notices.iter() {
                try!(writeln!(copyright, "- {}", notice));
            }
            try!(writeln!(copyright, ""));
        }
        if let Some(ref license_file_name) = meta.license_file {
            let license_file = package.manifest_path().with_file_name(license_file_name);
            let mut text = Vec::new();
            try!(try!(fs::File::open(license_file)).read_to_end(&mut text));
            try!(copyright.write_all(&text));
        } else if let Some(ref licenses) = meta.license {
            try!(writeln!(copyright, "License: {}", licenses));
            for license in licenses.trim().to_lowercase().replace('/', " or ").split(" or ") {
                let text = match license.trim().trim_right_matches('+') {
                    "agpl-3.0" => include_str!("licenses/AGPL-3.0"),
                    "apache-2.0" => include_str!("licenses/Apache-2.0"),
                    "bsd-2-clause" => include_str!("licenses/BSD-2-Clause"),
                    "bsd-3-clause" => include_str!("licenses/BSD-3-Clause"),
                    "cc0-1.0" => include_str!("licenses/CC0-1.0"),
                    "gpl-2.0" => include_str!("licenses/GPL-2.0"),
                    "gpl-3.0" => include_str!("licenses/GPL-3.0"),
                    "isc" => include_str!("licenses/ISC"),
                    "lgpl-2.0" => include_str!("licenses/LGPL-2.0"),
                    "lgpl-2.1" => include_str!("licenses/LGPL-2.1"),
                    "lgpl-3.0" => include_str!("licenses/LGPL-3.0"),
                    "mit" => include_str!("licenses/MIT"),
                    "mpl-2.0" => include_str!("licenses/MPL-2.0"),
                    "unlicense" => include_str!("licenses/Unlicense"),
                    "zlib" => include_str!("licenses/Zlib"),
                    license => bail!(format!("Unrecognized crate license: {} (parsed from {})", license, licenses)),
                };
                try!(write!(copyright, "\n{:->79}\n\n{}", "", text));
            }
        } else {
            bail!("Crate has no license or license_file");
        }

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

fn main() {
    if let Err(e) = real_main() {
        println!("{}", e);
        std::process::exit(1);
    }
}
