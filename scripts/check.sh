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
# Configuration C — boards (specs 2026-11-13-elya-boards, 2026-11-14-elya-riscv-board):
# LLVM's ARM and RISC-V back ends, and programs run on QEMU's Cortex-M3 and RISC-V
# boards. The `boards` feature needs an LLVM with those back ends (the
# maintainer's vcpkg one has x86 only, so check.ps1 has no configuration C). A
# machine without the boards' tools fails here rather than passing unchecked;
# ELYA_BOARDS=skip says so on purpose.
if [ "${ELYA_BOARDS:-}" = skip ]; then
  echo "check.sh: configuration C (boards) skipped: ELYA_BOARDS=skip"
else
  for tool in qemu-system-arm qemu-system-riscv32 arm-none-eabi-gcc riscv64-unknown-elf-gcc; do
    if ! command -v "$tool" >/dev/null 2>&1; then
      echo "check.sh: configuration C (boards) needs $tool on PATH (or ELYA_BOARDS=skip)" >&2
      exit 1
    fi
  done
  cargo clippy -p elya-codegen --all-targets --features boards -- -D warnings
  cargo test -p elya-codegen --features boards --test native_boards
fi
