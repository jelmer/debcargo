Crates.io to Debian
===========================

`debcargo` creates Debian source package from the provided Rust crate. Create
Debian source package from the downloaded crate which follows Rust teams crate
packaging [policy](https://wiki.debian.org/Teams/RustPackaging/Policy).


## Features ##

 * Uses `cargo` APIs to access source from crates.io.
 * Uses `cargo` API to generate dependency list.
 * Allows to package specific version of crate.
 * Easy to customize using config files and overlay directories.
 * When possible tries to detect copyright information from metadata and actual
   crate source, needed to create `debian/copyright`.
 * Put `FIXME` string where it can't detect full information so user can
   provide an override or manually fix it.
 * Customize package distribution.
 * Provide different name to the binary developed by the crate.
 * Results in a lintian-clean Debian package in most cases.


## Dependencies

For building:

  $ apt-get build-dep cargo
  $ apt-get install libssl-dev libcurl4-gnutls-dev

For running / testing:

  # As above, then:
  $ cargo install cargo-tree
  $ apt-get install dh-cargo lintian

For development:

  # As above, then:
  $ cargo install rustfmt cargo-graph cargo-outdated
  $ cargo graph | dot -T png > graph.png
  $ cargo outdated -R


## Examples ##

Following will download and unpack the latest `clap` crate and prepare the
source package.

`debcargo package clap`

Following will download and unpack version `2.25.0` of `clap` crate and prepare
the source package.

`debcargo package clap =2.25.0`

Following will provide additional packaging-specific config for downloading and
packaging latest `clap` crate from the crates.io.

`debcargo package --config clap-2/debian/debcargo.toml clap`

See `debcargo.toml.example` for a sample TOML file.


## Testing ##

To test the `debcargo` produced packages, you can run the following script.

`tests/sh/integrate.sh crate[s]`

where you can provide a list of crate names or local directories containing
crates, and the script will run debcargo to create a source package (`.dsc`)
and run lintian over it. If you find any issues, please add to the bugs in
TODO.md file.

`tests/sh/integrate.sh -b crate[s]`

will additionally run [sbuild](https://wiki.debian.org/sbuild) on the source
package to build binary Debian packages (`.deb`) and run lintian over that too.

`tests/sh/integrate.sh -r crate[s]`

will run the tests recursively over the listed crate(s) and all of their
transitive dependencies.

See `-h` for other options.


## License ##

Debcargo is licensed under `MIT/Apache-2.0`. It is written by `Josh Tripplet`
and improved by members of **Debian Rust Maintainers team**
