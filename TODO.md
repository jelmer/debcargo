See https://salsa.debian.org/groups/rust-team/-/issues

Whenever you make a major change, you can run:

  tests/sh/integrate.sh -krbz debcargo mdbook ripgrep exa sccache

in order to test it over a few hundred crates. Fix any build errors and
important lintian errors that crop up.

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
