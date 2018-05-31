## Release-critical bugs

- Generally, run `tests/sh/integrate.sh -rb ./` and fix any build errors and
  important lintian errors that crop up.

- We don't handle version ranges well yet:

  Cargo.toml dependency x (> a, < b) should convert to
  d/control dependency x-a | x-(a+1) | .. | x-(b-1) | x-b

  Cargo.toml dependency x (> a) should convert to
  d/control dependency x-a | x-(a+1) | .. | x-(max(current version, a+4))

  See ML thread starting 2018-02-18 for details:
  "debcargo update handling alternative build depends"

  Symptoms include sbuild failure like "unsat-dependency: dh-cargo:amd64 (>= 3)"


## Important features

- tests/sh/integrate.sh doesn't handle packages that are not part of
  debcargo's own dependency tree, due to a limitation in cargo-tree

  Ideal solution is to put the functionality inside debcargo and avoid
  cargo-tree completely.

- See debcargo.toml.example and the TODOs listed there

  - allow_prerelease_deps will solve this error for cargo 0.24:

    crates-io: Dependency on prerelease version: error-chain Predicate { op:
    Compatible, major: 0, minor: Some(11), patch: Some(0), pre:
    [AlphaNumeric("rc"), Numeric(2)] }

    The issue doesn't crop up with cargo 0.25+ but might crop up with other
    crates, i.e. it's still something we have to fix in debcargo.

    This would allow us to delete `tests/sh/build-allow-fail`


## Code review ##

by infinity0, for copyninja:

- [x] src/debian/control/ could be collapsed into control.rs, no need to split into
      too many different files, makes things confusing to navigate..

- src/crates.rs needs better names for the methods as well as comments
  explaining what they do. also it mixes up crate deps vs debian deps; code for
  debian deps should be moved into debian/


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
