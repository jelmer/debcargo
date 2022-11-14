Rust crates to Debian packages
==============================

`debcargo` is the official tool for packaging Rust crates to be part of the
[Debian](https://www.debian.org/) system.

It creates a Debian source package (`*.dsc`) from a Rust crate that follows
Debian's [general packaging policy](https://www.debian.org/doc/debian-policy/)
as well as the Debian Rust team's [crate packaging
policy](https://wiki.debian.org/Teams/RustPackaging/Policy).


## Features ##

 * Easy to customize, using config files and overlay directories. This includes
   patching or otherwise fixing Rust crates to adhere to Debian policy.
 * Guess copyright information from crate metadata and source code, used to
   suggest appropriate values for `debian/copyright`.
 * Put `FIXME (hint)` strings where it can't detect full information, so user
   can provide an override/overlay or manually fix it.
 * Resulting packages automatically support general functionality available to
   all policy-conforming Debian packages, such as:
   * binaries for [10+ architectures](https://www.debian.org/ports/) are made
     available directly to users, via `apt-get`
   * debugging symbols are placed in a separate binary package, integrating
     with the standard Debian [distribution system for debugging
     symbols](https://wiki.debian.org/HowToGetABacktrace)
   * full system integration with non-Rust software, including cross-language
     dependency resolution
   * cross-compilation support, including automatic resolution and installation
     of non-Rust cross-dependencies, via Debian build tools such as `sbuild`.
 * Determine a crate's full dependency tree (i.e. build order), from both
   Debian packaging and QA perspectives.


## Installation

On Debian systems, `debcargo` can be installed the usual way:

```shell
$ apt-get install debcargo
```

To build locally for development:

```shell
$ apt-get build-dep debcargo
$ cargo build debcargo
```

On non-Debian systems, you can try simply:

```shell
$ cargo build debcargo
```

and fix any build errors that come up, e.g. by installing missing libraries.
Probably, this will include OpenSSL as a dependency of cargo.


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


## Long-term package maintenance

The Debian Rust team uses this tool together with the configs and overlays in
https://salsa.debian.org/rust-team/debcargo-conf/. If you are interested in
contributing, please see that repository for further information and
instructions on how to collaborate with us.


## Building unofficial Debian packages

Debian packaging policy is quite detailed. If you just want to create Debian
binary packages (`*.deb`) without worrying about these policies, you may want
to use other tools instead that ignore and bypass these policies. For example,
`cargo-deb`.

The trade-off is that the resulting packages integrate less well with a Debian
system, and do not integrate at all with the Debian build system, which means
you lose the features described earlier. Furthermore, you will be responsible
for hosting and distributing those packages yourself, outside of the official
Debian distribution infrastructure.


## License ##

Debcargo is licensed under `MIT/Apache-2.0`. It is written by `Josh Triplett`
and `Ximin Luo`, and improved by members of **Debian Rust Maintainers team**
