extern crate debcargo;

use std::path::Path;
use debcargo::overrides::parse_overrides;


#[test]
fn get_source_override() {
    let filepath = Path::new("tests/clap_override.toml");

    let overrides  = parse_overrides(&filepath);
    assert!(overrides.is_ok());

    let overrides = overrides.unwrap();
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
    let section = overrides.section();
    assert!(section.is_some());
    assert_eq!(section.unwrap(), "misc");

    println!("{:?}", overrides);
    let sd = overrides.summary_description_for("debcargo");
    assert!(sd.is_some());

    if let Some((s,d)) = sd {
        assert_eq!(s,
                   "Tool to create Debian package from Rust crate");
        assert_eq!(d, "\
This package provides debcargo a tool to create Debian source package from Rust
crate. The package created by this tool is as per the packaging policy set by
Debian Rust team.
");
    }
}
