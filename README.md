Crates.io to Debian
===========================

`debcargo` creates Debian source package from the provided Rust crate. Create
Debian source package from the downloaded crate which follows Rust teams crate
packaging [policy](https://wiki.debian.org/Teams/RustPackaging/Policy).

It is not yet ready for use in Debian, but almost - there are just a few
critical bugs remaining, see TODO.md for details.


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

```shell
$ apt-get build-dep cargo
$ apt-get install libssl-dev libcurl4-gnutls-dev
```


## Examples ##

Following will download and unpack the latest `clap` crate and prepare the
source package.

```shell
$ debcargo package clap
```

Following will download and unpack version `2.25.0` of `clap` crate and prepare
the source package.

```shell
$ debcargo package clap =2.25.0
```

Following will provide additional packaging-specific config for downloading and
packaging latest `clap` crate from the crates.io.

```shell
$ debcargo package --config clap-2/debian/debcargo.toml clap
```

See `debcargo.toml.example` for a sample TOML file.


### Long-term maintenance workflow

New package:

```shell
$ PKG=the-crate-you-want-to-package
$ PKGDIR=$(debcargo deb-src-name $PKG)
$ BUILDDIR=$PWD/build/$PKGDIR
$ PKGCFG=$PKGDIR/debian/debcargo.toml
```

```shell
$ mkdir -p $PKGDIR/debian $BUILDDIR
$ cp /path/to/debcargo.git/debcargo.toml.example $PKGCFG
$ sed -i -e 's/^#overlay =/overlay =/' $PKGCFG
$ touch $PKGDIR/debian/copyright
$ debcargo package --config $PKGCFG --directory $BUILDDIR $PKG
$ ls $PKGDIR/debian/{changelog,copyright.debcargo.hint} # both should have been created
```

```shell
$ cd $PKGDIR
$PKGDIR$ ### update debian/copyright based on debian/copyight.debcargo.hint
$PKGDIR$ ### hack hack hack, deal with any FIXMEs
$PKGDIR$ dch -r -D experimental
$PKGDIR$ cd ..
$ rm -rf $BUILDDIR && debcargo package --config $PKGCFG --directory $BUILDDIR --changelog-ready $PKG
$ git add $PKGDIR
$ git commit -m "New package $PKG, it does A, B and C."
$ dput [etc]
```

Updating a package:

```shell
$ rm -rf $BUILDDIR && debcargo package --config $PKGCFG --directory $BUILDDIR $PKG
$ cd $PKGDIR
$PKGDIR$ git diff
$PKGDIR$ ### examine (any) differences in the hint files, e.g. d/copyright.debcargo.hint
$PKGDIR$ ### apply these differences to the real files, e.g. d/copyright
$PKGDIR$ ### hack hack hack, deal with any FIXMEs
$PKGDIR$ dch -r -D unstable
$PKGDIR$ cd ..
$ rm -rf $BUILDDIR && debcargo package --config $PKGCFG --directory $BUILDDIR --changelog-ready $PKG
$ git add $PKGDIR
$ git commit -m "Updated package $PKG, new changes: D, E, F."
$ dput [etc]
```


## License ##

Debcargo is licensed under `MIT/Apache-2.0`. It is written by `Josh Triplett`
and improved by members of **Debian Rust Maintainers team**
