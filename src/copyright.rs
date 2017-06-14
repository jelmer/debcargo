use walkdir;
use regex;
use chrono::{self, Datelike};
use itertools::Itertools;
use cargo::core::{manifest, package};

use std::fmt;
use std::fs;
use std::path::Path;
use std::collections::{HashSet, HashMap};
use std::io::{BufRead, BufReader, Read};

use errors::*;
use debian::get_deb_author;

const DEB_COPYRIGHT_FORMAT: &'static str = "https://www.debian.\
                                            org/doc/packaging-manuals/copyright-format/1.0/";


struct UpstreamInfo {
    name: String,
    contacts: Vec<String>,
    source: String,
}

struct Files {
    files: String,
    copyright: String,
    license: String,
}

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
    fn new(u: UpstreamInfo, f: Vec<Files>, l: Vec<License>) -> DebCopyright {
        DebCopyright {
            format: DEB_COPYRIGHT_FORMAT.to_string(),
            upstream: u,
            files: f,
            licenses: l,
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
    fn new(name: String, authors: Vec<String>, repo: String) -> UpstreamInfo {
        UpstreamInfo {
            name: name,
            contacts: authors,
            source: repo,
        }
    }
}

impl fmt::Display for Files {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Files: {}\n", self.files)?;
        write!(f, "Copyright: {}", self.copyright)?;
        write!(f, "License: {}\n\n", self.license)
    }
}

impl Files {
    fn new(name: String, notice: String, license: String) -> Files {
        Files {
            files: name,
            copyright: notice,
            license: license,
        }
    }
}

impl fmt::Display for License {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "License: {}\n", self.name)?;
        let text = Some(&self.text);
        for (n, s) in text.iter().enumerate() {
            if n != 0 {
                writeln!(f, " .")?;
            }

            for line in s.trim().lines() {
                let line = line.trim();
                if line.is_empty() {
                    writeln!(f, " .")?;
                } else if line.starts_with("- ") {
                    writeln!(f, " {}", line)?;
                } else {
                    writeln!(f, " {}", line)?;
                }
            }
        }

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
    for entry in walkdir::WalkDir::new(&debsrcdir) {
        let entry = try!(entry);
        if entry.file_type().is_file() {
            let copyright_file = entry.file_name().to_str().unwrap();
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


    let mut notices: Vec<Files> = Vec::new();
    if !copyright_notices.is_empty() {
        for (filename, notice) in &copyright_notices {
            notices.push(Files::new(filename,
                                    format!(" {}\n", notice).as_str(),
                                    "UNKNOWN; FIXME"));

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
                bail!("Unrecognized crate license: {} (parsed from {})",
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
            lblocks.push(License::new(l.to_string(), t));
        }
    }

    Ok(lblocks)
}

pub fn debian_copyright(package: &package::Package,
                        srcdir: &Path,
                        manifest: &manifest::Manifest)
                        -> Result<DebCopyright> {
    let meta = manifest.metadata().clone();
    let repository = match meta.repository {
        None => "".to_string(),
        Some(r) => r,
    };

    let upstream = UpstreamInfo::new(manifest.name().to_string(), meta.authors, repository);

    let mut licenses: Vec<License> = Vec::new();
    let mut crate_license: String = "".to_string();

    if let Some(ref license_file_name) = meta.license_file {
        let license_file = package.manifest_path().with_file_name(license_file_name);
        let mut text = Vec::new();
        fs::File::open(license_file)?.read_to_end(&mut text)?;
        licenses.reserve(1);
        let stext = String::from_utf8(text)?;
        licenses.push(License::new("FIXME".to_string(), stext));
    } else if let Some(ref license) = meta.license {
        licenses = get_licenses(license).unwrap();
        crate_license = license.trim().replace("/", " or ");
    } else {
        bail!("Crate has no license or license_file");
    }

    let mut files: Vec<Files> = Vec::new();
    files.push(gen_files(srcdir, &crate_license).unwrap());

    let current_year = chrono::Local::now().year();
    let deb_notice = format!("{} {}\n",
                             current_year,
                             get_deb_author().unwrap_or_default());

    files.push(Files::new("debian/*".to_string(), deb_notice, crate_license));
    Ok(DebCopyright::new(upstream, files, licenses))
}

// #[cfg(test)]
// mod tests {
//     use super::{gen_files, get_licenses};
//     use std::path::Path;

//     #[test]
//     fn test_gen_files() {
//         let path = Path::new("rust-nom-3-3.0.0");
//         let files = gen_files(path);
//         let act_str = format!("{}", files.ok().unwrap());
//         let expected = "Files: *
// Copyright: Copyright (c) 2015-2016 Geoffroy Couprie
//  FIXME
// License: UNKNOWN
// ";
//         assert_eq!(act_str, expected);
//     }

//     #[test]
//     fn test_get_licenses() {
//         let ls = "MIT/Apache-2.0".to_string();
//         let act_result = get_licenses(&ls).unwrap_or(Vec::new());
//         assert_eq!(act_result.len(), 2);
//     }

// }
