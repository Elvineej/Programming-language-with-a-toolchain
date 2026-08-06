# Elya

An effects-first, statically-typed, natively-compiled language (in progress).
This repository implements **Slice 2**: a **Hindley–Milner type-checked** interpreter
running on a **CEK abstract machine**, over a small subset of the language —
literals, arithmetic, `let`, `if/else`, functions, and `io.println`. Tail calls
run in **bounded continuation depth** (a measured guarantee).

See the design spec: `docs/superpowers/specs/2026-08-05-elya-language-design.md`,
the Slice 1 plan (`.../plans/2026-08-05-elya-slice-1-interpreter.md`), and the
Slice 2 spec + plan (`.../2026-08-06-elya-slice-2-*`).

## Build & run

```sh
cargo build
cargo run -- run examples/01_hello.elya     # prints: Hello, Elya!
cargo run -- check examples/02_arith.elya   # front-end only (parse + resolve + type-check)
```

## Local CI

```sh
sh scripts/check.sh         # fmt + clippy + tests (POSIX / Git Bash)
pwsh scripts/check.ps1      # fmt + clippy + tests (PowerShell)
sh scripts/setup-hooks.sh   # install a pre-push hook that runs check.sh (optional)
```

## Status

**Slice 2 complete.** Elya is a typed interpreter on a CEK machine:

- **Hindley–Milner inference** (Algorithm J): unification with occurs-check,
  let-polymorphism, and SCC-ordered generalization of top-level functions
  (`id : forall a. fn(a) -> a`), with zonked, letter-named type errors
  (`E0400`–`E0403`).
- **CEK abstract machine** — an iterative step loop with a persistent
  continuation; cross-checked to produce identical output to the retained
  tree-walker oracle on every program.
- **Guaranteed tail-call elimination** — 1,000,000-deep self- and
  mutual-tail-recursion run at a pinned constant continuation depth
  (`tests/tce.rs`).

Next: **Slice 3** — algebraic effects & handlers with row-polymorphic effect
inference, Elya's signature feature.
