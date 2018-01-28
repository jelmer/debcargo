extern crate debcargo;

use std::path::Path;
use debcargo::config::parse_config;


#[test]
fn source_package_override() {
    let filepath = Path::new("tests/clap_override.toml");

    let config = parse_config(&filepath);
    assert!(config.is_ok());

    let config = config.unwrap();

    assert!(config.is_source_present());
    assert!(config.is_packages_present());

    let policy = config.policy_version();
    assert!(policy.is_some());
    assert_eq!(policy.unwrap(), "4.0.0");

    let homepage = config.homepage();
    assert!(homepage.is_some());
    assert_eq!(homepage.unwrap(), "https://clap.rs");

    assert!(config.section().is_none());
    assert!(config.build_depends().is_none());

    let second_file = Path::new("tests/debcargo_override.toml");
    let config = parse_config(&second_file);
    assert!(config.is_ok());


    let config = config.unwrap();

    assert!(config.is_source_present());

    let section = config.section();
    assert!(section.is_some());
    assert_eq!(section.unwrap(), "rust");


    assert!(config.is_packages_present());
    let sd = config.summary_description_for("debcargo");
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
}
