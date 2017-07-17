use toml;

use std::io::Read;
use std::collections::HashMap;
use std::path::Path;
use std::default::Default;
use std::fs::File;
use debian::copyright::Files as CFiles;
use errors::*;


#[derive(Deserialize, Debug, Clone)]
pub struct Overrides {
    source: Option<Source>,
    packages: Option<HashMap<String, Package>>,
    copyright: Option<Copyright>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Source {
    section: Option<String>,
    policy: Option<String>,
    homepage: Option<String>,
    build_depends: Option<Vec<String>>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Package {
    summary: Option<String>,
    description: Option<String>,
    depends: Option<Vec<String>>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Copyright {
    source: Option<String>,
    ignore: Option<Vec<String>>,
    files: Option<HashMap<String, Files>>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Files {
    copyright: Vec<String>,
    license: String,
}

pub trait OverrideDefaults {
    fn apply_overrides(&mut self, overrides: &Overrides);
}


impl Default for Overrides {
    fn default() -> Self {
        Overrides {
            source: None,
            packages: None,
            copyright: None,
        }
    }
}

impl Default for Copyright {
    fn default() -> Self {
        Copyright {
            source: None,
            files: None,
            ignore: None,
        }
    }
}

impl Copyright {
    pub fn is_files_present(&self) -> bool {
        self.files.is_some()
    }

    pub fn files(&self) -> Option<&HashMap<String, Files>> {
        self.files.as_ref()
    }

    pub fn ignore(&self) -> Option<&Vec<String>> {
        self.ignore.as_ref()
    }
}

impl Overrides {
    pub fn is_source_present(&self) -> bool {
        self.source.is_some()
    }

    pub fn is_packages_present(&self) -> bool {
        self.packages.is_some()
    }

    pub fn is_copyright_present(&self) -> bool {
        self.copyright.is_some()
    }

    pub fn is_files_present(&self) -> bool {
        match self.copyright {
            Some(ref copyright) => copyright.is_files_present(),
            None => false,
        }
    }

    pub fn file_section_for(&self, filename: &str) -> Option<CFiles> {
        if self.is_copyright_present() {
            let copyright = self.copyright.to_owned();
            match copyright.unwrap().files() {
                Some(files) => {
                    if files.contains_key(filename) {
                        let value = files.get(filename).unwrap();
                        return Some(CFiles::new(filename,
                                                value.copyright.join("\n ").as_str(),
                                                &value.license,
                                                ""));
                    }
                }
                None => return None,
            }
        }
        None
    }

    pub fn copyright_ignores(&self) -> Option<Vec<&str>> {
        if self.is_copyright_present() {
            if let Some(ref copyright) = self.copyright {
                if let Some(ignores) = copyright.ignore() {
                    let ignore = ignores.iter().map(|x| x.as_str()).collect();
                    return Some(ignore);
                }
            }
        }
        None
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
