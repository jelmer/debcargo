#!/bin/bash
set -e
cargo build
rm -rf rust-*
target/debug/debcargo package "$@"
nano rust-${1/_/-}-*/debian/${file:-control}
