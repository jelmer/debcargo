#!/bin/bash
set -e

scriptdir="$(dirname "$0")"

# outputs
directory=tmp
failures_file=""
# inputs
allow_failures="$scriptdir/build-allow-fail"
lintian_overrides="$scriptdir/lintian-overrides"
config_dir="$scriptdir/../configs"
# tweaks
run_lintian=true
run_sbuild=false
keepfiles=false
recursive=false
update=false
extraargs=
use_rec_hack=false

DEB_HOST_ARCH=${DEB_HOST_ARCH:-$(dpkg-architecture -qDEB_HOST_ARCH)}

while getopts 'd:f:a:l:c:bkrux:zh?' o; do
	case $o in
	d ) directory=$OPTARG;;
	f ) failures_file=$OPTARG;;

	a ) allow_failures=$OPTARG;;
	l ) lintian_overrides=$OPTARG;;
	c ) config_dir=$OPTARG;;

	b ) run_sbuild=true;;
	k ) keepfiles=true;;
	r ) recursive=true;;
	u ) update=true;;
	x ) extraargs="$extraargs $OPTARG";;
	z ) use_rec_hack=true;;
	h|\? ) cat >&2 <<eof
Usage: $0 [-ru] (<crate name>|<path/to/crate>) [..]

Run debcargo, do a source-only build, and call lintian on the results.

  -h            This help text.

Options for output:
  -d DIR        Output directory, default: $directory. Warning: this will be
                wiped at the start of the test!
  -f FILE       File to output failed crates in, instead of exiting non-zero.
                Relative paths are taken relative to the output directory.

Options for input:
  -a FILE       File that lists crate names to ignore failures for, default:
                $allow_failures.
  -l FILE       Install this file as debian/source/lintian-overrides, to
                override some generic stuff we haven't fixed yet. Default:
                $lintian_overrides.
  -c DIR        Path to config directory, default: $config_dir.

Options to control running:
  -b            Run sbuild on the resulting dsc package.
  -k            Don't wipe the output directory at the start of the test, and
                don't rebuild a crate if its directory already exists.
  -r            Operate on all transitive dependencies. Requires cargo-tree.
  -u            With -r, run "cargo update" before calculating dependencies.
                Otherwise, cargo-tree uses the versions listed in Cargo.lock.
  -x ARG        Give ARG as an extra argument to debcargo, e.g. like
                -x--copyright-guess-harder.
  -z            Use the slower but more accurate "cargo-tree-deb-rec"
                script to calculate dependencies with -r.
eof
		exit 2;;
	esac
done
shift $(expr $OPTIND - 1)

allow_fail() {
	local crate="$1"
	local version="$2"
	if ! test -f "${allow_failures}"; then
		return 1
	elif grep -qx "${crate}" "${allow_failures}"; then
		echo >&2 "Allowing ${crate} to fail..."
		return 0
	elif [ -n "$version" ] && grep -qx "${crate}-${version}" "${allow_failures}"; then
		echo >&2 "Allowing ${crate}-${version} to fail..."
		return 0
	else
		return 1
	fi
}

run_lintian() {(
	local crate="$1"
	local version="$2"
	local cratedir="$crate${version:+-$version}"
	cd "$directory"

	allow_fail "$crate" $version && return 0

	base="$(cd "$cratedir" && echo $(dpkg-parsechangelog -SSource)_$(dpkg-parsechangelog -SVersion))"
	changes="${base}_source.changes"
	lintian -EIL +pedantic "$changes" || true
	changes="${base}_${DEB_HOST_ARCH}.changes"
	lintian -EIL +pedantic "$changes" || true
)}

DEB_HOST_ARCH=$(dpkg-architecture -q DEB_HOST_ARCH)
if [ -z "$CHROOT" ]; then
	if schroot -i -c "debcargo-unstable-${DEB_HOST_ARCH}-sbuild" >/dev/null 2>&1; then
		CHROOT="debcargo-unstable-${DEB_HOST_ARCH}-sbuild"
	else
		CHROOT=${CHROOT:-unstable-"$DEB_HOST_ARCH"-sbuild}
	fi
fi
run_sbuild() {(
	local crate="$1"
	local version="$2"
	local cratedir="$crate${version:+-$version}"
	cd "$directory"

	allow_fail "$crate" $version && return 0
	base="$(cd "$cratedir" && echo $(dpkg-parsechangelog -SSource)_$(dpkg-parsechangelog -SVersion))"
	dsc="${base}.dsc"
	build="${base}_${DEB_HOST_ARCH}.build"
	changes="${base}_${DEB_HOST_ARCH}.changes"

	if [ -f "$changes" ]; then
		echo >&2 "skipping already-built ${dsc}"
		return 0
	fi

	echo >&2 "sbuild $dsc logging to $build"
	sbuild --arch-all --arch-any --no-run-lintian --resolve-alternatives -c "$CHROOT" -d unstable --extra-package=. $SBUILD_EXTRA_ARGS "$dsc"
)}

