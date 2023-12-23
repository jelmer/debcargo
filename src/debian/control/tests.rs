use super::PkgTest;

struct PkgTestFmtData<'a> {
    feature: &'a str,
    extra_test_args: Vec<&'a str>,
    depends: Vec<String>,
    extra_restricts: Vec<&'a str>,
}

#[test]
fn pkgtest_fmt_has_no_extra_whitespace() {
    let checks = vec![
        PkgTestFmtData {
            feature: "",
            extra_test_args: Vec::new(),
            depends: Vec::new(),
            extra_restricts: Vec::new(),
        },
        PkgTestFmtData {
            feature: "X",
            extra_test_args: vec!["--no-default-features", "--features X"],
            depends: vec!["libfoo-dev".into(), "bar".into()],
            extra_restricts: vec!["flaky"],
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

        for ln in pkgtest.to_string().lines() {
            let trimmed = ln.trim_end();
            assert_eq!(trimmed, ln);
        }
    }
}
