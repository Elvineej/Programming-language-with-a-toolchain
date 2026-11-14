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
# Configuration C — boards (spec 2026-11-13-elya-boards): LLVM's ARM back end,
# and programs run on QEMU's Cortex-M3 board. Where the tools are; the `boards`
# feature needs an LLVM with the ARM back end (the maintainer's vcpkg one has
# x86 only, so check.ps1 has no configuration C).
if command -v qemu-system-arm >/dev/null 2>&1 && command -v arm-none-eabi-gcc >/dev/null 2>&1; then
  cargo clippy -p elya-codegen --all-targets --features boards -- -D warnings
  cargo test -p elya-codegen --features boards --test native_boards
else
  echo "check.sh: configuration C (boards) not run: no qemu-system-arm or arm-none-eabi-gcc on PATH"
fi