build_source() {(
	local crate="$1"
	local version="$2"
	local cratedir="$crate${version:+-$version}"
	cd "$directory"

	if [ -d "$cratedir" ]; then
		echo >&2 "skipping already-built ${cratedir}"
		return 0
	fi

	local deb_src_name="$($debcargo deb-src-name "$crate" "$version")"
	local config="$config_dir/${deb_src_name}/debian/debcargo.toml"
	if [ -f "$config" ]; then
		option="--config $config"
		echo >&2 "using config: $config"
	elif [ "$deb_src_name" != "$($debcargo deb-src-name "$crate" "")" ]; then
		config="$config_dir/old-version/debian/debcargo.toml"
		option="--config $config"
		echo >&2 "using config: $config"
	fi

	echo $debcargo package $extraargs --no-overlay-write-back --directory $cratedir $option "${crate}" $version
	if $debcargo package $extraargs --no-overlay-write-back --directory $cratedir $option "${crate}" $version; then
		:
	else
		local x=$?
		if allow_fail "$crate" $version; then
			return 0
		fi
		echo >&2 "crate failed: $crate $version"
		if [ -n "$failures_file" ]; then
			echo "$crate" $version >> "$failures_file"
			return 0
		else
			return $x
		fi
	fi
	cd "${cratedir}"
	mkdir -p debian/source
	cp "$lintian_overrides" debian/source/lintian-overrides
	dpkg-buildpackage -d -S --no-sign
)}

cargo_tree() {
	"$scriptdir/cargo-tree-any" "$@" --all-targets --no-indent -a
}

cargo_tree_rec() {
	# tac|awk gives us reverse-topological ordering https://stackoverflow.com/a/11532197
	cargo_tree "$@" | tac | awk '!x[$0]++'
}
if $use_rec_hack; then
cargo_tree_rec() {
	local cache="$directory/z-cache_${*/\//_}"
	if [ ! -f "$cache" ]; then
		$scriptdir/cargo-tree-deb-rec "$@" > "$cache"
	fi
	cat "$cache"
}
fi

run_x_or_deps() {
	local x="$1"
	shift
	case "$x" in
	*/*)
		test -d "$x" || x=$(dirname "$x")
		# might give spurious "broken pipe" errors, see @sfackler/cargo-tree#2
		spec=$(cargo_tree "$x" | head -n1)
		tree_args="$x"
		# debcargo does not support packaging path-based crates yet
		echo >&2 "warning: will use latest version from crates.io instead of $x"
		;;
	*-[0-9]*)
		spec="${x%-[0-9]*} ${x##*-}"
		tree_args="${x%-[0-9]*}:${x##*-}"
		;;
	*)
		spec="$x"
		tree_args="$x"
		;;
	esac
	if $update && test -d "$spec"; then
		( cd "$spec"; cargo update )
	fi
	if $recursive; then
		set -o pipefail
		cargo_tree_rec $tree_args | head -n-1 | while read pkg ver extra; do
			"$@" "$pkg" "${ver#v}"
		done
		set +o pipefail
	fi
	echo $spec | while read pkg ver extras; do
		"$@" "$pkg" "${ver#v}"
	done
}

# make all paths absolute so things don't mess up when we switch dirs
allow_failures=$(readlink -f "$allow_failures")
lintian_overrides=$(readlink -f "$lintian_overrides")
config_dir=$(readlink -f "$config_dir")
directory=$(readlink -f "$directory")
scriptdir=$(readlink -f "$scriptdir")

# ensure $directory exists and maybe wipe it
if ! $keepfiles; then
	# don't rm the directory itself, in case it's a symlink
	rm -rf "$directory"/*
fi
mkdir -p "$directory"

cargo build
debcargo="$scriptdir/../../target/debug/debcargo"
test -x $debcargo

for i in "$@"; do run_x_or_deps "$i" build_source; done
if $run_sbuild; then
	if ! schroot -i -c "$CHROOT" >/dev/null; then
		echo >&2 "create the $CHROOT schroot by running e.g.:"
		echo >&2 "  sudo sbuild-createchroot unstable /srv/chroot/$CHROOT http://deb.debian.org/debian"
		echo >&2 "  sudo schroot -c source:$CHROOT -- apt-get -y install dh-cargo"
		echo >&2 "  sudo sbuild-update -udr $CHROOT"
		echo >&2 "See https://wiki.debian.org/sbuild for more details"
		exit 1
	fi
	for i in "$@"; do run_x_or_deps "$i" run_sbuild; done
fi
if $run_lintian; then
	for i in "$@"; do run_x_or_deps "$i" run_lintian; done
fi
