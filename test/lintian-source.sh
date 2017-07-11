#!/bin/bash
# Run debcargo, do a source-only build, and call lintian on the results.
#
# $ "$0" <crate name>
# $ "$0" </path/to/Cargo.toml>
#
set -e
ALLOW_FAIL="${ALLOW_FAIL:-test/build-allow-fail}"
OVERRIDE_DIR="${OVERRIDE_DIR:-test/overrides}"

if [ -d "$OVERRIDE_DIR" ]; then
    OVERRIDE_EXISTS=1
fi

cargo build

oldpwd="$PWD"
if [ -z "$NOCLEAN" ]; then
	rm -rf tmp && mkdir -p tmp
fi
cd tmp

allow_fail() {
	if ( cd "$oldpwd" && grep -q "${crate}" "${ALLOW_FAIL}" ); then
		echo >&2 "Allowing ${crate} to fail..."
		return 0
	else
		return 1
	fi
}

run_lintian() {
	crate="$1"
	allow_fail "$crate" && return 0
	# The source name is different depending on if it's a non-library crate or not
	changes="$(ls -1 rust-"${crate/_/-}"-*_*_source.changes rust-"${crate/_/-}"_*_source.changes 2>/dev/null || true)"
	lintian -EvIL +pedantic "$changes" || true
}

build_source() {
    crate="$1"
    if [ -n "$OVERRIDE_EXISTS" ]; then
        if [ -f "$OVERRIDE_DIR/${crate}_overrides.toml" ]; then
            option="--override ${OVERRIDE_DIR}/${crate}_overrides.toml"
        fi
    fi

    if allow_fail "$crate"; then
	../target/debug/debcargo package $option "${crate}" || return 0
    else
	../target/debug/debcargo package $option "${crate}"
    fi
    cratedir="$(find . -maxdepth 1 -name "rust-${crate/_/-}-*" -type d)"
    ( cd "${cratedir}" && dpkg-buildpackage -d -S --no-sign )
}

ghetto_parse_deps() {
	grep -v '^#' "$1" \
	| tr '\n' '\t' \
	| sed -e 's/\t\[/\n[/g' \
	| sed -ne '/^\[.*dependencies\]/p' \
	| tr '\t' '\n' \
	| sed -ne 's/\([^[:space:]]*\)[[:space:]]*=.*/\1/gp' \
	| sort -u
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
