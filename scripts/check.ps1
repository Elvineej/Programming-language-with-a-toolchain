$ErrorActionPreference = "Stop"
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { exit 1 }
# Configuration A — LLVM-free: the front end and the shipped CLI must build and
# pass with no codegen feature and no llvm_sys anywhere in the graph.
cargo clippy -p elya -p elya-cli --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit 1 }
cargo test -p elya -p elya-cli
if ($LASTEXITCODE -ne 0) { exit 1 }
# Configuration B — codegen: the whole workspace, including the execution proof.
cargo clippy --workspace --all-targets --features elya-cli/codegen -- -D warnings
if ($LASTEXITCODE -ne 0) { exit 1 }
cargo test --workspace --features elya-cli/codegen
if ($LASTEXITCODE -ne 0) { exit 1 }
