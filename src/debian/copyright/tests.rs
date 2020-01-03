use super::get_licenses;

#[test]
fn check_get_licenses() {
    let test_data: &[(&str, &[(&str, bool)])] = &[
        ("AGPL-3.0", &[("AGPL-3.0", true)]),
        ("AcmeCorp-1.0", &[("AcmeCorp-1.0", false)]),
        ("AGPL-3.0-or-later", &[("AGPL-3.0-or-later", true)]),
        ("Apache-2.0/MIT", &[("Apache-2.0", true), ("MIT", true)]),
        ("Apache-2.0 or MIT", &[("Apache-2.0", true), ("MIT", true)]),
        (
            "FooBar-1.0 AND MIT",
            &[("FooBar-1.0", false), ("MIT", true)],
        ),
    ];
    for (name, expected) in test_data {
        let licenses = get_licenses(name).expect("getting licenses failed");
        let found: Vec<_> = licenses
            .iter()
            .map(|l| (l.name.as_str(), !l.text.starts_with("FIXME")))
            .collect();
        assert_eq!(&found[..], &expected[..]);
    }
}
