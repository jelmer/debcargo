This file contains information about developing debcargo itself.


## Dependencies

For testing:

```shell
# Install dependencies for building (see README.md), then:
$ cargo install cargo-tree # use https://github.com/infinity0/cargo-tree
$ apt-get install dh-cargo lintian
```

For development:

```shell
# As above, then:
$ cargo install rustfmt cargo-graph cargo-outdated
$ cargo graph | dot -T png > graph.png
$ cargo outdated -R
```


## Testing

Whenever you make a major change, you should run:

  tests/sh/integrate.sh -kbr debcargo exa fd-find ripgrep

in order to test it over a few hundred crates. Fix any build errors and
important lintian errors that crop up.

If you make a change that has wide-reaching implications, such as messing with
the dependency logic, do a more thorough test:

  tests/sh/integrate.sh -kbR debcargo exa fd-find ripgrep mdbook sccache

This will run it over a few thousand crates. -R will run the test over all the
transitive dependencies of all the binary packages, which is needed for entry
into Debian Testing. This is wider than -r, which runs the test over all the
transitive build-dependencies of the source package, which is needed for entry
into Debian Unstable.

### Details

To test the `debcargo` produced packages, you can run the following script.

```shell
$ tests/sh/integrate.sh crate[s]
```

where you can provide a list of crate names or local directories containing
crates, and the script will run debcargo to create a source package (`.dsc`)
and run lintian over it. If you find any issues, please add to the bugs in
TODO.md file.

```shell
$ tests/sh/integrate.sh -kb crate[s]
```

will additionally run [sbuild](https://wiki.debian.org/sbuild) on the source
package to build binary Debian packages (`.deb`) and run lintian over that too.
It will automatically pick up any extra .debs you already have in the output
directory, if they are dependencies of what you're building. The `-k` flag
tells the script not to wipe the directory before it does anything else.

```shell
$ tests/sh/integrate.sh -r crate[s]
```

will run the script recursively over the listed crate(s) and all of their
transitive dependencies, in dependency-order. However, sometimes this does not
work properly because of sfackler/cargo-tree#34 and some dependencies will get
skipped. In this case you can giving the `-z` flag which uses a slower but more
accurate (for Debian) method for resolving dependencies. (This is noticeable
mostly when building with `-kbr` because sbuild will fail complaining about
missing dependencies.)

See `-h` for other options.
