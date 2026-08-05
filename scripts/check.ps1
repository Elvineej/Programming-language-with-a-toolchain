$ErrorActionPreference = "Stop"
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { exit 1 }
cargo clippy --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit 1 }
cargo test --all
if ($LASTEXITCODE -ne 0) { exit 1 }
