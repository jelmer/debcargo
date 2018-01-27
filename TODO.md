# TODO #

We manually review output `debcargo` based on this we add things to "Bugs" or
"Features" section below. See **Testing** section in README file for details on
how to run tests.

If a task is completed put a `x` between `[]`.

## Bugs ##

Below list is found by running `tests/sh/lintian-source.sh -r ./`.

Known issues:

 - [ ] crates-io: Dependency on prerelease version: error-chain Predicate { op:
   Compatible, major: 0, minor: Some(11), patch: Some(0), pre:
   [AlphaNumeric("rc"), Numeric(2)] }
   - Feature of debcargo and not a bug; fixed in newer versions of crates-io
     (and cargo 0.25)

Minor issues, could leave for later:

 - [ ] globset: When generating d/copyright, failed to clone repository
       https://github.com/BurntSushi/ripgrep/tree/master/globset: unexpected HTTP status code: 404; class=Net (12)
 - [ ] ignore: When generating d/copyright, failed to clone repository
       https://github.com/BurntSushi/ripgrep/tree/master/ignore: unexpected HTTP status code: 404; class=Net (12)
 - [ ] termcolor: When generating d/copyright, failed to clone repository
       https://github.com/BurntSushi/ripgrep/tree/master/termcolor: unexpected HTTP status code: 404; class=Net (12)


## Features ##

- [x] Ability to override detected values in `debian/copyright`.
- [x] Display warnings when detected value is different than override value in
      `debian/copyright`
- [x] Ability to override/add to detected values in `debian/control`
- [x] Display FIXME warning only if there is any FIXME's present in debian folder.
- [ ] Ability to provide ITP number to be closed for `debian/changelog`
- [x] Refactor `debian/changelog` into its own representation module similar to
      `debian/control`.
- [ ] Ability to override debian/compat value to allow easier backporting
- [ ] Integrate `apt-pkg-native` crate to check if the crate or its dependency
      is already in archive and display information.
- [ ] Display first level dependency with equivalent Debian names at the end
      which are not yet packaged in Debian as a information to maintainer.
- [ ] A `dependency` sub-command to print all the dependencies including first
      level and recursive using `cargo` API.
- [ ] Employ `licensecheck` tool to look for license and copyright information.
      Currently we use regex to grep through sources.


## Code review ##

infinity0:

src/debian/control/ could be collapsed into control.rs, no need to split into
too many different files, makes things confusing to navigate..
