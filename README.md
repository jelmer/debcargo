Crates.io to Debian
===========================

`debcargo` creates Debian source package from the provided Rust crate. Create
Debian source package from the downloaded crate which follows Rust teams crate
packaging [policy](https://wiki.debian.org/Teams/RustPackaging/Policy).


## Features ##

 * Package specific versions of crates from crates.io.
 * Easy to customize, using config files and overlay directories.
 * Tries to auto-detect copyright information from metadata and actual
   crate source, used to guess appropriate values for `debian/copyright`.
 * Put `FIXME (hint)` strings where it can't detect full information, so user can
   provide an override/overlay or manually fix it.
 * Results in a lintian-clean Debian package in most cases.
 * Packages can be cross-compiled by `sbuild` assuming the crate doesn't
   include anything that breaks it (such as arch-specific build.rs scripts).


## Dependencies

For building:

```shell
$ apt-get build-dep cargo
$ apt-get install libssl-dev libcurl4-gnutls-dev quilt
$ cargo build debcargo
```


## Examples ##

To download and unpack the latest `clap` crate and prepare the source package:

```shell
$ debcargo package clap
```

To download and unpack version `2.25.0` of `clap` crate and prepare the source package:

```shell
$ debcargo package clap =2.25.0
```

To provide additional packaging-specific config for downloading and packaging
latest `clap` crate from crates.io:

```shell
$ debcargo package --config clap-2/debian/debcargo.toml clap
```

See `debcargo.toml.example` for a sample TOML file.


### Long-term maintenance workflow

See https://salsa.debian.org/rust-team/debcargo-conf/blob/master/README.rst
for an example.


## License ##

Debcargo is licensed under `MIT/Apache-2.0`. It is written by `Josh Triplett`
and improved by members of **Debian Rust Maintainers team**
