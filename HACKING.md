This file contains information about developing debcargo itself.


## Dependencies

For testing:

```shell
# Install dependencies for building (see README.md), then:
$ apt-get install dh-cargo lintian
```

For development:

```shell
# As above, then:
$ cargo install cargo-outdated
$ cargo outdated -R
```


## Testing

Whenever you make a major change, you should run:

```shell
$ tests/sh/integrate.sh -kbr debcargo exa fd-find ripgrep
```

in order to test it over a few hundred crates. Fix any build errors and
important lintian errors that crop up.

If you make a change that has wide-reaching implications, such as messing with
the dependency logic, do a more thorough test:

```shell
$ tests/sh/integrate.sh -kbR debcargo exa fd-find ripgrep mdbook sccache
```

This will run the test over around a thousand crates. -R runs it over all the
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
$ tests/sh/integrate.sh -kbr crate[s]
```

will run the script recursively over the listed crate(s) and all the transitive
build-dependencies of the generated source packages, in dependency order. This
covers all the dependencies that are needed for entry into Debian Unstable, and
typically covers a few hundred crates. You may want or need to edit or update
some of the overrides in `tests/configs`, to prune old or buggy dependencies.

```shell
$ tests/sh/integrate.sh -kbR crate[s]
```

will run the script recursively over the listed crate(s) and all the transitive
runtime-dependencies of the binary packages, in dependency order. This covers
all the dependencies that are needed for entry into Debian Testing, and
typically covers a few thousand crates. You may want or need to edit or update
some of the overrides in `tests/configs`, to prune old or buggy dependencies.

See `-h` for other options.
