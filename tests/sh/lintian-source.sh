#!/bin/bash
set -e

scriptdir="$(dirname "$0")"

allow_failures="$scriptdir/build-allow-fail"
override_dir="$scriptdir/overrides"
recursive=false
update=false
while getopts 'a:o:ruh?' o; do
	case $o in
	a ) allow_failures=$OPTARG;;
	o ) override_dir=$OPTARG;;
	r ) recursive=true;;
	u ) update=true;;
	h|\? ) cat >&2 <<eof
Usage: $0 [-ru] (<crate name>|<path/to/crate>) [..]

Run debcargo, do a source-only build, and call lintian on the results.

If -r is given, operates on all transitive dependencies (requires cargo-tree).

If -u is given, runs "cargo update" before calculating dependencies. Otherwise,
cargo-tree uses the versions listed in Cargo.lock.
eof
		exit 2;;
	esac
done
shift $(expr $OPTIND - 1)

cargo build

oldpwd="$PWD"
if [ -z "$NOCLEAN" ]; then
	rm -rf tmp && mkdir -p tmp
fi
cd tmp

allow_fail() {
	if ( cd "$oldpwd" && grep -q "${crate}" "${allow_failures}" ); then
		echo >&2 "Allowing ${crate} to fail..."
		return 0
	else
		return 1
	fi
}

run_lintian() {
	crate="$1"
	version="$2"
	allow_fail "$crate" && return 0
	# The source name is different depending on if it's a non-library crate or not
	changes="$(ls -1 rust-"${crate/_/-}"-*_*_source.changes rust-"${crate/_/-}"_*_source.changes 2>/dev/null || true)"
	lintian -EvIL +pedantic "$changes" || true
}

build_source() {
	crate="$1"
	version="$2"
	if [ -z "$version" ]; then
		cratedir="$crate"
	else
		cratedir="$crate-$version"
	fi
	if [ -f "$override_dir/${crate}_overrides.toml" ]; then
		option="--override ${override_dir}/${crate}_overrides.toml"
	fi

	if allow_fail "$crate"; then
	../target/debug/debcargo package --directory $cratedir $option "${crate}" $version || return 0
	else
	../target/debug/debcargo package --directory $cratedir $option "${crate}" $version
	fi
	( cd "${cratedir}" && dpkg-buildpackage -d -S --no-sign )
}

cargo_tree() {(
	cd "$1"
	cargo tree --no-indent -q -a
)}

run_x_or_deps() {
	x="$1"
	shift
	case "$x" in
	*/*)
		test -d "$x" || x=$(dirname "$x")
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
	*)
		"$@" "$x";;
	esac
}

for i in "$@"; do run_x_or_deps "$i" build_source; done
for i in "$@"; do run_x_or_deps "$i" run_lintian; done
