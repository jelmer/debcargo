use walkdir;
use regex;
use chrono::{self, Datelike, NaiveDateTime, DateTime, Utc};
use cargo::core::{manifest, package};
use tempdir::TempDir;
use textwrap::fill;
use git2::Repository;

use std::fmt;
use std::fs;
use std::env;
use std::cmp::Ordering;
use std::path::Path;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};

use errors::*;
use debian::control::get_deb_author;


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
            write!($fmt, "")
        }
    }
}


struct UpstreamInfo {
    name: String,
    contacts: Vec<String>,
    source: String,
}

#[derive(Clone)]
pub struct Files {
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
        if !self.source.is_empty() {
            write!(f, "Source: {}\n", self.source)?;
        }

        write!(f, "")
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
        write!(f, "Copyright: {}\n", self.copyright)?;
        write!(f, "License: {}\n", self.license)?;
        if !self.comment.is_empty() {
            write!(f, "Comment:\n")?;
            format_para!(f, &self.comment)?;
        }

        write!(f, "\n")

    }
}

impl Files {
    pub fn new(name: &str, notice: &str, license: &str, comment: &str) -> Files {
        Files {
            files: name.to_string(),
            copyright: notice.to_string(),
            license: license.to_string(),
            comment: comment.to_string(),
        }
    }

    pub fn files(&self) -> &str {
        &self.files
    }

    pub fn copyright_str(&self) -> &str {
        &self.copyright
    }

    pub fn license(&self) -> &str {
        &self.license
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

macro_rules! default_files {
    ($file:expr, $notice:expr) => {{
        let comment = concat!(
            "FIXME: These notices are extracted from files. Please review ",
            "them before uploading to the archive. Also delete this comment.");
        Files::new($file,
                   $notice,
                   "UNKNOWN; FIXME",
                   &fill(comment, 79))
    }}
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
            notices.push(default_files!(filename, notice));
        }
    }


    Ok(notices)
}

fn get_licenses(license: &str) -> Result<Vec<License>> {
    let mut licenses = HashMap::new();
    let sep = regex::Regex::new(r"(?i:[or|/])")?;

    let lses: Vec<&str> = sep.split(license).filter(|s| s.len() != 0).collect();
    for ls in lses {
        let text: &str = match ls.trim().to_lowercase().trim_right_matches('+') {
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
            "mpl-1.1" => include_str!("licenses/MPL-1.1"),
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

fn copyright_fromgit(repo_url: &str) -> Result<String> {
    let tempdir = TempDir::new_in(".", "debcargo")?;
    let repo = Repository::clone(repo_url, tempdir.path())?;

    let mut revwalker = repo.revwalk()?;
    revwalker.push_head()?;

    // Get the latest and first commit id. This is bit ugly
    let latest_id = revwalker.next().unwrap()?;
    let first_id = revwalker.last().unwrap()?; // revwalker ends here is consumed by last

    let first_commit = repo.find_commit(first_id)?;
    let latest_commit = repo.find_commit(latest_id)?;

    let first_year =
        DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(first_commit.time().seconds(), 0),
                                  Utc)
            .year();

    let latest_year =
        DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(latest_commit.time().seconds(), 0),
                                  Utc)
            .year();



    let notice = match first_year.cmp(&latest_year) {
        Ordering::Equal => format!("{}", first_year),
        _ => format!("{}-{},", first_year, latest_year),
    };

    Ok(notice)
}


pub fn debian_copyright(package: &package::Package,
                        srcdir: &Path,
                        manifest: &manifest::Manifest,
                        guess_harder: bool)
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
    let deb_notice = format!("{}, {}", current_year, get_deb_author().unwrap_or_default());
    files.push(Files::new("debian/*", &deb_notice, &crate_license, ""));

    // Insert catch all block as the first block of copyright file. Capture
    // copyright notice from git log of the upstream repository.
    let years = if guess_harder && !repository.is_empty() {
        match copyright_fromgit(repository) {
            Ok(x) => x,
            Err(e) => {
                debcargo_warn!("Failed to generate d/copyright from git repository {}: {}\n", repository, e);
                "FIXME UNKNOWN".to_string()
            }
        }
    } else {
        "FIXME UNKNOWN".to_string()
    };
    let notice = match meta.authors.len() {
        1 => format!("{} {}", years, &meta.authors[0]),
        _ => {
            let author_notices: Vec<String> = meta.authors
                .iter()
                .map(|s| format!("{} {}", years, s))
                .collect();
            author_notices.join("\n ").trim().to_owned()
        }
    };
    let comment = concat!(
        "FIXME: Since upstream copyright years are not available in ",
        "Cargo.toml, they were extracted from the upstream Git ",
        "repository. This may not be correct information so you should ",
        "review and fix this before uploading to the archive."
    );
    files.insert(
        0,
        Files::new("*", &notice, &crate_license, &fill(comment, 79)),
    );

    Ok(DebCopyright::new(upstream, &files, &licenses))
}
