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


## Testing ##

To test the `debcargo` produced packages, you can run the following script.

```shell
$ tests/sh/integrate.sh crate[s]
```

where you can provide a list of crate names or local directories containing
crates, and the script will run debcargo to create a source package (`.dsc`)
and run lintian over it. If you find any issues, please add to the bugs in
TODO.md file.

```shell
$ tests/sh/integrate.sh -b crate[s]
```

will additionally run [sbuild](https://wiki.debian.org/sbuild) on the source
package to build binary Debian packages (`.deb`) and run lintian over that too.

```shell
$ tests/sh/integrate.sh -r crate[s]
```

will run the tests recursively over the listed crate(s) and all of their
transitive dependencies.

See `-h` for other options.
