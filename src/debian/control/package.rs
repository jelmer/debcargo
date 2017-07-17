use std::fmt;

use itertools::Itertools;

use errors::*;
use crates::CrateInfo;
use overrides::{Overrides, OverrideDefaults};
use debian::control::{deb_name, deb_feature_name};

pub struct Package {
    name: String,
    arch: String,
    section: String,
    depends: String,
    suggests: String,
    provides: String,
    summary: String,
    description: String,
    boilerplate: String,
}


impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Package: {}", self.name)?;
        writeln!(f, "Architecture: {}", self.arch)?;

        if !self.section.is_empty() {
            writeln!(f, "Section: {}", self.section)?;
        }

        writeln!(f, "Depends:\n {}", self.depends)?;
        if !self.suggests.is_empty() {
            writeln!(f, "Suggests:\n {}", self.suggests)?;
        }

        if !self.provides.is_empty() {
            writeln!(f, "Provides:\n {}", self.provides)?;
        }

        self.write_description(f)
    }
}

impl Package {
    pub fn new(basename: &str,
               upstream_name: &str,
               crate_info: &CrateInfo,
               feature: Option<&str>)
               -> Result<Package> {
        let deb_feature = &|f: &str| deb_feature_name(basename, f);

        let deps = match feature {
            None => crate_info.non_dev_dependencies()?,
            Some(f) => {
                let mut feature_deps = vec![format!("{} (= ${{binary:Version}})",
                                                    deb_name(basename))];
                crate_info.get_feature_dependencies(f, deb_feature, &mut feature_deps)?;
                feature_deps
            }
        };

        let (default_features, _) = crate_info.default_deps_features();
        let non_default_features = crate_info.non_default_features(&default_features);
        let (summary, description) = crate_info.get_summary_description();


        // Suggests is needed only for main package and not feature package.
        let suggests = if feature.is_none() {
            non_default_features.iter().cloned().map(deb_feature).join(",\n ")
        } else {
            "".to_string()
        };

        // Provides is also only for main package and not feature package.
        let provides = if feature.is_none() {
            default_features.into_iter()
                .map(|f| format!("{} (= ${{binary:Version}})", deb_feature(f)))
                .join(",\n ")
        } else {
            "".to_string()
        };

        let depends = vec!["${misc:Depends}".to_string()]
            .iter()
            .chain(deps.iter())
            .join(",\n ");

        let short_desc = match summary {
            None => format!("Source of Rust {} crate", basename),
            Some(ref s) => {
                format!("{} - {}",
                        s,
                        if let Some(f) = feature { f } else { "Source" })
            }
        };

        let name = match feature {
            None => deb_name(basename),
            Some(s) => deb_feature(s),
        };

        let long_desc = match description {
            None => "".to_string(),
            Some(ref s) => s.to_string(),
        };

        let boilerplate = match feature {
            None => {
                format!(concat!("This package contains the source for the",
                                " Rust {} crate,\npackaged for use with",
                                " cargo, debcargo, and dh-cargo."),
                        upstream_name)
            }
            Some(f) => {
                format!(concat!("This package enables feature {} for the",
                                " Rust {} crate. Purpose of this package",
                                " is\nto pull the additional dependency",
                                " needed to enable feature {}."),
                        f,
                        upstream_name,
                        f)
            }
        };

        Ok(Package {
            name: name,
            arch: "all".to_string(),
            section: "".to_string(),
            depends: depends,
            suggests: suggests,
            provides: provides,
            summary: short_desc,
            description: long_desc,
            boilerplate: boilerplate,
        })
    }

    pub fn new_bin(upstream_name: &str,
                   name: &str,
                   summary: &Option<String>,
                   description: &Option<String>,
                   boilerplate: &str)
                   -> Self {
        let short_desc = match *summary {
            None => format!("Binaries built from the Rust {} crate", upstream_name),
            Some(ref s) => s.to_string(),
        };

        let long_desc = match *description {
            None => "".to_string(),
            Some(ref s) => s.to_string(),
        };

        Package {
            name: name.to_string(),
            arch: "any".to_string(),
            section: "misc".to_string(),
            depends: vec!["${misc:Depends}".to_string(), "${shlibs:Depends}".to_string()]
                .iter()
                .join(",\n "),
            suggests: "".to_string(),
            provides: "".to_string(),
            summary: short_desc,
            description: long_desc,
            boilerplate: boilerplate.to_string(),
        }
    }

    pub fn name(&self) -> &String {
        &self.name
    }

    fn write_description(&self, out: &mut fmt::Formatter) -> fmt::Result {
        writeln!(out, "Description: {}", self.summary)?;
        let description = Some(&self.description);
        let boilerplate = Some(&self.boilerplate);
        for (n, s) in description.iter().chain(boilerplate.iter()).enumerate() {
            if n != 0 {
                writeln!(out, " .")?;
            }
            for line in s.trim().lines() {
                let line = line.trim();
                if line.is_empty() {
                    writeln!(out, " .")?;
                } else if line.starts_with("- ") {
                    writeln!(out, "  {}", line)?;
                } else {
                    writeln!(out, " {}", line)?;
                }
            }
        }
        write!(out, "")
    }
}

impl OverrideDefaults for Package {
    fn apply_overrides(&mut self, overrides: &Overrides) {
        if let Some((s, d)) = overrides.summary_description_for(&self.name) {
            if !s.is_empty() {
                self.summary = s.to_string();
            }

            if !d.is_empty() {
                self.description = d.to_string();
            }
        }
    }
}
