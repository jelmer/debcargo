use walkdir;
use regex;
use chrono::{self, Datelike};
use cargo::core::{manifest, package};
use subprocess::{self, Exec};
use tempdir::TempDir;

use std::fmt;
use std::fs;
use std::i32;
use std::env;
use std::str::FromStr;
use std::cmp::Ordering;
use std::path::Path;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read};

use errors::*;
use debian::get_deb_author;

const DEB_COPYRIGHT_FORMAT: &'static str = "https://www.debian.\
                                            org/doc/packaging-manuals/copyright-format/1.0/";


macro_rules! format_para {
    ($fmt: expr, $text:expr) => {
        {
            let text = Some($text.to_string());
            for (n, s) in text.iter().enumerate() {
                if n != 0 {
                    writeln!($fmt, " .")?;
                }

                for line in s.trim().lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        writeln!($fmt, " .")?;
                    } else if line.starts_with("- ") {
                        writeln!($fmt, " {}", line)?;
                    } else {
                        writeln!($fmt, " {}", line)?;
                    }
                }
            }
        }
    }
}


struct UpstreamInfo {
    name: String,
    contacts: Vec<String>,
    source: String,
}

#[derive(Clone)]
struct Files {
    files: String,
    copyright: String,
    license: String,
    comment: String,
}

#[derive(Clone)]
struct License {
    name: String,
    text: String,
}

pub struct DebCopyright {
    format: String,
    upstream: UpstreamInfo,
    files: Vec<Files>,
    licenses: Vec<License>,
}

impl fmt::Display for DebCopyright {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Format: {}\n", self.format)?;
        writeln!(f, "{}", self.upstream)?;

        for file in &self.files {
            write!(f, "{}", file)?;
        }

        for license in &self.licenses {
            write!(f, "{}", license)?;
        }

        write!(f, "\n")
    }
}

impl DebCopyright {
    fn new(u: UpstreamInfo, f: &[Files], l: &[License]) -> DebCopyright {
        DebCopyright {
            format: DEB_COPYRIGHT_FORMAT.to_string(),
            upstream: u,
            files: f.to_vec(),
            licenses: l.to_vec(),
        }
    }
}

impl fmt::Display for UpstreamInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Upstream-Name: {}\n", self.name)?;
        write!(f, "Upstream-Contact:")?;
        for contact in &self.contacts {
            write!(f, " {}\n", contact)?;
        }
        write!(f, "Source: {}\n", self.source)
    }
}

impl UpstreamInfo {
    fn new(name: String, authors: &[String], repo: &str) -> UpstreamInfo {
        UpstreamInfo {
            name: name,
            contacts: authors.to_vec(),
            source: repo.to_string(),
        }
    }
}

impl fmt::Display for Files {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Files: {}\n", self.files)?;
        write!(f, "Copyright: {}", self.copyright)?;
        write!(f, "License: {}\n", self.license)?;
        if !self.comment.is_empty() {
            write!(f, "Comment:\n")?;
            format_para!(f, &self.comment)?;
        }

        write!(f, "\n")

    }
}

impl Files {
    fn new(name: &str, notice: &str, license: &str, comment: &str) -> Files {
        Files {
            files: name.to_string(),
            copyright: notice.to_string(),
            license: license.to_string(),
            comment: comment.to_string(),
        }
    }
}

impl fmt::Display for License {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "License: {}\n", self.name)?;
        format_para!(f, &self.text)?;
        write!(f, "\n")
    }
}

impl License {
    fn new(name: String, text: String) -> License {
        License {
            name: name,
            text: text,
        }
    }
}

