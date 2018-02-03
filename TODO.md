# TODO #

We manually review output `debcargo`, and based on this we add things to "Bugs"
or "Features" section below. See **Testing** section in README file for details
on how to run tests, e.g. `tests/sh/integrate.sh -rb ./`.

If a task is completed put a `x` between `[]`.


## Code review ##

by infinity0, for copyninja:

- [x] src/debian/control/ could be collapsed into control.rs, no need to split into
      too many different files, makes things confusing to navigate..

- src/crates.rs needs better names for the methods as well as comments
  explaining what they do. also it mixes up crate deps vs debian deps; code for
  debian deps should be moved into debian/


## Important features

- sbuild of aho-corasick fails (with bin = true) because `cargo install` wants
  dev-dependencies (e.g. csv) but debcargo doesn't take them into account yet.

- See debcargo.toml.example and the TODOs listed there

  - allow_prerelease_deps will solve this error for cargo 0.24:

    crates-io: Dependency on prerelease version: error-chain Predicate { op:
    Compatible, major: 0, minor: Some(11), patch: Some(0), pre:
    [AlphaNumeric("rc"), Numeric(2)] }

    It is already fixed in cargo 0.25

    This would allow us to delete `tests/sh/build-allow-fail`

- Run `tests/sh/integrate.sh -rb ./` and fix the lintian errors that occur in
  the Debian binary packages.


## Lower-priority tasks

Minor issues

- fs::rename cannot handle cross-device moves, e.g. if --directory is on a
  different partition from . then debcargo fails

- the ? syntax loses the stack, use Result.expect() to give context, or use
  error-chain instead...

- [ ] globset, ignore, termcolor:
      When generating d/copyright, failed to clone repository
      https://github.com/BurntSushi/ripgrep/tree/master/XXX: unexpected HTTP status code: 404; class=Net (12)

Features for later

- [ ] Integrate `apt-pkg-native` crate to check if the crate or its dependency
      is already in archive and display information.
- [ ] Display first level dependency with equivalent Debian names at the end
      which are not yet packaged in Debian as a information to maintainer.
- [ ] A `dependency` sub-command to print all the dependencies including first
      level and recursive using `cargo` API.
- [ ] Employ `licensecheck` tool to look for license and copyright information.
      Currently we use regex to grep through sources.
