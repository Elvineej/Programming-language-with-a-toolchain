# Elya

An effects-first, statically-typed, natively-compiled language (in progress).
This repository currently implements **Slice 1**: a tree-walking interpreter for
a small subset of the language — literals, arithmetic, `let`, `if/else`,
functions, and `io.println`.

See the design spec: `docs/superpowers/specs/2026-08-05-elya-language-design.md`
and the Slice 1 plan: `docs/superpowers/plans/2026-08-05-elya-slice-1-interpreter.md`.

## Build & run

```sh
cargo build
cargo run -- run examples/01_hello.elya     # prints: Hello, Elya!
cargo run -- check examples/02_arith.elya   # front-end only (parse + name resolution)
```

## Local CI

```sh
sh scripts/check.sh         # fmt + clippy + tests (POSIX / Git Bash)
pwsh scripts/check.ps1      # fmt + clippy + tests (PowerShell)
sh scripts/setup-hooks.sh   # install a pre-push hook that runs check.sh (optional)
```

This project is **local-only**: no git remote is configured, and nothing is pushed anywhere.

## Status

Slice 1 (this repo) is a working interpreter with the test/diagnostics/architecture
infrastructure later slices build on. Upcoming slices (each with its own spec → plan):
Slice 2 introduces Hindley–Milner types and a CEK abstract machine; Slice 3 adds
algebraic effects and handlers — Elya's signature feature.
