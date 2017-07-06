use toml;

use std::io::Read;
use std::collections::HashMap;
use std::path::Path;
use std::fs::File;
use copyright::Files as CFiles;
use errors::*;

#[derive(Deserialize, Debug)]
pub struct Overrides {
    source: Option<Source>,
    packages: Option<HashMap<String, Package>>,
    files: Option<HashMap<String, Files>>,
}

#[derive(Deserialize, Debug)]
pub struct Source {
    section: Option<String>,
    policy: Option<String>,
    homepage: Option<String>,
    build_depends: Option<Vec<String>>
}

#[derive(Deserialize, Debug)]
pub struct Package {
    summary: Option<String>,
    description: Option<String>,
    depends: Option<Vec<String>>
}

#[derive(Deserialize, Debug)]
pub struct Files {
    copyright: Vec<String>,
    license: String,
}

impl Overrides {
    pub fn file_section_for(&self, filename: &str) -> Option<CFiles> {
        match self.files {
            Some(ref files) => {
                if files.contains_key(filename) {
                    let value = files.get(filename).unwrap();
                    Some(CFiles::new(filename,
                                     value.copyright.join("\n ").as_str(),
                                     &value.license,
                                     ""))
                } else {
                    None
                }
            }
            None => None,
        }
    }

    pub fn summary_description_for(&self, pkgname: &str) -> Option<(&str, &str)> {
        match self.packages {
            Some(ref pkg) => {
                if pkg.contains_key(pkgname) {
                    let package = pkg.get(pkgname).unwrap();
                    let s = match package.summary {
                        Some(ref s) => s,
                        None => "",
                    };

                    let d = match package.description {
                        Some(ref d) => d,
                        None => "",
                    };
                    Some((s, d))
                } else {
                    None
                }
            }
            None => None,
        }
    }

    pub fn policy_version(&self) -> Option<&str> {
        if let Some(ref s) = self.source {
            if let Some(ref policy) = s.policy {
                return Some(policy);
            }
        }
        None
    }

    pub fn homepage(&self) -> Option<&str> {
        if let Some(ref s) = self.source {
            if let Some(ref homepage) = s.homepage {
                return Some(homepage);
            }
        }
        None
    }

    pub fn build_depends(&self) -> Option<Vec<&str>> {
        if let Some(ref s) = self.source {
                if let Some(ref bdeps) = s.build_depends {
                    let build_deps = bdeps.iter().map(|x| x.as_str()).collect();
                    return Some(build_deps);
                }
        }
        None
    }

    pub fn section(&self) -> Option<&str> {
        if let Some(ref s) = self.source {
            if let Some(ref section) = s.section {
                return Some(section);
            }
        }
        None
    }
}

pub fn parse_overrides(src: &Path) -> Result<Overrides> {
    let mut override_file = File::open(src)?;
    let mut content = String::new();
    override_file.read_to_string(&mut content)?;

    let overrides: Overrides = toml::from_str(&content)?;
    Ok(overrides)
}
