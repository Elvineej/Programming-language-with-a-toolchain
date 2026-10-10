# Elya

An effects-first, statically-typed, natively-compiled language (in progress).
Elya is a **Hindley–Milner type-checked** language with **row-polymorphic
algebraic effects**, running on a **CEK abstract machine**. Tail calls execute
in **bounded continuation depth** (a measured guarantee). An **affine resource**
discipline turns "don't use this twice" from a best-effort lint into a
type-checked guarantee. The current frontier is the **native back end**, grown
to meet the evaluator feature by feature under a differential test that runs
every program both ways and compares them.

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
overrides it. As of Slice 5b-7 the backend covers arithmetic, control flow,
top-level functions, algebraic data types with `match`, closures, and text
(`Int`, `Bool`, `String` and `Unit`; `let`, `+ - *`, the six comparisons, strict
`&&`/`||`, `if`/`else`, calls to named functions of up to five parameters,
monomorphic ADTs constructed then matched to extract an `Int`, non-recursive
lambdas of up to four parameters that capture their enclosing locals, and string
literals written to stdout by `io.println`), so anything outside it is rejected by
name rather than mis-compiled — the tree-walking `elya run` remains the full
language. Strings have no operations yet: `<>` and `==` on strings are refused by
name, not mis-compiled.
Tail calls are eliminated under a guarantee the LLVM verifier enforces, so
mutually recursive functions recur to any depth in a compiled binary just as
they do under `elya run` — and that holds through a closure call, which jumps
through a loaded code pointer rather than degrading to an ordinary call. ADT
values, closures and strings are heap-allocated (tag + fields; tag + code
pointer + captures; tag + length + bytes) and reclaimed by a
threshold-triggered mark-sweep collector backed by a shadow stack, so a
compiled binary that allocates unboundedly in a loop runs in bounded memory
rather than exhausting it; a closure is described to the collector by a
synthetic constructor descriptor row, so it is traced by exactly the code that
traces an ADT, and a string carries a row of its own that traces nothing — so
a `String` held in an ADT field or captured by a closure survives collection
by that same one mechanism. The evaluator refcounts instead, and reclaim
timing is unobservable in this acyclic subset — the collector reports its live
word level so the day that stops being true, the divergence is measured rather
than argued. A failed match traps with a defined error rather than undefined
behaviour. Every compiled program in the test corpus is additionally checked
against what the evaluator computes — both the value `main` returns and the
exact text `io.println` wrote, compared byte for byte — so the two never drift
apart silently.

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
- **Affine resources** — a type declared `linear` makes its values usable at
  most once (`E0428`), and an affine value may not be captured into a
  multiply-resuming continuation (`E0429`) — the double-free that a tracing
  collector cannot catch.
- **Name rules for effects** — operation names are unique per module (`E0202`); a
  handler clause must name a declared operation of the effect it names, with the
  right arity (`E0203`), at most once per handler (`E0204`), and a handler covers
  every operation of its effect (`E0207`); a top-level function
  may not share an operation's name (`E0205`); an operation may only be called,
  never used as a value (`E0206`); and a local binding shadows an operation of the
  same name, lexically. A bare clause (`ask() -> …`) means its operation's effect.

### Native compiler

`elya build` compiles through a typed Core IR to native code with LLVM 18:

- integers, booleans and control flow; **integer arithmetic is exact or fails by
  name** (overflow and a zero divisor stop the program, the same words as the
  evaluator); functions with **guaranteed tail calls** (`tailcc` + `musttail`, run
  1,000,000 deep);
- ADTs and pattern matching, nested and literal patterns included (compiled to flat
  matches; a failed match traps); closures, including indirect
  tail calls; strings and `io.println`;
- Elya's **own precise mark-sweep garbage collector**: non-moving, with a shadow
  stack, compiler-generated descriptor tables and free lists;
- **one-shot effect handlers** (selective CPS): `handle`/`resume` anywhere in a
  clause, deep re-installation, O(1) handler lookup, one-shot enforced by a named
  trap, and a state-passing handler loop bounded at N = 1,000,000; effectful
  lambdas and closure calls, and handles nested in handlers or effectful code;
- **effect-polymorphic functions**: a function generic in its effect row is
  specialized per calling convention at the rows it is used with, and a pure
  closure used where an effectful one is expected gets an adapter;
- **multi-shot handlers** (`with multi`): each resume re-enters a copy of the
  captured frames, so backtracking and search handlers run natively.

Every native program is checked against the reference evaluator, for both the
value and the exact bytes printed. CI runs the full gate on Windows too.

Not yet native (refused by name): code polymorphic in a type variable (`Ty::Var`).

### Async without function colouring

Suspension is an effect, so a scheduler is just a handler. In
[`examples/04_async.elya`](examples/04_async.elya) tasks call `yld()` and `fork(f)`;
the handler `task` turns a computation into a value that holds the rest of it (its
`resume`, stored in a constructor field), and `run` is a round-robin queue written in
plain Elya. The same `each` loops in ordinary code and, inside the tasks, over a body
that yields at every step: there is no async copy of it and no `await`. It runs in the
evaluator and natively, printing the same interleaving:

```
main: start
  ping
main
    pong
  ping
...
```

Function types can be written in `type` and `effect` declarations for this
(`Paused(fn() / {Log} -> Task)`); a declaration's rows are exactly what is written.

### Exact replay, and tests without mocks

A program touches the outside world only through effects, so whoever handles them decides
what the world says. In [`examples/05_replay.elya`](examples/05_replay.elya) a dice game
asks a `World` effect for rolls and clock readings. `record` sits between the program and
any world and logs every answer; `replay` feeds a log back with no world at all and stops
by name -- `Diverged(WantRoll(8), ..)` -- the first time the program asks for something the
recording does not hold; `fixed` is a test double. All four are ordinary handlers, the
game is written once, and it runs the same in the evaluator and natively:

```
test: 5 quick sixes score 60
recorded a live run
replay: same score
replay of the changed game: diverged at an 8-sided roll
```

### Affine resources

A type declared `linear` makes its values **affine** — usable at most once:

```elya
linear type File { File }

fn read(f) { match f { File -> "contents" } }

pub fn main() {
  let f = File
  let _ = read(f)
  io.println(read(f))   // error: E0428 — affine value `f` used more than once
}
```

Use-at-most-once is enforced today (`E0428`, `tests/affine.rs`), and so is the
payoff that motivated it: an affine value may not be **captured into a
multiply-resuming continuation** (`E0429`) — the double-free that a tracing GC
cannot catch. Each rule is checked both ways — the violation fires, and the
legitimate neighbouring program still compiles.

The guarantee is scoped honestly: it is **intra-function local** and models
create-and-consume within a single function body. Two deliberate
over-/under-approximations (a callee that duplicates an affine parameter; whole-
body branch-insensitivity) are tracked obligations to be tightened when the
analysis goes inter-procedural — see the Slice 4d-2 design spec.

## License

Elya is **source-available, not open source**. It is licensed under the
[PolyForm Strict License 1.0.0](LICENSE.md): you may **read it and use it** for any
**noncommercial** purpose (personal projects, study, research, teaching, charities and
public institutions). You may **not** modify it, make new works based on it, or distribute
it, whether original or modified, and commercial use is not permitted, without the author's
written permission. To ask, get in touch via [GitHub](https://github.com/Elvineej).

Copyright (c) 2026 Elvin Elakkattu.
