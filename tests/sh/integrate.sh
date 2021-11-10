#!/bin/bash
set -e

scriptdir="$(dirname "$0")"

# outputs
directory=tmp
failures_file=""
# inputs
allow_failures="$scriptdir/build-allow-fail"
lintian_suppress_tags="$scriptdir/lintian-suppress-tags"
config_dir="$scriptdir/../configs"
# tweaks
run_lintian=true
run_sbuild=false
keepfiles=false
resolve=
extraargs=

export DEBCARGO_TESTING_IGNORE_DEBIAN_POLICY_VIOLATION=1
export DEBCARGO_TESTING_RUZT=1

export DEB_HOST_ARCH=${DEB_HOST_ARCH:-$(dpkg-architecture -qDEB_HOST_ARCH)}

while getopts 'd:f:a:l:c:bkrRux:zh?' o; do
	case $o in
	d ) directory=$OPTARG;;
	f ) failures_file=$OPTARG;;

	a ) allow_failures=$OPTARG;;
	c ) config_dir=$OPTARG;;

	b ) run_sbuild=true;;
	k ) keepfiles=true;;
	r ) resolve=SourceForDebianUnstable;;
	R ) resolve=BinaryAllForDebianTesting;;
	x ) extraargs="$extraargs $OPTARG";;
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
  -c DIR        Path to config directory, default: $config_dir.

Options to control running:
  -b            Run sbuild on the resulting dsc package.
  -k            Don't wipe the output directory at the start of the test, and
                don't rebuild a crate if its directory already exists.
  -r            Operate on all transitive build-dependencies of the source
                package, needed for entry into Debian Unstable.
  -R            Operate on all transitive dependencies of the binary packages,
                needed for entry into Debian Testing.
  -x ARG        Give ARG as an extra argument to debcargo, e.g. like
                -x--copyright-guess-harder.
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

shouldbuild() {
	local dst="$1"
	local src="$2"
	test ! -e "$dst" -o "$src" -nt "$dst"
}

changelog_pkgname() {(
	local cratedir="$1"
	cd "$cratedir"
	# dpkg-parsechangelog is really slow when dealing with hundreds of crates
	#echo $(dpkg-parsechangelog -SSource)_$(dpkg-parsechangelog -SVersion)
	head -n1 debian/changelog | sed -nre 's/^(\S*) \((\S*)\).*/\1_\2/gp'
)}

run_lintian() {(
	local crate="$1"
	local version="$2"
	local cratedir="$crate${version:+-$version}"
	cd "$directory"

	allow_fail "$crate" $version && return 0

	local base="$(changelog_pkgname "$cratedir")"
	local out="${base}.lintian.out"

	if ! ( shouldbuild "$out" "${base}_source.changes" \
	    || shouldbuild "$out" "${base}_${DEB_HOST_ARCH}.changes" ); then
		echo >&2 "skipping already-linted ${base}_*.changes in ${out}"
		return 0
	fi

	echo >&2 "running lintian for ${base} into ${out}"
	rm -f "$out" "${out}.tmp"
	changes="${base}_source.changes"
	lintian --suppress-tags-from-file "$lintian_suppress_tags" -EIL +pedantic "$changes" | tee -a "${out}.tmp"
	changes="${base}_${DEB_HOST_ARCH}.changes"
	lintian --suppress-tags-from-file "$lintian_suppress_tags" -EIL +pedantic "$changes" | tee -a "${out}.tmp"
	mv "${out}.tmp" "$out"
)}

if [ -z "$CHROOT" ]; then
	if schroot -i -c "debcargo-unstable-${DEB_HOST_ARCH}-sbuild" >/dev/null 2>&1; then
		CHROOT="debcargo-unstable-${DEB_HOST_ARCH}-sbuild"
	else
		CHROOT=${CHROOT:-unstable-"$DEB_HOST_ARCH"-sbuild}
	fi
