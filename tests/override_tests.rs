extern crate debcargo;

use std::path::Path;
use debcargo::overrides::parse_overrides;


#[test]
fn source_package_override() {
    let filepath = Path::new("tests/clap_override.toml");

    let overrides = parse_overrides(&filepath);
    assert!(overrides.is_ok());

    let overrides = overrides.unwrap();

    assert!(overrides.is_source_present());
    assert!(overrides.is_packages_present());
    assert!(overrides.is_files_present());

    let policy = overrides.policy_version();
    assert!(policy.is_some());
    assert_eq!(policy.unwrap(), "4.0.0");

    let homepage = overrides.homepage();
    assert!(homepage.is_some());
    assert_eq!(homepage.unwrap(), "https://clap.rs");

    assert!(overrides.section().is_none());
    assert!(overrides.build_depends().is_none());

    let second_file = Path::new("tests/debcargo_override.toml");
    let overrides = parse_overrides(&second_file);
    assert!(overrides.is_ok());


    let overrides = overrides.unwrap();

    assert!(overrides.is_source_present());

    let section = overrides.section();
    assert!(section.is_some());
    assert_eq!(section.unwrap(), "misc");


    assert!(overrides.is_packages_present());
    let sd = overrides.summary_description_for("debcargo");
    assert!(sd.is_some());

    if let Some((s, d)) = sd {
        assert_eq!(s, "Tool to create Debian package from Rust crate");
        assert_eq!(d,
                   "\
This package provides debcargo a tool to create Debian source package from \
                    Rust
crate. The package created by this tool is as per the packaging policy \
                    set by
Debian Rust team.
");
    }

    assert!(!overrides.is_files_present());
}

#[test]
fn files_override() {
    let filepath = Path::new("tests/clap_override.toml");
    let overrides = parse_overrides(&filepath);
    assert!(overrides.is_ok());

    let overrides = overrides.unwrap();
    assert!(overrides.is_files_present());

    let wildcard_files = overrides.file_section_for("*");
    assert!(wildcard_files.is_some());

    let debian_files = overrides.file_section_for("debian/*");
    assert!(debian_files.is_some());

    let wildcard_files = wildcard_files.unwrap();
    assert_eq!(wildcard_files.copyright_str(),
               "2015-2016, Kevin B. Knapp <kbknapp@gmail.com");
    assert_eq!(wildcard_files.license(), "MIT");
    assert_eq!(wildcard_files.files(), "*");

    let debian_files = debian_files.unwrap();
    assert_eq!(debian_files.files(), "debian/*");
    assert_eq!(debian_files.copyright_str(),
               "2017, Vasudev Kamath <vasudev@copyninja.info");
    assert_eq!(debian_files.license(), "MIT");

    let dontexist_files = overrides.file_section_for("./dontexists.rs");
    assert!(dontexist_files.is_none());

    let non_existent_file = Path::new("tests/Idontexist");
    let overrides = parse_overrides(&non_existent_file);
    assert!(overrides.is_err());
}
