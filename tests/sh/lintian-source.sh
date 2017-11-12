#!/bin/bash
set -e

scriptdir="$(dirname "$0")"

allow_failures="$scriptdir/build-allow-fail"
directory=tmp
failures_file=""
lintian_overrides="$scriptdir/lintian-overrides"
override_dir="$scriptdir/overrides"
recursive=false
update=false
while getopts 'a:d:f:l:o:ruh?' o; do
	case $o in
	a ) allow_failures=$OPTARG;;
	d ) directory=$OPTARG;;
	f ) failures_file=$OPTARG;;
	l ) lintian_overrides=$OPTARG;;
	o ) override_dir=$OPTARG;;
	r ) recursive=true;;
	u ) update=true;;
	h|\? ) cat >&2 <<eof
Usage: $0 [-ru] (<crate name>|<path/to/crate>) [..]

Run debcargo, do a source-only build, and call lintian on the results.

  -h            This help text.
  -a            File that lists crate names to ignore failures for, default:
                $allow_failures.
  -d            Output directory, default: $directory. Warning: this will be
                wiped at the start of the test!
  -f            File to output failed crates in, instead of exiting non-zero.
                Relative paths are taken relative to the output directory.
  -l            Install this file as debian/source/lintian-overrides, to
                override some generic stuff we haven't fixed yet. Default:
                $lintian_overrides.
  -o            Path to overrides directory.
  -r            For crates specified by path, operate on all transitive
                dependencies. Requires cargo-tree.
  -u            With -r, run "cargo update" before calculating dependencies.
                Otherwise, cargo-tree uses the versions listed in Cargo.lock.
eof
		exit 2;;
	esac
done
shift $(expr $OPTIND - 1)

cargo build

oldpwd="$PWD"
allow_failures=$(readlink -f "$allow_failures")
lintian_overrides=$(readlink -f "$lintian_overrides")
override_dir=$(readlink -f "$override_dir")

if [ -z "$NOCLEAN" ]; then
	rm -rf "$directory" && mkdir -p "$directory"
fi
cd "$directory"

allow_fail() {
	local crate="$1"
	local version="$2"
	if grep -qx "${crate}" "${allow_failures}"; then
		echo >&2 "Allowing ${crate} to fail..."
		return 0
	elif [ -n "$version" ] && grep -qx "${crate}-${version}" "${allow_failures}"; then
		echo >&2 "Allowing ${crate}-${version} to fail..."
		return 0
	else
		return 1
	fi
}

run_lintian() {
	local crate="$1"
	local version="$2"
	local cratedir="$crate${version:+-$version}"
	allow_fail "$crate" $version && return 0
	changes="$(cd "$cratedir" && echo $(dpkg-parsechangelog -SSource)_$(dpkg-parsechangelog -SVersion)_source.changes)"
	lintian -EIL +pedantic "$changes" || true
}

build_source() {
	local crate="$1"
	local version="$2"
	local cratedir="$crate${version:+-$version}"
	if [ -f "$override_dir/${crate}_overrides.toml" ]; then
		option="--override ${override_dir}/${crate}_overrides.toml"
	fi

	if ../target/debug/debcargo package --directory $cratedir $option "${crate}" $version; then
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
	( cd "${cratedir}"
	mkdir -p debian/source
	cp "$lintian_overrides" debian/source/lintian-overrides
	dpkg-buildpackage -d -S --no-sign
	)
}

cargo_tree() {(
	cd "$1"
	cargo tree --no-indent -q -a
)}

run_x_or_deps() {
	local x="$1"
	shift
	case "$x" in
	*/*)
		test -d "$x" || local x=$(dirname "$x")
		# 2>/dev/null is needed because of https://github.com/sfackler/cargo-tree/issues/25
		cargo_tree "$x" 2>/dev/null | head -n1 | while read pkg ver extras; do
			echo >&2 "warning: using version $ver from crates.io instead of $extras"
			"$@" "$pkg" "${ver#v}"
		done
		if $recursive; then
			if $update; then
				( cd "$x"; cargo update )
			fi
			cargo_tree "$x" | tail -n+2 | sort -u | while read pkg ver; do
				"$@" "$pkg" "${ver#v}"
			done
		fi
	;;
	*-[0-9]*)
		"$@" "${x%-[0-9]*}" "${x##*-}";;
	*)
		"$@" "$x";;
	esac
}

for i in "$@"; do run_x_or_deps "$i" build_source; done
for i in "$@"; do run_x_or_deps "$i" run_lintian; done