fi
GPG_KEY_ID="Debcargo Integration Test"
run_sbuild() {(
	local crate="$1"
	local version="$2"
	local cratedir="$crate${version:+-$version}"
	cd "$directory"

	allow_fail "$crate" $version && return 0
	local base="$(changelog_pkgname "$cratedir")"
	local dsc="${base}.dsc"
	local build="${base}_${DEB_HOST_ARCH}.build"
	local changes="${base}_${DEB_HOST_ARCH}.changes"

	if ! shouldbuild "$changes" "$dsc"; then
		echo >&2 "skipping already-built ${dsc} in ${changes}"
		return 0
	fi

	if [ ! -f "signing-key.gpg" ]; then
		mkdir -p "$PWD/gpg"
		chmod 700 "$PWD/gpg"
		GNUPGHOME="$PWD/gpg" gpg --batch --pinentry-mode=loopback --passphrase "" --quick-gen-key "$GPG_KEY_ID"
		GNUPGHOME="$PWD/gpg" gpg --batch --export "$GPG_KEY_ID" > signing-key.gpg
	fi

	# Update the local repo
	apt-ftparchive packages . > Packages
	apt-ftparchive release . > Release
	GNUPGHOME="$PWD/gpg" gpg --batch -a --detach-sign -u "$GPG_KEY_ID" -o Release.gpg --yes Release
	# We use --build-dep-resolver=aspcud as both apt/aptitude fail to resolve
	# certain complex dependency situations e.g. bytes-0.4. For our official
	# Debian rust packages we patch those crates to have simpler dependencies;
	# but we don't want to maintain those patches for this integration test.
	# We also pass criteria to minimise the Rust packages we take from the
	# Debian archive, and maximise the ones generated by this test.
	echo >&2 "sbuild $dsc logging to $build"
	sbuild --arch-all --arch-any --no-run-lintian --build-dep-resolver=aspcud \
	  --aspcud-criteria="-removed,-changed,-new,+count(solution,APT-Release:=/o=sbuild-build-depends-archive/),-count(solution,APT-Release:=/o=Debian/)" \
	  --extra-repository="deb file:$(readlink -f "$directory") ./" --extra-repository-key="$PWD/signing-key.gpg" \
	  -c "$CHROOT" -d unstable $SBUILD_EXTRA_ARGS "$dsc"
)}

build_source() {(
	local crate="$1"
	local version="$2"
	local cratedir="$crate${version:+-$version}"
	cd "$directory"

	if [ -d "$cratedir" ]; then
		if [ -f "$cratedir/debian/changelog" ]; then
			local base="$(changelog_pkgname "$cratedir")"
			if ! shouldbuild "${base}_source.buildinfo" "$cratedir/debian/changelog"; then
				echo >&2 "skipping already-built ${cratedir}"
				return 0
			fi
		fi
		rm -rf "$cratedir"
	fi

	local deb_src_name="$($debcargo deb-src-name "$crate" "$version")"
	local config="$config_dir/${deb_src_name}/debian/debcargo.toml"
	if [ -f "$config" ]; then
		option="--config $config"
		if ! grep -q 'semver_suffix = true' "$config"; then
			echo >&2 "bad config: $config must contain \"semver_suffix = true\""
			return 1
		fi
		echo >&2 "using config: $config"
	elif [ "$deb_src_name" != "$($debcargo deb-src-name "$crate" "")" ]; then
		config="$config_dir/old-version/debian/debcargo.toml"
		option="--config $config"
		echo >&2 "using config: $config"
	fi

	if ( set -x; $debcargo package $extraargs --no-overlay-write-back --directory $cratedir $option "${crate}" $version ); then
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
	dpkg-buildpackage -d -S --no-sign
)}

cargo_tree_rec() {
	local resolve="$1"
	shift
	local cache="$directory/z.${*/\//_}.$resolve.list"
	if [ ! -f "$cache" ]; then
		"$debcargo" build-order --resolve-type "$resolve" \
		  --config-dir "${config_dir}" "$@" > "$cache.tmp"
		mv "$cache.tmp" "$cache"
	fi
	cat "$cache"
}

run_x_or_deps() {
	local x="$1"
	shift
	case "$x" in
	*-[0-9]*)
		spec="${x%-[0-9]*} ${x##*-}"
		tree_args="${x%-[0-9]*}:${x##*-}"
		;;
	*)
		spec="$x"
		tree_args="$x"
		;;
	esac
	if [ -n "$resolve" ]; then
		set -o pipefail
		cargo_tree_rec "$resolve" $tree_args | while read pkg ver extra; do
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
lintian_suppress_tags=$(readlink -f "$lintian_suppress_tags")
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

for i in "$@"; do run_x_or_deps "$i" true; done
for i in "$@"; do run_x_or_deps "$i" build_source; done
# sudo schroot -c source:debcargo-unstable-amd64-sbuild -- sh -c 'echo "deb [allow-insecure=yes] file:/home/infinity0/var/lib/rust/debcargo-tmp ./" > /etc/apt/sources.list.d/local-debcargo-integration-test.list'
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
