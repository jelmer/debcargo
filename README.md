Crates.io to Debian
===========================

`debcargo` creates Debian source package from the provided Rust crate. Create
Debian source package from the downloaded crate which follows Rust teams crate
packaging [policy](https://wiki.debian.org/Teams/RustPackaging/Policy).


## Features ##

 * Uses `cargo` APIs to access source from crates.io.
 * Uses `cargo` API to generate dependency list.
 * Allows to package specific version of crate.
 * easy to customize using override files.
 * When possible tries to detect copyright information from metadata and actual
   crate source, needed to create `debian/copyright`.
 * Put `FIXME` string where it can't detect full information so user can provide
   overrides or manually fix it.
 * Customize package distribution.
 * Provide different name to the binary developed by the crate.
 * With proper override values creates lintian clean Debian source package.


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

Following will provide override for downloading and packaging latest `clap`
crate from the crates.io.

`debcargo package --override clap_overrides.toml clap`


## Overrides ##

Overrides should be provided in form of `TOML` file. The file should be provided
to the `package` sub-command of `debcargo` using `--override` switch. Currently
allowed overrides are

 * source section of `debian/control` can have following overrides
   * policy - Debian Standards-Version to use. By default debcargo uses latest
     policy version
   * homepage - Can override or providing missing homepage for crate
   * section - By default library crates get section values as `rust` where as if
     crate is binary it will get a `FIXME`. This value can be used to avoid that.
   * build_depends - This is TOML list value which appends to the default
     `Build-Depends` field created
     by `debcargo`. This should be used when crate needs external development
     headers for its building (eg. `libssl-dev` needed by cargo and debcargo).
 * package section  of `debian/control` can have following overrides. These
   override should be specified per package basis.
   * summary - this will be short description for package. By default `debcargo`
     will try to use description from `Cargo.toml` but some times this may lead
     to meaningless, weird short description.
   * description - this will be long description for the package.
   * depends - A TOML list value, providing additional non-crate dependency. For
     now debcargo does not use it.
  * copyright overrides allow overriding values in `debian/copyright`. Some
    values are global and others are Files section overrides.
     * source - `Source` field value in `debian/copyright`. By default
       `repository` key from `Cargo.toml` is used, but at times this key will be
       missing and we can use this override to handle those cases.
     * ignore - This is a TOML list containing files which should be ignored
       while scanning for copyright. This can be used to skip files like COPYING
       LICENESE etc.
     * files - Is a TOML hash map with file name as key and containing following keys.
       * copyright - A list of copyright string in following format
       `2016, Some Author <some.email@domain>`. Though there is no specific
       validation for format.
       * license - A valid license for the file.

Below is a sample TOML value

```toml
    [source]
    policy = "4.0.0"
    homepage = "https://clap.rs"

    [packages."librust-clap-2-dev"]

    summary = "Simple, efficient and full featured Command line argument parser - source"
    description = """
    clap is used to parse and validate string of command line arguments provided by
    user at runtime. It provides help and version flags by default and additionally
    provide help subcommands in addition to traditional flags.
    This package provides clap with following default features.
     * suggestions: provides did you mean suggestions on typos
     * color: turns on colored error messages.
     * wrap_help: Wrap the help at actual terminal width when available.
    """

    [copyright]
    source = "https://github.com/clap/clap.rs"
    ignore = ["LICENSE-MIT"]
    [copyright.files."*"]
    copyright = ["2015-2016, Kevin B. Knapp <kbknapp@gmail.com"]
    license = "MIT"

    [copyright.files."debian/*"]
    copyright = ["2017, Vasudev Kamath <vasudev@copyninja.info"]
    license = "MIT"

    [copyright.files."./LICENSE-MIT"]
    copyright = ["2015-2016, Kvin B. Knapp <kbknapp@gmail.com>"]
    license = "MIT"

    [copyright.files."./src/args/arg_matches.rs"]
    copyright = ["2015, The Rust Project Developers"]
    license = "MIT"
```


## Testing ##

To test the `debcargo` produced source you can run the following script.

`tests/sh/lintian-source.sh crate[s]`

Where you can provide list of crate and the script builds the package source and
prepapres source only changes and runs lintian over it. If you find any issue
with source please feel free to add the bugs to TODO.md file.

It is also possible to provide a `Cargo.toml` to this script and it runs
`debcargo` on individual dependencies listed in the `Cargo.toml`. This can be
done as follows

`tests/sh/lintian-source.sh` Cargo.toml

It is also possible to provide override files for test script. You can put
override files under `tests/sh/overrides` directory and name of the file should be
*<cratename>_overrides.toml*. You can put it in arbitrary directory also, in
that case you need to give the `-o` option.

`tests/sh/lintian-source.sh -o /path/to/override/dir crate[s]`


## License ##

Debcargo is licensed under `MIT/Apache-2.0`. It is written by `Josh Tripplet`
and improved by members of **Debian Rust Maintainers team**
