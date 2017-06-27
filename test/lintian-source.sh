#!/bin/bash
# Run debcargo, do a source-only build, and call lintian on the results.
#
# $ "$0" <crate name>
# $ "$0" </path/to/Cargo.toml>
#
set -e
CRATE="${1:-debcargo}"

cargo build

oldpwd="$PWD"
rm -rf tmp && mkdir -p tmp && cd tmp

run_lintian() {
	crate="$1"
	# The source name is different depending on if it's a non-library crate or not
	changes="$(ls -1 rust-"${crate/_/-}"-*_*_source.changes rust-"${crate/_/-}"_*_source.changes 2>/dev/null || true)"
	lintian -EvIL +pedantic "$changes" || true
}

build_source() {
	crate="$1"
	../target/debug/debcargo package "${crate}"
	cratedir="$(find . -maxdepth 1 -name "rust-${crate/_/-}-*" -type d)"
	( cd "${cratedir}" && dpkg-buildpackage -d -S --no-sign )
}

ghetto_parse_deps() {
	grep -v '^#' "$1" \
	| tr '\n' '\t' \
	| sed -e 's/\t\[/\n[/g' \
	| sed -ne '/^\[.*dependencies\]/p' \
	| tr '\t' '\n' \
	| sed -ne 's/\([^[:space:]]*\)[[:space:]]*=.*/\1/gp'
}

run_x_or_deps() {
	x="$1"
	shift
	case "$x" in
	*Cargo.toml)
		for i in $(cd "$oldpwd" && ghetto_parse_deps "$x"); do
			"$@" "$i";
		done
	;;
	*)
		"$@" "$x";;
	esac
}

for i in "$@"; do run_x_or_deps "$i" build_source; done
for i in "$@"; do run_x_or_deps "$i" run_lintian; done
