use super::PkgTest;

struct PkgTestFmtData<'a> {
    feature: &'a str,
    extra_test_args: Vec<&'a str>,
    depends: Vec<String>,
    extra_restricts: Vec<&'a str>,
    expected: &'a str,
}

#[test]
fn pkgtest_fmt_has_no_extra_whitespace() {
    let checks = vec![
        PkgTestFmtData {
            feature: "",
            extra_test_args: Vec::new(),
            depends: Vec::new(),
            extra_restricts: Vec::new(),
            expected: r"Test-Command: /usr/share/cargo/bin/cargo-auto-test crate 1.0 --all-targets
Features: test-name=librust-crate-dev:
Depends: dh-cargo (>= 31), @
Restrictions: allow-stderr, skip-not-installable
",
        },
        PkgTestFmtData {
            feature: "X",
            extra_test_args: vec!["--no-default-features", "--features X"],
            depends: vec!["libfoo-dev".into(), "bar".into()],
            extra_restricts: vec!["flaky"],
            expected: r"Test-Command: /usr/share/cargo/bin/cargo-auto-test crate 1.0 --all-targets --no-default-features --features X
Features: test-name=librust-crate-dev:X
Depends: dh-cargo (>= 31), libfoo-dev, bar, @
Restrictions: allow-stderr, skip-not-installable, flaky
",
        },
    ];

    for check in checks {
        let pkgtest = PkgTest::new(
            "librust-crate-dev",
            "crate",
            check.feature,
            "1.0",
            check.extra_test_args,
            &check.depends,
            check.extra_restricts,
        )
        .unwrap();

        assert_eq!(check.expected, &pkgtest.to_string());
    }
}
