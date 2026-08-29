#!/usr/bin/env sh
set -e
cargo fmt --all -- --check
# Configuration A — LLVM-free: the front end and the shipped CLI must build and
# pass with no codegen feature and no llvm_sys anywhere in the graph.
cargo clippy -p elya -p elya-cli --all-targets -- -D warnings
cargo test -p elya -p elya-cli
# Configuration B — codegen: the whole workspace, including the execution proof.
cargo clippy --workspace --all-targets --features elya-cli/codegen -- -D warnings
cargo test --workspace --features elya-cli/codegen
