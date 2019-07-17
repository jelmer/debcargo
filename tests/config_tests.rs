extern crate debcargo;

use debcargo::config::{parse_config, PackageKey};
use std::path::Path;

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

    let filepath = Path::new("tests/debcargo_override.toml");
    let config = parse_config(&filepath);
    assert!(config.is_ok());

    let config = config.unwrap();

    assert!(config.is_source_present());

    let section = config.section();
    assert!(section.is_some());
    assert_eq!(section.unwrap(), "rust");

    assert!(config.is_packages_present());
    let sd = config.package_summary(PackageKey::Bin);
    assert!(sd.is_some());

    if let Some((s, d)) = sd {
        assert_eq!(s, "Tool to create Debian package from Rust crate");
        assert_eq!(
            d,
            "\
This package provides debcargo a tool to create Debian source package from \
                    Rust
crate. The package created by this tool is as per the packaging policy \
                    set by
Debian Rust team.
"
        );
    }
}

#[test]
fn sd_top_level() {
    let filepath = Path::new("tests/debcargo_override_top_level.toml");
    let config = parse_config(&filepath);
    assert!(config.is_ok());

    let config = config.unwrap();

    assert!(config.is_source_present());

    let section = config.section();
    assert!(section.is_some());
    assert_eq!(section.unwrap(), "rust");

    assert_eq!(
        config.summary,
        "Tool to create Debian package from Rust crate"
    );
    assert_eq!(
        config.description,
        "\
This package provides debcargo a tool to create Debian source package from \
                    Rust
crate. The package created by this tool is as per the packaging policy \
                    set by
Debian Rust team.
"
    );
}
