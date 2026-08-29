# Elya

An effects-first, statically-typed, natively-compiled language (in progress).
Elya is a **Hindley–Milner type-checked** language with **row-polymorphic
algebraic effects**, running on a **CEK abstract machine**. Tail calls execute
in **bounded continuation depth** (a measured guarantee). The current frontier
is an **affine resource** discipline that turns "don't use this twice" from a
best-effort lint into a type-checked guarantee.

The language is grown in reviewed vertical slices: brainstorm → spec → plan →
implement. Design specs and implementation plans live under
`docs/superpowers/specs/` and `docs/superpowers/plans/` — start with the
language design (`.../2026-08-05-elya-language-design.md`).

## Build & run

The repository is a Cargo workspace: `elya` (the front end, deliberately
LLVM-free), `elya-codegen` (Core → LLVM → native object; the only crate that
links `llvm_sys`), and `elya-cli` (the `elya` binary).

```sh
cargo build -p elya-cli                                 # the compiler; no LLVM in this graph
cargo run -p elya-cli -- run examples/01_hello.elya     # prints: Hello, Elya!
cargo run -p elya-cli -- check examples/02_arith.elya   # front-end only (parse + resolve + type-check)
```

Native compilation is opt-in behind `--features codegen`, so a default build
needs no LLVM at all. With it, `elya build` compiles a program to a real native
executable (LLVM object + `clang` link):

```sh
cargo run -p elya-cli --features codegen -- build prog.elya   # -> prog(.exe)
./prog                                                        # runs natively
```

Output defaults to the input stem plus the platform executable suffix; `-o <out>`
overrides it. As of Slice 5b-1 the backend covers the arithmetic subset only
(`Int` literals, `let`, `+ - *`), so anything outside it is rejected by name
rather than mis-compiled — the tree-walking `elya run` remains the full language.

## Local CI

```sh
sh scripts/check.sh         # fmt + clippy + tests, both feature configurations (POSIX / Git Bash)
pwsh scripts/check.ps1      # fmt + clippy + tests, both feature configurations (PowerShell)
sh scripts/setup-hooks.sh   # install a pre-push hook that runs check.sh (optional)
```

## Status

Elya is a typed language on a CEK machine, built up through these slices:

- **Hindley–Milner inference** (Algorithm J): unification with occurs-check,
  let-polymorphism, and SCC-ordered generalization of top-level functions
  (`id : forall a. fn(a) -> a`), with zonked, letter-named type errors
  (`E0400`–`E0403`).
- **CEK abstract machine** — an iterative step loop with a persistent
  continuation; cross-checked to produce identical output to the retained
  tree-walker oracle on every program.
- **Guaranteed tail-call elimination** — 1,000,000-deep self- and
  mutual-tail-recursion run at a pinned constant continuation depth
  (`tests/tce.rs`), and it holds through pattern matches, closures, and
  effect handlers.
- **Algebraic effects & handlers** with **row-polymorphic** effect inference:
  ambient effect rows, `perform`/`handle`/`resume`, and one-shot continuation
  capture on the CEK machine. Effects are discharged statically — unhandled
  effects (`E0420`), purity violations (`E0421`), and row mismatches (`E0423`)
  are compile errors.
- **Parametric ADTs & pattern matching** with an **exhaustiveness** checker
  (`E0430`–`E0433`), including nested and literal patterns.
- **Generic (parametric) effects** and a parameter-passing **`State`** effect
  that runs in bounded continuation depth.
- **Effect resumption discipline** — an effect declared `multi` may resume its
  continuation more than once; a handler that resumes a one-shot effect
  multiply is rejected (`E0427`).

### Frontier: affine resources

The signature in-progress feature. A type declared `linear` makes its values
**affine** — usable at most once:

```elya
linear type File { File }

fn read(f) { match f { File -> "contents" } }

pub fn main() {
  let f = File
  let _ = read(f)
  io.println(read(f))   // error: E0428 — affine value `f` used more than once
}
```

Use-at-most-once is enforced today (`E0428`, `tests/affine.rs`). The payoff —
statically forbidding an affine value from being **captured into a
multiply-resuming continuation** (the double-free that a tracing GC cannot
catch) — is landing next as `E0429`.

The guarantee is scoped honestly: it is **intra-function local** and models
create-and-consume within a single function body. Two deliberate
over-/under-approximations (a callee that duplicates an affine parameter; whole-
body branch-insensitivity) are tracked obligations to be tightened when the
analysis goes inter-procedural — see the Slice 4d-2 design spec.
