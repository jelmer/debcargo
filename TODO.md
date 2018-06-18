Generally, run `tests/sh/integrate.sh -rb ./` and fix any build errors and
important lintian errors that crop up.

This document is probably better moved to the GitLab issue tracker on salsa.debian.org

## Release-critical

Our new package naming scheme (without the semver embedded in the name) is
hitting a dpkg limitation, we cannot release until it is fixed:

<infinity0> no, everything is working as intended, it's a problem with the expression syntax
<infinity0> basically I have X build-depends A, B, C. A depends on slab >=0.1 <<0.2, B depends on slab >=0.3 <<0.4, C depends on slab >=0.4 <<0.5 and i have slab-0.1 provides slab=0.1 and slab-0.3 provides slab =0.3 and slab at version 0.4
<infinity0> because i can only express >= and << constraints separately, dpkg is satisfied that slab-0.1 and slab (0.4) satisfy the two constraints (>= 0.3) (<< 4)
<infinity0> what i need to express is that the two constraints must be met by a single package, and that is currently not expressible in the dpkg syntax
<infinity0> rpm had this problem a while back with rust packages and they fixed it by allowing such syntax
<infinity0> guillem: ^
<infinity0> i guess one can additionally code the logic that if a package has both (>=) and (<<) defined for a single package, it should implicitly resolve it to one package rather than two packages (that provide two versions)
<infinity0> DonKult: ^
<infinity0> 02:17:38 <ignatenkobrain> infinity0: well, RPM supports that
<infinity0> 02:18:04 <ignatenkobrain> `(x >= 1.0.0 with x < 2.0.0)` matches exactly ONE package


## Important issues

- See debcargo.toml.example and the TODOs listed there

  - allow_prerelease_deps will solve this error for cargo 0.24:

    crates-io: Dependency on prerelease version: error-chain Predicate { op:
    Compatible, major: 0, minor: Some(11), patch: Some(0), pre:
    [AlphaNumeric("rc"), Numeric(2)] }

    The issue doesn't crop up with cargo 0.25+ but might crop up with other
    crates, i.e. it's still something we have to fix in debcargo.

    This would allow us to delete `tests/sh/build-allow-fail`

- (This is semi-fixed but needs more work.)

  We don't handle version ranges well yet:

  Cargo.toml dependency x (> a, < b) should convert to
  d/control dependency x-a | x-(a+1) | .. | x-(b-1) | x-b

  Cargo.toml dependency x (> a) should convert to
  d/control dependency x-a | x-(a+1) | .. | x-(max(current version, a+4))

  See ML thread starting 2018-02-18 for details:
  "debcargo update handling alternative build depends"

  Symptoms include sbuild failure like "unsat-dependency: dh-cargo:amd64 (>= 3)"


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
