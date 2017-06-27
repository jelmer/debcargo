#!/bin/sh
set -e
cargo build

rm -rf tmp && mkdir -p tmp && cd tmp
../target/debug/debcargo package debcargo
( cd rust-debcargo-*
dpkg-buildpackage -d -S --no-sign
lintian -EvIL +pedantic ../rust-debcargo_*_source.changes
)