fn gen_files(debsrcdir: &Path) -> Result<Vec<Files>> {
    let mut copyright_notices = HashMap::new();
    let copyright_notice_re =
        try!(regex::Regex::new(r"(?:[Cc]opyright|©)(?:\s|[©:,()Cc<])*\b(\d{4}\b.*)$"));

    // Get current working directory and move inside the extracted source of
    // crate. This is necessary so as to capture correct path for files in
    // debian/copyright.
    let current_dir = env::current_dir()?;
    env::set_current_dir(debsrcdir)?;

    // Here we specifically use "." to avoid absolute paths. If we use
    // current_dir then we end up having absolute path from user home directory,
    // which again messes debian/copyright.
    // Use of . creates paths in format ./src/ which is acceptable.
    for entry in walkdir::WalkDir::new(".") {
        let entry = try!(entry);
        if entry.file_type().is_file() {
            let copyright_file = entry.path().to_str().unwrap();
            let file = try!(fs::File::open(entry.path()));
            let reader = BufReader::new(file);
            for line in reader.lines() {
                if let Ok(line) = line {
                    if let Some(m) = copyright_notice_re.captures(&line) {
                        let m = m.get(1).unwrap();
                        let start = m.start();
                        let end = m.end();
                        let notice = line[start..end]
                            .trim_right()
                            .trim_right_matches(". See the COPYRIGHT")
                            .to_string();
                        copyright_notices.insert(copyright_file.to_string(), notice);
                    }
                } else {
                    break;
                }
            }
        }
    }

    // Restore back to original working directory as we can continue without
    // problems.
    env::set_current_dir(current_dir.as_path())?;

    let mut notices: Vec<Files> = Vec::new();
    if !copyright_notices.is_empty() {
        for (filename, notice) in &copyright_notices {
            notices.push(Files::new(filename,
                                    format!(" {}\n", notice).as_str(),
                                    "UNKNOWN; FIXME",
                                    concat!("These notices are extracted from ",
                                            "files. Please review them before ",
                                            "uploading to\n archive. Also delete",
                                            " this comment")));

        }
    }


    Ok(notices)
}

fn get_licenses(license: &str) -> Result<Vec<License>> {
    let mut licenses = HashMap::new();

    for ls in license.trim().replace("/", " or ").split(" or ") {
        let text = match ls.trim().to_lowercase().trim_right_matches('+') {
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
            ls => {
                debcargo_bail!("Unrecognized crate license: {} (parsed from {})",
                               ls,
                               license)
            }
        };
        licenses.insert(ls.to_string(), text.to_string());
    }

    let mut lblocks: Vec<License> = Vec::new();
    if !licenses.is_empty() {
        lblocks.reserve(licenses.capacity());
        for (l, t) in licenses {
            lblocks.push(License::new(l, t));
        }
    }

    Ok(lblocks)
}

fn copyright_fromgit(repo: &str) -> Result<String> {
    let tempdir = TempDir::new_in(".", "debcargo")?;
    Exec::cmd("git").args(&["clone", "--bare", repo, tempdir.path().to_str().unwrap()])
        .stdout(subprocess::NullFile)
        .stderr(subprocess::NullFile)
        .popen()?;

    let first = {
            Exec::cmd("git")
                .args(&["log", "--format=%ad", "--date=format:%Y", "--reverse"])
                .cwd(tempdir.path()) | Exec::cmd("head").arg("-n1")
        }
        .capture()?
        .stdout_str();

    let last = {
            Exec::cmd("git")
                .args(&["log", "--format=%ad", "--date=format:%Y"])
                .cwd(tempdir.path()) | Exec::cmd("head").arg("-n1")
        }
        .capture()?
        .stdout_str();

    let notice = match first.trim().cmp(last.trim()) {
        Ordering::Equal => first,
        _ => format!("{}-{},", first.trim(), last.trim()),
    };

    Ok(notice)
}


pub fn debian_copyright(package: &package::Package,
                        srcdir: &Path,
                        manifest: &manifest::Manifest)
                        -> Result<DebCopyright> {
    let meta = manifest.metadata().clone();
    let repository = match meta.repository {
        None => "",
        Some(ref r) => r,
    };

    let upstream = UpstreamInfo::new(manifest.name().to_string(), &meta.authors, repository);

    let mut licenses: Vec<License> = Vec::new();
    let mut crate_license: String = "".to_string();

    if let Some(ref license_file_name) = meta.license_file {
        let license_file = package.manifest_path().with_file_name(license_file_name);
        let mut text = Vec::new();
        fs::File::open(license_file)?.read_to_end(&mut text)?;
        licenses.reserve(1);
        let stext = String::from_utf8(text)?;
        licenses.push(License::new("UNKNOWN; FIXME".to_string(), stext));
    } else if let Some(ref license) = meta.license {
        licenses = get_licenses(license).unwrap();
        crate_license = license.trim().replace("/", " or ");
    } else {
        debcargo_bail!("Crate has no license or license_file");
    }

    let mut files = gen_files(srcdir)?;

    let current_year = chrono::Local::now().year();
    let deb_notice = format!("{}, {}\n",
                             current_year,
                             get_deb_author().unwrap_or_default());
    files.push(Files::new("debian/*", &deb_notice, &crate_license));

    // Insert catch all block as the first block of copyright file. Capture
    // copyright notice from git log of the upstream repository.
    let notices = copyright_fromgit(repository)?;
    files.insert(0,
                 Files::new("*",
                            format!("{}\n", notices.join("\n ")).as_str(),
                            &crate_license));

    Ok(DebCopyright::new(upstream, &files, &licenses))
}
