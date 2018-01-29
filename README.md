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


### Long-term maintenance workflow

New package:

 $ PKG=debcargo
 $ TMPDIR=/tmp
 $ debcargo="debcargo package --config $PKG/debian/debcargo.toml --directory $TMPDIR/$PKG"

 $ mkdir -p $PKG/debian
 $ cp /path/to/debcargo.git/debcargo.toml.example $PKG/debian/debcargo.toml
 $ sed -i -e 's/^#overlay =/overlay =/' $PKG/debian/debcargo.toml
 $ touch $PKG/debian/copyright
 $ $debcargo $PKG
 $ ls $PKG/debian/{changelog,copyright.debcargo.hint} # both should have been created

 $ cd $PKG
 $PKG$ # update debian/copyright based on debian/copyight.debcargo.hint
 $PKG$ # hack hack hack, deal with any FIXMEs
 $PKG$ dch -r -D experimental
 $ cd ..
 $ rm -rf $TMPDIR/$PKG && $debcargo --changelog-ready $PKG
 $ git add $PKG
 $ git commit -m "New package $PKG, it does A, B and C."
 $ dput [etc]

Updating a package:

 $ rm -rf $TMPDIR/$PKG && $debcargo $PKG
 $ cd $PKG
 $PKG$ git diff
 $PKG$ # examine (any) differences in the hint files, e.g. d/copyright.debcargo.hint
 $PKG$ # apply these differences to the real files, e.g. d/copyright
 $PKG$ # hack hack hack, deal with any FIXMEs
 $PKG$ dch -r -D unstable
 $ cd ..
 $ rm -rf $TMPDIR/$PKG && $debcargo --changelog-ready $PKG
 $ git add $PKG
 $ git commit -m "New package $PKG, it does A, B and C."
 $ dput [etc]


## License ##

Debcargo is licensed under `MIT/Apache-2.0`. It is written by `Josh Tripplet`
and improved by members of **Debian Rust Maintainers team**
