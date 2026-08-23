# Slice 5b-1 — Native Codegen: the Arithmetic MVP

**Goal.** Prove the native back-end spine exists end to end: an Elya source file
becomes a native executable that runs and prints the right integer.

**Arc.** 5b is the native-compilation arc (the brainstorm's `N1`..`N8`). This is
`N1`, the first slice of the back end. 5a built Core IR and left it a leaf with
no consumer; 5b-1 makes Core load-bearing by giving it its first real one.

**Non-negotiable boundary.** The slice is not done when the compiler emits
LLVM IR. It is done when a process the compiler produced has exited `0` after
writing `3\n` to stdout, and a test asserted that.

---

## §0 How to read this document

This slice buys exactly one thing: **the road exists**. It compiles a language
subset so small it is not a language — integer literals, `let`, variables, and
`+ - *`. Every later 5b slice widens the subset. None of them re-prove the
spine, because this one did.

Read §2 and §3 first if you are reviewing the design. They hold the two genuinely
new things: the inkwell/LLVM reach, and the Core→LLVM mapping. §5 holds the proof
obligation, which is the part that must not be negotiated down.

### The decoy we are NOT building

> A `codegen` module that emits well-formed, verifier-clean LLVM IR for the
> arithmetic subset, with a tidy `insta` snapshot of the `.ll` text, that has
> never been assembled, linked, or executed.

That decoy is indistinguishable from a working back end in a code review. It
would pass a gate, read well, and be wrong in the one way that matters: the
entire novel risk of this slice lives *after* IR generation — does the object
file emit, does it link against a C runtime on this platform, does the process
run, does `printf` produce the digits we expect. Snapshotting IR tests the part
we already know how to do and skips the part we don't.

Two structural consequences, both deliberate:

- **There is no snapshot test of the emitted IR in this slice.** Not one. An IR
  snapshot is a proof that would stay green while the binary is broken.
- **No test may skip.** No `#[ignore]`, no `if clang_missing() { return }`
  early-return, no "toolchain not available" soft pass. If the toolchain is
  absent the build or the test fails loudly. A skipped execution proof is the
  decoy wearing a different hat.

---

## §1 Scope

### In

- A new leaf module `src/codegen.rs`, behind a `codegen` cargo feature.
- `inkwell` as an optional dependency, pinned to one LLVM version.
- Core→LLVM lowering for: `CoreLit::Int`, `CoreKind::Var`, `CoreKind::Let`,
  and `CoreKind::Prim` for `Add`, `Sub`, `Mul`.
- Exactly one type in the value mapping: `Ty::Base(TyCon::Int)` → `i64`.
- A single-function module: the Elya `main`, zero parameters, no effects.
- Object-file emission via `TargetMachine`, then linking by shelling out to
  `clang`.
- One print primitive (§4) so the result is observable.
- An `elya build <file.elya>` subcommand.
- An execution-based test corpus (§5).

### Out (and why, briefly)

| Excluded | Why it is out of *this* slice |
|---|---|
| `Div`, `Rem` | Fidelity: LLVM `sdiv`/`srem` by zero is **undefined behavior**, the evaluator errors. A correct guard needs a branch, i.e. basic blocks. §3.4. |
| `And`, `Or` | Fidelity: short-circuit needs a branch. Strict `and i1` is a different language. §3.4. |
| `Bool`, `Str`, `Unit` values | Not observable through the int-only print primitive, so they could not be proven by execution. See §1.1. |
| `If`, `Unary`, `Float`, `Block`, `Handle`, `Resume`, `Qualified` | The seven constructs `core.rs` already declines to lower. Untouched here. |
| Calls, multiple functions | `N2`. This slice compiles one function. |
| Lambdas, ADTs, `Match`, heap, GC | `N4`/`N5`. They *lower* to Core today; they are not codegen-ready. |
| Runtime polymorphism | `N7`. Core stays polymorphic; codegen here demands monomorphic `Int`. |
| Effects | `N8`, the arc's crux. |
| Optimization passes, debug info, cross-compilation | Not spine. `OptimizationLevel::None`, host triple only. |

### §1.1 Why Bool was cut

Bool was in the draft subset (`i1`, six `icmp` predicates) as a cheap way to
prove the type-dispatch seam. It is cut, and the reason is the review criterion
itself: the print primitive prints an integer, so a `Bool` value in this slice
could not be observed by a running binary. It could only be checked by
inspecting IR — which is the decoy. Anything this slice supports must be
provable by execution; Bool cannot be, so Bool waits for `N3` (`If`), where it
becomes observable through the branch it selects.

This is the criterion doing real work: it *shrank* the slice.

### Honesty line

Native codegen is **necessary but not sufficient** for self-hosting. It is what
eventually lets Rust drop out of the *runtime*. Rewriting the Elya compiler in
Elya is separate, later, and much larger work. Nothing in 5b-1 brings that day
closer except by existing.

And plainly: a compiler that compiles exactly `1 + 2` is not a compiler. It is a
proof that the road from source text to a running process is paved end to end.
That proof is the deliverable.

---

## §2 The inkwell integration (the new reach)

This is the riskiest part of the slice and it is risky for a boring reason:
`inkwell` is a safe wrapper over `llvm-sys`, and `llvm-sys` links against a
**system LLVM installation including its development libraries**. This is not a
`cargo add`. It is a native toolchain dependency.

### §2.1 What has to be true on the machine

1. An LLVM installation with development libraries and `llvm-config`.
2. `clang` on `PATH` (ships with LLVM; used as the linker driver, §3.6).
3. Either `llvm-config` discoverable on `PATH`, or `LLVM_SYS_<NNN>_PREFIX` set
   to the install root, where `<NNN>` is the LLVM major/minor without the dot
   (e.g. `LLVM_SYS_181_PREFIX` for LLVM 18.1).

### §2.2 The Windows friction, stated plainly

The official LLVM release binaries for Windows have historically been a
clang-focused distribution that does **not** ship the static libraries and
`llvm-config` that `llvm-sys` needs. If that is the case on this machine, the
options are: obtain a distribution that includes LLVM development libraries, or
build LLVM from source (hours). There is no third option that keeps inkwell.

This is why **Task 1 is a hard prerequisite gate that writes no Elya code**
(§9). It answers "does this machine have a usable LLVM?" before a single line of
Core→LLVM mapping is written, so a toolchain dead end costs one task and not a
slice. If Task 1 cannot be made green, the correct action is to **stop and report**,
not to work around it: the fallback (emit textual `.ll`, shell to `clang`, zero
new dependencies) is a design fork the user already ruled on, and re-opening it
is the user's call, not the implementer's.

### §2.3 Cargo wiring

```toml
[dependencies]
logos = "0.14"
ariadne = "0.4"
inkwell = { version = "0.5", features = ["llvm18-1"], optional = true }

[features]
codegen = ["dep:inkwell"]
```

- `inkwell` requires **exactly one** `llvmNN-M` feature. Zero fails to build;
  two fail to build. The exact feature string is **determined empirically in
  Task 1** from the LLVM actually installed — `llvm18-1` above is a placeholder
  and Task 1 corrects it.
- `optional = true` + the `codegen` feature is what preserves the project's
  no-new-deps posture for everyone not compiling natively: the default build
  and the default test run pull in no LLVM at all.

### §2.4 Feature gating, and why it does not become a silent skip

Feature-gating is a build-time convenience, not a test escape hatch. The project
gate (`scripts/check.sh`) is extended to run the codegen feature explicitly:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
cargo clippy --all-targets --features codegen -- -D warnings
cargo test --features codegen --test native_codegen
```

So on this machine the execution proof runs on every gate. The last two lines
are what stop `codegen` from becoming a feature nobody exercises.

Failure modes are all loud: missing LLVM fails the `llvm-sys` build script;
missing `clang` fails the link step with its stderr surfaced; a wrong answer
fails the stdout assertion.

### §2.5 What inkwell buys over hand-written `.ll`

Worth naming since it costs a heavy dependency: a typed IR builder (type errors
become Rust type errors instead of `llc` parse errors), `module.verify()` as a
real correctness check we can call in a unit test, and `TargetMachine` object
emission without an external `llc` invocation. The last one matters most — it
removes a subprocess and a text-format round trip from the middle of the spine.

---

## §3 The Core→LLVM mapping (arithmetic subset)

Input: `core::CoreModule` from `core::lower_module`. This slice is Core's first
real consumer.

### §3.1 Module shape

```
CoreModule { fns: [ CoreFn { name: "main", params: [], body } ] }
```

Anything else is rejected:

- more than one `CoreFn` → `Unsupported("multi-function module")`
- no `CoreFn` named `main` → `Unsupported("no `main`")`
- `main.params` non-empty → `Unsupported("main takes parameters")`

Rejections are errors, not silent skips. `N2` lifts the first and third.

### §3.2 Type mapping

| `Ty` | LLVM |
|---|---|
| `Ty::Base(TyCon::Int)` | `i64` |
| everything else | `CodegenError::Unsupported` |

Codegen reads the **inline `ty` field** on each `CoreExpr`, not a side table.
This is the first consumer to actually depend on Shape C, and it is worth
noting: the reason codegen can be a pure structural fold with no environment
lookups for types is that 5a-2 put the types on the nodes.

`CoreExpr.span` stays provenance-only. Debug-info emission is deferred; the span
is carried into `CodegenError` for diagnostics and nowhere else.

Note the polymorphism boundary: Core nodes may carry `Ty::Var(_)`, because Core
is deliberately stay-polymorphic. Codegen demands monomorphic `Int` and errors
otherwise. `main`'s body in this corpus is monomorphic, so this never fires; when
it does fire, that is `N7` knocking.

### §3.3 Expression lowering

A recursive fold returning an `IntValue<'ctx>`, threading an environment
`HashMap<String, IntValue<'ctx>>`.

| `CoreKind` | LLVM |
|---|---|
| `Lit(CoreLit::Int(n))` | `i64_type.const_int(n as u64, true)` |
| `Var(x)` | environment lookup; absent → `Unsupported("unbound var")` |
| `Let(x, rhs, body)` | lower `rhs`; save any prior binding of `x`; bind; lower `body`; restore |
| `Prim(Add, [a, b])` | `build_int_add` |
| `Prim(Sub, [a, b])` | `build_int_sub` |
| `Prim(Mul, [a, b])` | `build_int_mul` |
| everything else | `Unsupported` |

**No `alloca`, no `mem2reg`.** Bindings are immutable and this slice has no
control flow, so values map directly to SSA registers. The save/restore
discipline on `Let` is what makes shadowing (`let x = 1; let x = x + 1;`)
correct.

`Prim` arity is checked: anything other than exactly two operands for a binary
op is `Unsupported`, not a panic.

### §3.4 Two semantic-fidelity findings

These came out of writing the mapping and are the most useful thing in §3. The
general obligation: **native codegen must never be more-undefined than the tree
evaluator.**

**Division.** LLVM's `sdiv`/`srem` with a zero divisor is undefined behavior —
not a trap, not an error, genuinely UB, meaning the optimizer may assume it does
not happen. The evaluator raises a runtime error. Lowering `Div` to a bare
`sdiv` would make the native build silently more dangerous than the interpreted
one. A faithful lowering needs a zero test and a branch, which needs basic
blocks, which this slice does not have. **`Div` and `Rem` are therefore out of
5b-1**, and the guard lands in the slice that introduces blocks.

**Overflow.** LLVM `add`/`sub`/`mul` **without** the `nsw`/`nuw` flags is
defined two's-complement wrapping; **with** `nsw`, signed overflow is UB. This
slice emits them **without** flags — the defined-wrapping choice — deliberately
declining the small optimization win that would introduce UB. Whether that
matches the evaluator (Rust's `i64` arithmetic panics on overflow in debug and
wraps in release) is an open reconciliation, tracked in §11. It is a divergence
between the two back ends, and it is written down rather than discovered later.

### §3.5 Function and entry shape

Two LLVM functions are emitted:

```llvm
declare i32 @printf(ptr, ...)
@.fmt = private unnamed_addr constant [6 x i8] c"%lld\0A\00"

define i64 @elya_main() {
entry:
  ; <lowered Core body>
  ret i64 %result
}

define i32 @main() {
entry:
  %v = call i64 @elya_main()
  %p = call i32 (ptr, ...) @printf(ptr @.fmt, i64 %v)
  ret i32 0
}
```

Elya's `main` becomes `@elya_main`, and the C entry point `@main` is a separate
generated shim. This costs one extra call and buys two things: the Elya function
namespace stays clean for `N2` (where real Elya functions get emitted by name),
and the print convention is confined to a shim rather than tangled into body
lowering — so deleting it later is deleting a function, not unpicking a fold.

### §3.6 Object emission and linking

```
Target::initialize_native(&InitializationConfig::default())
triple  = TargetMachine::get_default_triple()
machine = target.create_target_machine(
              &triple, host_cpu, host_features,
              OptimizationLevel::None, RelocMode::Default, CodeModel::Default)
module.verify()?                       // real check, runs before emission
machine.write_to_file(&module, FileType::Object, obj_path)?
```

Linking shells out to `clang <obj> -o <exe>`. `clang` is hardcoded, not
configurable: LLVM is already a hard prerequisite of this slice and `clang`
ships with it, so a knob would be surface without a user. A non-zero exit
surfaces `clang`'s stderr in `CodegenError::Link`.

Host triple only. Cross-compilation is out.

---

## §4 The print primitive

**One primitive, not a runtime library.** Concretely, the entire runtime
interface of this slice is:

- **one external symbol**: `printf`, from the platform C runtime;
- **one generated shim**: `@main`, per §3.5.

There is no `runtime.c`, no static archive to build or ship, no linker script,
no Elya-visible builtin, and **no front-end change of any kind**.

The convention is: *the integer value of `main` is printed on exit.* That is a
**codegen convention, not a language feature**. Elya's surface syntax, parser,
resolver, and type checker are untouched — `pub fn main() { 1 + 2 }` already
type-checks today (verified: `check_main_discharge` at `types.rs:1935` constrains
only `main`'s *effect row*, not its return type).

### Rejected alternative: an `io.print_int` builtin

Adding a real print builtin was the obvious other route and it is worse here on
three counts. It is language-surface change smuggled into a codegen slice. It
routes through `Expr::Qualified`, which `core.rs` declines to lower
(`Unsupported`) — so it would drag whole-body lowering into the spine slice. And
`Qualified` is typed `Ty::Error` at `types.rs:907` ("typed at the Call site
(builtins)"), making it a live candidate for the recorder-totality gap class
(§6.3) — so it would drag an inference fix in too.

The convention costs a five-line shim. Real `io.println` arrives with strings
(`N6`), where it belongs.

---

## §5 The deliverable

`tests/native_codegen.rs`, gated `#![cfg(feature = "codegen")]`.

Each case: source text → compile → **run the produced binary** → assert on its
exit status and stdout.

| # | Source | Expected stdout | Proves |
|---|---|---|---|
| 1 | `pub fn main() { 1 + 2 }` | `3` | the spine |
| 2 | `pub fn main() { let x = 6; let y = 7; x * y }` | `42` | `Let`, `Var`, `Mul` |
| 3 | `pub fn main() { (2 + 3) * 4 - 5 }` | `15` | nesting, precedence, `Sub` |
| 4 | `pub fn main() { 3 - 10 }` | `-7` | signed negatives survive `%lld` |

Four cases, not one, so the proof is not "a program that prints a hardcoded 3."

**The strongest case drives the real CLI.** At least one test invokes the actual
`elya` binary via `env!("CARGO_BIN_EXE_elya")` — `elya build prog.elya -o
prog.exe` — then runs `prog.exe`. That makes the proof cover the user-facing
entry point, not just the library API.

Assertions per case, all three required:

```rust
assert!(output.status.success(), "binary exited {:?}", output.status);
assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "3");
assert!(String::from_utf8_lossy(&output.stderr).is_empty());
```

**Temp files.** All intermediates (`.o`, `.exe`, source) go in a unique
per-test subdirectory under `std::env::temp_dir()` — **never under a tracked
path** — and are removed on success. The uniqueness key is process id plus a
per-test counter. No `tempfile` dependency: this is roughly ten lines, and the
slice is already spending its dependency budget on inkwell.

---

## §6 Non-interference guarantees

### §6.1 Without the feature, nothing changes

`cargo build` and `cargo test --all` with no features must behave exactly as
they do today: no LLVM, no inkwell, no new transitive dependencies, `elya run`
and `elya check` byte-identical, every existing snapshot untouched. The gate's
first three lines (§2.4) are unchanged and must stay green.

### §6.2 The front end and Core are not modified

Codegen is a **new leaf**. It consumes `core::CoreModule` and is consumed by
nothing. No change to `lex`, `parse`, `resolve`, `types`, `exhaust`, `affine`,
`eval`, or `core` is anticipated — `CoreFn { name, params, body }` already
exposes what codegen needs.

That is a prediction, and Task 2 is where it gets tested rather than assumed. If
Core does need a change, it is reported at the Task 2 checkpoint before any
codegen is written.

### §6.3 Recorder totality: measure, then fix

The recorder-totality gap is a tracked **class** — every node lowering can reach
must be present in `node_types`, currently proven only where the corpus happens
to reach. This slice does **not** open with a speculative audit of `infer_call`'s
descend-vs-synthesize branches. A blanket audit would close branches that may not
be on this slice's path at all.

Instead: **Task 2 measures.** It runs the real pipeline plus `lower_module` over
the four corpus programs and looks at what comes back.

- A `LowerError::Untyped(span)` is a recorder gap on a path this slice actually
  needs. Fix it with the shape already proven in 5a-2 Task 3: record the
  synthesized node's type at its span, in inference.
- A `LowerError::Unsupported(_)` means the subset was drawn wrong. Narrow the
  corpus; do not widen `core.rs`.

Expectation, stated so it can be falsified: all four programs lower unchanged
today, because 5a-2's surface-1 snapshot already proves `Lit`/`Var`/`Prim` lower
(`(fn add1 (n) (prim Add (var n : Int) (lit 1 : Int) : Int))`). If that holds,
this slice closes zero recorder gaps and the class stays tracked. That is the
correct outcome, not a missed opportunity.

---

## §7 Pipeline and module changes

| File | Change |
|---|---|
| `Cargo.toml` | optional `inkwell` dep; `codegen` feature (§2.3) |
| `src/lib.rs` | `#[cfg(feature = "codegen")] pub mod codegen;` |
| `src/codegen.rs` | **new** — the whole slice (flat module, matching `core.rs`) |
| `src/main.rs` | `#[cfg(feature = "codegen")]` `build` subcommand |
| `scripts/check.sh` | two added lines (§2.4) |
| `tests/native_codegen.rs` | **new** — the execution proof |

### Public surface of `src/codegen.rs`

```rust
pub enum CodegenError {
    Unsupported(&'static str),
    Verify(String),
    Io(std::io::Error),
    Link { code: Option<i32>, stderr: String },
}

/// Core -> object file on disk. The spine.
pub fn compile_module(core: &CoreModule, obj_path: &Path) -> Result<(), CodegenError>;

/// Object file -> executable, via `clang`.
pub fn link(obj: &Path, exe: &Path) -> Result<(), CodegenError>;

/// Debugging aid ONLY. Never asserted on; no snapshot test uses it. See §0.
pub fn emit_ir(core: &CoreModule) -> Result<String, CodegenError>;
```

`emit_ir` exists because a failed execution proof is much easier to debug with
the IR in hand. Its docstring says what its docstring says here: it is not a
proof, and nothing in the test suite asserts on its output.

### CLI

```
elya build <file.elya> [-o <out>]
```

Runs the existing front end (so type errors and warnings surface exactly as
`elya check` does), lowers to Core, compiles, links. Default output is the input
stem with the platform executable extension. Exit code follows the existing
`ExitCode` convention in `main.rs`.

---

## §8 Testing strategy

Three layers, in ascending order of what they prove.

**Layer 1 — unit tests on the fold** (`src/codegen.rs`, `#[cfg(test)]`). That
`module.verify()` succeeds for each corpus program, and that out-of-subset
constructs produce the *specific* `Unsupported` message rather than a panic. Six
rejections: `Div`, `And`, a `Bool` literal, a lambda, a multi-function module,
and a parameterised `main`. These are cheap and catch mapping mistakes early.
They are **not** the proof.

**Layer 2 — lowering measurement** (Task 2). The four programs reach `CoreModule`
through the real pipeline. Documents the actual reach into Core (§6.3).

**Layer 3 — execution** (`tests/native_codegen.rs`). §5. This is the proof.
Layers 1 and 2 could all pass while the back end is useless; layer 3 could not.

**Explicitly absent:** any `insta` snapshot of LLVM IR (§0), and any test that
can skip (§0).

---

## §9 Build order

Each task ends green, is committed atomically, and pauses for review.

### Task 1 — Toolchain prerequisite gate *(no Elya code)*

Isolates the entire novel risk before any design commitment, the way 5a-1
isolated the typed-table feeder.

Add the optional dep and feature. Determine the correct `llvmNN-M` feature
string empirically from the installed LLVM. Then a single test,
`toolchain_smoke`, that builds a trivial LLVM module **by hand** — no Core, no
Elya, an `@elya_main` returning the constant `3` plus the §3.5 shim — verifies
it, emits an object, links it with `clang`, runs it, and asserts stdout is `3`.

Also verifies the `%lld` format string round-trips on this platform's C runtime.

**Deliverable:** `cargo test --features codegen` prints `3` from a process this
repo produced.

**Stop condition:** if this cannot be made green, **stop and report**. Do not
route around it. The textual-`.ll` fallback is the user's fork to re-open.

### Task 2 — Core reach measurement *(no codegen)*

Run `front_end` + `lower_module` over the four §5 corpus programs. Assert all
four produce a `CoreModule`. Fix any `Untyped` per §6.3; narrow the corpus on any
`Unsupported`. Report what was actually needed at the checkpoint — including
"nothing," which is the expected answer.

**Deliverable:** a test proving the corpus lowers, and a factual statement about
whether the recorder-totality class was touched.

### Task 3 — Core→LLVM for the arithmetic subset

Implement §3.1–§3.3 and §3.5's `@elya_main`. Layer-1 unit tests: `verify()`
passes for all four programs; the six out-of-subset rejections produce their
specific `Unsupported` messages.

**Deliverable:** verifier-clean IR for the corpus. Deliberately not yet a proof.

### Task 4 — Shim, object, link, and the execution proof

The `printf` declaration, the format-string global, the `@main` shim,
`TargetMachine` object emission, `link`, the temp-dir harness, and all four §5
cases run through the **library API**.

**Deliverable:** four binaries run and print `3`, `42`, `15`, `-7`. **This is the
slice's milestone.**

### Task 5 — `elya build` and the CLI-driven proof

The `build` subcommand, plus one §5 case re-run through
`env!("CARGO_BIN_EXE_elya")` so the proof covers the real entry point. Extend
`scripts/check.sh` per §2.4.

**Deliverable:** `elya build` produces a runnable native executable, asserted by
a test that runs it.

---

## §10 Risks and mitigations

| # | Risk | Mitigation |
|---|---|---|
| 1 | **LLVM dev libraries unavailable on Windows** (§2.2). The dominant risk. | Task 1 is a hard gate costing one task. Abort-and-report is an explicit, allowed, non-failure outcome. |
| 2 | inkwell/LLVM version mismatch — exactly one `llvmNN-M` feature must match the install. | Determined empirically in Task 1; the value in §2.3 is a placeholder. |
| 3 | Slow gate: `llvm-sys` links a large native library. | Feature-gated, so the default gate is untouched. `CARGO_INCREMENTAL=0` is already the project norm. |
| 4 | **Semantic drift from the evaluator** (§3.4). | `Div`/`Rem` excluded until a guard is possible; arithmetic emitted without `nsw`/`nuw`; overflow reconciliation tracked in §11. |
| 5 | Test artifacts polluting the repo. | Unique temp dirs under `std::env::temp_dir()`, never a tracked path, removed on success. |
| 6 | The execution proof quietly degrading into an IR check. | §0 forbids IR snapshots and skips structurally; §2.4 makes the gate run the feature. |
| 7 | `%lld` behaving differently across C runtimes. | Verified in Task 1 before anything depends on it; case 4 asserts a negative. |
| 8 | Feature-gated code rotting (compiles only one way). | `check.sh` clippies **both** configurations with `-D warnings`. |

---

## §11 Deferred and honestly flagged

**Deferred to named later slices:**

- `Div`/`Rem` with a zero guard, `And`/`Or` with short-circuit, `If`, `Bool` —
  all need basic blocks (`N3`).
- Multi-function modules and calls (`N2`).
- ADTs, `Match`, heap allocation, GC (`N4`) — the heap threshold, where the
  runtime story stops being one `printf`.
- Closures (`N5`), strings and real `io.println` (`N6`), runtime polymorphism
  (`N7`), effects and the `Handle`/`Resume` crux (`N8`).
- The remaining Core `Unsupported` arms: `Float`, `Qualified`, `Unary`, `Block`.

**Tracked obligations, not this slice:**

- **Overflow semantics reconciliation** (§3.4). The evaluator and the native
  back end may disagree on `i64` overflow. Written down here so it is a decision
  later rather than a surprise.
- **Recorder totality as a class** (§6.3). Stays tracked; closes incrementally by
  real demand, never by speculative audit.
- The print convention (§4) is temporary scaffolding, removed when `io.println`
  is genuinely compiled.
- Debug info from `CoreExpr.span`; optimization passes; cross-compilation.
- Pre-existing and untouched: the relay+own-effect row leak, the affine
  callee-duplication soundness gap, the affine intra-procedural
  over-approximation.

---

## §12 Milestone checklist

- [ ] `inkwell` is an **optional** dependency behind a `codegen` feature; the
      default build pulls in no LLVM.
- [ ] Task 1's hand-built LLVM module compiles, links, runs, and prints `3` on
      this machine — proven before any Core→LLVM code exists.
- [ ] The four corpus programs reach `CoreModule` through the real pipeline, and
      whether any recorder gap was closed is stated as fact.
- [ ] `module.verify()` passes for every corpus program.
- [ ] Out-of-subset constructs produce specific `Unsupported` errors; nothing
      panics.
- [ ] Exactly one external symbol (`printf`) and one generated shim. No runtime
      library. No front-end change.
- [ ] **Four native binaries run and print `3`, `42`, `15`, `-7`** — asserted on
      exit status, stdout, and empty stderr.
- [ ] At least one case drives the real `elya build` binary end to end.
- [ ] **No `insta` snapshot of LLVM IR exists in this slice.**
- [ ] **No test can skip** — no `#[ignore]`, no toolchain-probe early return.
- [ ] `cargo test --all` with no features stays green; every existing snapshot is
      unchanged.
- [ ] `scripts/check.sh` clippies and tests **both** feature configurations with
      `-D warnings`.
- [ ] `Div`, `Rem`, `And`, `Or` are rejected, and §3.4 records why.
