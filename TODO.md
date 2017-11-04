# TODO #

We manually review output `debcargo` based on this we add things to "Bugs" or
"Features" section below. See **Testing** section in README file for details on
how to run tests.

If a task is completed put a `x` between `[]`.

## Bugs ##

Below list is found by running test/lintian-source.sh with `debcargo`'s
`Cargo.toml` file as input.

 - [ ] libgit2-sys: Unrepresentable dependency version predicate: libz-sys
   Predicate { op: GtEq, major: 0, minor: None, patch: None, pre: [] }
 - [ ] psapi-sys: Unrepresentable dependency version predicate: winapi Predicate
   { op: Wildcard(Major), major: 0, minor: None, patch: None, pre: [] }

## Features ##

- [x] Ability to override detected values in `debian/copyright`.
- [x] Display warnings when detected value is different than override value in
      `debian/copyright`
- [x] Ability to override/add to detected values in `debian/control`
- [x] Display FIXME warning only if there is any FIXME's present in debian folder.
- [ ] Ability to provide ITP number to be closed for `debian/changelog`
- [ ] Refactor `debian/changelog` into its own representation module similar to
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
