#!/usr/bin/env sh
set -e
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
