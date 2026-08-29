# Elya — Slice 5b-2 Implementation Plan: Native Control Flow (arc node N3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Teach the native back end to branch — `if`, `Bool` as `i1`, the six comparison operators, and strict `&&`/`||` — so that compiled Elya binaries can compute something whose answer depends on a runtime test.

**Architecture:** The front end grows a real `CoreKind::If` node (Core has none today; `Expr::If` is refused at lowering). The codegen fold widens its value representation from "one type, i64" to "two integer widths, i64 and i1", then gains a three-block LLVM diamond joined by a `phi`. Comparisons become `icmp` with **signed** predicates; `&&`/`||` become `and i1`/`or i1`, strict, because both of Elya's evaluators are strict and the back end must not diverge from them. The proof is execution: seven source programs compiled to native binaries and run, each output additionally compared against what the evaluator says `main` is worth.

**Tech Stack:** Rust 2021 · inkwell 0.5 (`default-features = false`, features `["llvm18-0", "target-x86"]`) · LLVM 18.1.6 (vcpkg) · `clang` (winget) as link driver · `cargo` workspace: `elya` (front end, LLVM-free) / `elya-codegen` (the only crate linking `llvm_sys`) / `elya-cli`.

**Spec:** [../specs/2026-08-29-elya-slice-5b2-native-control-flow-design.md](../specs/2026-08-29-elya-slice-5b2-native-control-flow-design.md)

## Global Constraints

Every task's requirements implicitly include this section.

- **Rust 2021, MSRV 1.75.** No new dependencies in any crate; inkwell's feature set is fixed and must not change.
- **Command prelude.** Every cargo invocation is prefixed `$env:CARGO_INCREMENTAL="0";` — the incremental cache hangs on this machine.
- **Codegen builds use `-j 2`.** The `llvm_sys` rlib is 3.73 GiB and every test binary links it; higher parallelism exhausts memory.
- **`cargo fmt --all` (write mode) runs BEFORE the gate.** The gate fmt-*checks* and fails hard on drift.
- **The gate is `pwsh scripts/check.ps1`** — five stages, both feature configurations:
  1. `cargo fmt --all -- --check`
  2. `cargo clippy -p elya -p elya-cli --all-targets -- -D warnings` (Configuration A — LLVM-free)
  3. `cargo test -p elya -p elya-cli`
  4. `cargo clippy --workspace --all-targets --features elya-cli/codegen -- -D warnings` (Configuration B — codegen)
  5. `cargo test --workspace --features elya-cli/codegen`
- **clippy `-D warnings` in both configurations.** No `#[allow]` added to silence a finding without a written reason.
- **Atomic commits, never red.** Each task's final step is: run the gate; only if it exits 0, `git add <explicit paths>`, commit, push.
- **`git add <explicit paths>` only. Never `git add -A` or `git add .`.**
- **Commit trailer:** `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`
- **Push `origin main` after each task commit.**
- **Commit and push require the user's explicit go-ahead** before running (repo operating rule). The plan states the exact command so that confirmation is a yes/no, not a design question.
- **Run `graphify update .` after code changes** (project CLAUDE.md) so the knowledge graph stays current.

**Non-negotiable boundaries:**

- **No `insta` snapshot of LLVM IR anywhere** (5b-1 §0). `emit_ir` is a debugging aid; nothing in the suite asserts on its output.
- **No test may skip.** No `#[ignore]`, no toolchain probes that turn a missing tool into a pass, no soft passes.
- **Semantic fidelity** (5b-1 §3.4, extended in 5b-2 §4.1): native codegen must never be *more*-undefined than the tree evaluator, and never *less*-undefined in an observable way.
- **Out-of-subset constructs are refused BY NAME** — `CodegenError::Unsupported("…")` with a specific static message — never mis-compiled, never a panic.
- **`elya run` behaviour is bit-for-bit unchanged.** This is what keeps the differential check non-circular.
- **Configuration A must have zero `llvm_sys` in its dependency graph.** `src/` never mentions inkwell or LLVM.

---

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `src/core.rs` | `CoreKind::If` variant; `Expr::If` lowering arm; `pretty_expr` rendering | 1 |
| `tests/core_lowering.rs` | `walk` arm for the new variant; `If` lowering + origin test | 1 |
| `src/eval.rs` | `run_loop` returns `main`'s value; `run_module_value` exposes it | 2 |
| `tests/eval_value.rs` | The accessor agrees with the machine, and `run_module` is unchanged | 2 |
| `crates/codegen/src/lib.rs` | `repr_ty` widening (i64/i1); `require_int` moves to the return-type check | 3 |
| `crates/codegen/src/lib.rs` | `If` diamond + `phi`; six `icmp` predicates; `Eq`/`Ne` operand dispatch; strict `and`/`or`; module doc correction | 4 |
| `crates/codegen/tests/native_codegen.rs` | Seven-program execution corpus; the differential check | 5 |
| `README.md` | The "backend covers" paragraph, updated to the N3 subset | 6 |
| `docs/superpowers/specs/2026-08-29-elya-slice-5b2-native-control-flow-design.md` | §12 milestone checklist, ticked | 6 |

---

## Deliberate deviations from the spec's letter, flagged for review

Three, all found by reading the code the spec describes. The first two change what the tasks do; the third is a one-line correction of an error that had already propagated.

**1. The Core reach is five sites, not "exactly one Core arm" (spec §2.1).**

Spec §2.1 says the front-end reach is one arm — the `Expr::If` refusal at [src/core.rs:177](../../../src/core.rs#L177). That understates it, because **`CoreKind` has no `If` variant** (✓ VERIFIED — [src/core.rs:26-40](../../../src/core.rs#L26-L40) lists exactly `Lit`, `Var`, `App`, `Prim`, `Lambda`, `Let`, `Match`). Flipping the refusal into a lowering has nowhere to lower *to*. The real site list is five: the enum variant, the lowering arm, `pretty_expr`, the `walk` helper in `tests/core_lowering.rs`, and a temporary refusal arm in the codegen fold so the workspace still builds between Task 1 and Task 4. A repo-wide grep confirms those are **all three** exhaustive `CoreKind` matches in existence (✓ VERIFIED — only `src/core.rs`, `crates/codegen/src/lib.rs`, and `tests/core_lowering.rs` mention `CoreKind::` at all).

The alternative — desugaring `If` into `CoreKind::Match` on `CorePat::Lit(CoreLit::Bool(_))` — was considered and **rejected**. It would force codegen to accept *some* `Match` nodes, contradicting spec §11's "Core lowers `Match` today; codegen still refuses it", and it would degrade the refusal message from `"If"` to `"Match"`, which is a worse thing to read.

The offsetting good news: **no test anywhere asserts `Unsupported("If")` or `Unsupported("Block")`** (✓ VERIFIED by grep), so spec §6.2's caution that "Task 1 finds them" resolves to zero test updates.

**2. The §5 differential check is not implementable as written without a small additive change to `src/eval.rs`, which spec §6.3 forbids by its letter.**

Spec §5 says each corpus program is "also run through `elya run` and the two outputs compared". It cannot be, as written: `elya run` → `elya::run_source` → `interp.output()`, and `output()` is **only what `io.println` accumulated**. Both evaluators discard `main`'s value (✓ VERIFIED — [src/eval.rs:871-878](../../../src/eval.rs#L871-L878), `ret`'s `let Some(node) = k else { return Ok(None); // final result; output already captured via io.println }`). So for `pub fn main() { if 1 < 2 { 10 } else { 20 } }`, `elya run` prints *nothing* while the native binary prints `10`. Comparing those two is vacuous — the check would pass while proving nothing.

The underlying fact is worth naming, because it is more interesting than the bug: **the 5b-1 print convention invented an observable the language does not have.** `printf("%lld\n", elya_main())` prints a value that no Elya program can otherwise observe. The differential check is therefore not "same observable, two implementations"; it is "the native binary's stdout must equal what the evaluator computed for `main`", and something has to be able to say what that was.

Options weighed and discarded: making corpus programs print via `io.println` (impossible — `io.println` rejects non-`String` arguments and `Str` is refused by codegen until N6); a test-local reference evaluator over Core (a second evaluator, circular and worse); an `io.print_int` builtin (explicitly refused by 5b-1 §4, "declined to smuggle an `io.print_int` builtin into a codegen slice").

**Chosen: one new public function in `src/eval.rs`, and `run_loop` returns the value it already has instead of throwing it away.** `run_module` keeps its exact signature and behaviour by delegating. `src/lib.rs` and `src/types.rs` are **not** touched — the rendering rule (`Int(n)` → `n.to_string()`, anything else a harness failure) lives in the codegen test where the `%lld` convention lives, so the LLVM-free front end learns nothing about print formats. This deviates from §6.3's letter ("Neither `eval.rs` nor `types.rs` is modified") while preserving its stated *purpose* verbatim: "`elya run` behaviour is bit-for-bit unchanged, which is what makes the §5 differential check meaningful rather than circular." Task 2 is a standalone commit precisely so this deviation can be rejected on its own without disturbing the back-end work.

**3. The 5b-1 module doc carries the same wrong strictness assumption the spec's deferral line did.**

[crates/codegen/src/lib.rs:9](../../../crates/codegen/src/lib.rs#L9) reads "no And/Or (short-circuit needs a branch)". Both evaluators are strict (spec §4.1), so this is the same latent error the 5b-1 spec's deferral line carried — it propagated into the code comment too. Task 4 corrects it in the same commit that makes `And`/`Or` work.

---

### Task 1: Core grows an `If` node

The front-end half, standalone: after this task `if` lowers to Core, and codegen refuses it by name. Nothing about the back end changes yet. Configuration A alone would catch a regression here.

**Files:**
- Modify: [src/core.rs](../../../src/core.rs) — `CoreKind` enum (`:26-40`), `lower_expr` (`:177`), `pretty_expr` (`:234`)
- Modify: [crates/codegen/src/lib.rs:172-174](../../../crates/codegen/src/lib.rs#L172-L174) — one temporary refusal arm
- Test: [tests/core_lowering.rs](../../../tests/core_lowering.rs) — `walk` helper (`:33-60`) and one new test

**Interfaces:**
- Consumes: `lower_expr(&Expr, Span, &BTreeMap<Span, Ty>) -> Result<CoreExpr, LowerError>`; `lower_block(&Block, &table) -> Result<CoreExpr, LowerError>`; the test helpers `lower_src(&str) -> (CoreModule, BTreeMap<Span, Ty>)`, `nodes(&CoreModule) -> Vec<&CoreExpr>`, `render1(&Ty) -> String`, `both_origin_checks(&CoreModule, &BTreeMap<Span, Ty>)`.
- Produces: `CoreKind::If(Rc<CoreExpr>, Rc<CoreExpr>, Rc<CoreExpr>)` — condition, then-branch, else-branch, in that order, all three non-optional. Tasks 4 and 5 match on it. Also `CodegenError::Unsupported("If")`, which Task 4 deletes.

- [x] **Step 1: Write the failing test**

Append to [tests/core_lowering.rs](../../../tests/core_lowering.rs):

```rust
#[test]
fn if_lowers_to_a_two_branch_core_node() {
    // `else_block` is `Rc<Spanned<Block>>`, not an Option (src/ast.rs:180-184),
    // so every surface `if` is already two-branch: Core needs no synthesized
    // Unit else, and the back end's phi always has exactly two incoming values.
    let (core, table) = lower_src("pub fn main() { if 1 < 2 { 10 } else { 20 } }\n");
    let root = &core.fns[0].body;
    let CoreKind::If(cond, then_e, else_e) = &root.kind else {
        panic!("main's body should lower to CoreKind::If, got {:?}", root.kind);
    };
    assert_eq!(render1(&root.ty), "Int", "the if node takes its branches' type");
    assert_eq!(render1(&cond.ty), "Bool");
    assert_eq!(render1(&then_e.ty), "Int");
    assert_eq!(render1(&else_e.ty), "Int");

    // Every `If` node is direct-origin: its span is the surface `if`'s span, so
    // the frozen table must already hold its type (spec §2.2 — `infer_expr` is
    // the single record point and `Expr::If` runs through it).
    both_origin_checks(&core, &table);
    assert!(
        nodes(&core).iter().any(|n| matches!(n.kind, CoreKind::If(..))),
        "the walk helper must reach into If children"
    );
}
```

- [x] **Step 2: Run it and watch it fail**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test core_lowering if_lowers_to_a_two_branch_core_node
```

Expected: FAIL — `lower_src` panics with `lowering the corpus subset should succeed: Unsupported("If")`, and the file does not compile because `CoreKind::If` does not exist.

- [x] **Step 3: Add the `CoreKind::If` variant**

In [src/core.rs](../../../src/core.rs), inside `pub enum CoreKind`, after the `Let` variant and before `Match`:

```rust
    /// `if cond { .. } else { .. }` — always two-branch, because the AST's
    /// `else_block` is not optional. Both branches carry the same type (the
    /// checker unified them), which is what lets the back end join them with a
    /// single-typed `phi`.
    If(Rc<CoreExpr>, Rc<CoreExpr>, Rc<CoreExpr>),
```

- [x] **Step 4: Replace the lowering refusal with a real arm**

In [src/core.rs](../../../src/core.rs), delete the line `Expr::If { .. } => return Err(LowerError::Unsupported("If")),` (`:177`) and add this arm alongside `Expr::Lambda`, which is the model — both reach a `Block` through `lower_block`, never through the still-refused `Expr::Block`:

```rust
        Expr::If {
            cond,
            then_block,
            else_block,
        } => {
            let c = lower_expr(&cond.node, cond.span, table)?;
            let t = lower_block(&then_block.node, table)?;
            let e = lower_block(&else_block.node, table)?;
            CoreKind::If(Rc::new(c), Rc::new(t), Rc::new(e))
        }
```

- [x] **Step 5: Add the `pretty_expr` arm**

In [src/core.rs](../../../src/core.rs)'s `pretty_expr` (`:234`), after the `Let` arm. Each arm pushes only the opening form; the shared tail at the end of the function appends `" : <ty>)"`.

```rust
        CoreKind::If(cond, then_e, else_e) => {
            s.push_str("(if ");
            pretty_expr(cond, p, s);
            s.push(' ');
            pretty_expr(then_e, p, s);
            s.push(' ');
            pretty_expr(else_e, p, s);
        }
```

- [x] **Step 6: Add the `walk` arm in the test helper**

In [tests/core_lowering.rs](../../../tests/core_lowering.rs)'s `nodes::walk` (`:33-60`), after the `Let` arm:

```rust
            CoreKind::If(cond, then_e, else_e) => {
                walk(cond, out);
                walk(then_e, out);
                walk(else_e, out);
            }
```

- [x] **Step 7: Add the temporary codegen refusal**

In [crates/codegen/src/lib.rs](../../../crates/codegen/src/lib.rs), in `lower_expr`'s match, alongside the other refusals at `:172-174`:

```rust
        // Temporary: Core can build `If` (5b-2 Task 1) before the back end can
        // lower it. Task 4 replaces this with the diamond.
        CoreKind::If(..) => Err(CodegenError::Unsupported("If")),
```

- [x] **Step 8: Run the tests and watch them pass**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test core_lowering
```

Expected: PASS, including all seven `surfaceN_*` snapshot tests — no 5a-2 corpus program contains an `if`, so no `.snap` file changes. If insta reports a pending snapshot, something is wrong; stop and investigate rather than accepting it.

- [x] **Step 9: Format, gate, commit**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo fmt --all
$env:CARGO_INCREMENTAL="0"; pwsh scripts/check.ps1
if ($LASTEXITCODE -eq 0) {
  graphify update .
  git add src/core.rs tests/core_lowering.rs crates/codegen/src/lib.rs
  git commit -m @'
feat(core): Core grows an If node (5b-2 Task 1)

`Expr::If` lowered to `Unsupported("If")` because `CoreKind` had no `If`
variant at all — the spec's "one Core arm" understated the reach. Adds the
variant, the lowering arm (condition via lower_expr, both blocks via
lower_block, mirroring Expr::Lambda), the pretty_expr rendering, and the
test walker arm. Codegen refuses `If` by name for now; Task 4 lowers it.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
'@
  git push origin main
}
```

---

### Task 2: The evaluator hands back `main`'s value

**This is the flagged deviation from spec §6.3** — read "Deliberate deviations" #2 above before implementing. Standalone and LLVM-free, so it can be rejected without touching the back end. `elya run` behaviour must be provably unchanged, and this task's test is what proves it.

**Files:**
- Modify: [src/eval.rs](../../../src/eval.rs) — `cek::run_loop` (`:650-663`), `cek::run_module` (`:589-599`), and the top-level re-export near `:258`
- Test: `tests/eval_value.rs` (create)

**Interfaces:**
- Consumes: `cek::run_loop(&mut Interp, &Fns, &Ops, State) -> Result<(), RuntimeError>` (signature changes here); `cek::State::{Eval, Return}`; `rt(Span, &str) -> RuntimeError`.
- Produces: `pub fn elya::eval::run_module_value(module: &Module) -> Result<(Interp, Value), RuntimeError>` — same machine, same order, same errors as `run_module`; the only difference is that `main`'s value is not discarded. Task 5's differential harness is its only consumer.

- [x] **Step 1: Write the failing test**

Create `tests/eval_value.rs`:

```rust
//! The accessor that makes the native back end's differential check possible
//! (5b-2 §5): `elya run` observes only `io.println` output, so nothing public
//! could say what `main` evaluated to. `run_module_value` says it — and
//! `run_module` must keep behaving exactly as before.

use elya::eval::{run_module, run_module_value, Value};
use elya::parse::parse_module;
use elya::Session;

fn module_of(src: &str) -> elya::ast::Module {
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    m
}

#[test]
fn run_module_value_reports_what_main_evaluated_to() {
    let m = module_of("pub fn main() { (2 + 3) * 4 - 5 }\n");
    let (_, v) = run_module_value(&m).expect("evaluates");
    assert_eq!(v, Value::Int(15));
}

#[test]
fn run_module_value_reports_non_int_results_too() {
    let m = module_of("pub fn main() { 1 < 2 }\n");
    let (_, v) = run_module_value(&m).expect("evaluates");
    assert_eq!(v, Value::Bool(true));
}

#[test]
fn run_module_still_observes_only_println_output() {
    // The whole point of the accessor is that it is additive: `elya run` sees
    // exactly what it saw before, so the differential check stays non-circular.
    let m = module_of("pub fn main() {\n  io.println(\"hi\")\n  1 + 2\n}\n");
    let interp = run_module(&m).expect("evaluates");
    assert_eq!(interp.output(), "hi\n");

    let (interp2, v) = run_module_value(&m).expect("evaluates");
    assert_eq!(interp2.output(), interp.output(), "same machine, same output");
    assert_eq!(v, Value::Int(3), "and the value the output never showed");
}

#[test]
fn run_module_value_surfaces_runtime_errors_unchanged() {
    let m = module_of("pub fn notmain() { 1 }\n");
    assert!(run_module(&m).is_err());
    assert!(run_module_value(&m).is_err(), "same failure, same door");
}
```

- [x] **Step 2: Run it and watch it fail**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test eval_value
```

Expected: FAIL to compile — `run_module_value` is not a member of `elya::eval`.

- [x] **Step 3: Make `run_loop` return the value it already has**

In [src/eval.rs](../../../src/eval.rs), replace `cek::run_loop` (`:650-663`) with:

```rust
    fn run_loop(
        interp: &mut Interp,
        fns: &Fns,
        ops: &Ops,
        mut st: State,
    ) -> Result<Value, RuntimeError> {
        loop {
            interp.note_kont_depth(kont_len(kont_of(&st)));
            // The machine halts exactly when a `Return` meets an empty
            // continuation, and that value is `main`'s result. Catching it here
            // rather than letting `step` fall off the end is what lets the value
            // escape the loop at all; `run_module` still discards it, so nothing
            // about `elya run` changes.
            st = match st {
                State::Return(v, None) => return Ok(v),
                other => match step(interp, fns, ops, other)? {
                    Some(next) => next,
                    // `ret` returns `None` only for an empty continuation, which
                    // the arm above already caught. Defensive, not expected.
                    None => {
                        return Err(rt(
                            Span::EMPTY,
                            "internal: machine halted with frames pending",
                        ))
                    }
                },
            };
        }
    }
```

The `note_kont_depth` call stays first, before the match, so the peak-depth bookkeeping the TCE tests assert on ([tests/tce.rs:19](../../../tests/tce.rs#L19), `:30`; [tests/state_effect.rs:77](../../../tests/state_effect.rs#L77)) observes the identical sequence of states it did before.

- [x] **Step 4: Split `cek::run_module` into a value-returning core plus the existing wrapper**

In [src/eval.rs](../../../src/eval.rs), replace `cek::run_module` (`:589-599`) with:

```rust
    /// Run `main` and hand back both the interpreter and the value `main`
    /// evaluated to. `run_module` throws that value away — `elya run` observes
    /// only `io.println` output — but a compiled binary prints it, so the back
    /// end's differential check needs a way to ask what it should have been.
    pub fn run_module_value(module: &Module) -> Result<(Interp, Value), RuntimeError> {
        let fns = fn_table(module);
        let ops = op_table(module);
        let mut interp = Interp::new();
        let Some(main) = fns.get("main").copied() else {
            return Err(rt(Span::EMPTY, "no `main` function found"));
        };
        let start = eval_block_state(&main.body.node, Env::new(), None);
        let v = run_loop(&mut interp, &fns, &ops, start)?;
        Ok((interp, v))
    }

    pub fn run_module(module: &Module) -> Result<Interp, RuntimeError> {
        run_module_value(module).map(|(interp, _)| interp)
    }
```

- [x] **Step 5: Re-export at the module's top level**

In [src/eval.rs](../../../src/eval.rs), beside the existing `pub fn run_module` / `pub fn run_module_tree` (`:258-264`):

```rust
/// Run `main` under the CEK machine and return its value alongside the
/// interpreter. Same machine, same evaluation order, same errors as
/// [`run_module`]; the only difference is that the result is not discarded.
pub fn run_module_value(module: &Module) -> Result<(Interp, Value), RuntimeError> {
    cek::run_module_value(module)
}
```

- [x] **Step 6: Run the tests and watch them pass**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test eval_value
$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test tce --test state_effect
```

Expected: PASS for both. The TCE tests are named explicitly because they assert on measured peak continuation depth at 1e6 recursion — the one thing a change to `run_loop` could plausibly perturb.

- [x] **Step 7: Format, gate, commit**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo fmt --all
$env:CARGO_INCREMENTAL="0"; pwsh scripts/check.ps1
if ($LASTEXITCODE -eq 0) {
  graphify update .
  git add src/eval.rs tests/eval_value.rs
  git commit -m @'
feat(eval): expose main's value for the back end's differential check (5b-2 Task 2)

`elya run` observes only io.println output — both evaluators drop main's
value at `ret`'s empty-continuation exit — so "compare native output against
elya run" was vacuous for every program in the codegen corpus. `run_loop`
now returns the value it already held, and `run_module_value` hands it back.

`run_module` delegates and is unchanged: same machine, same order, same
errors, same `Interp::output`. Deviates from the 5b-2 spec's §6.3 letter
("eval.rs is not modified") while preserving its stated purpose, that
`elya run` behaviour stay bit-for-bit identical.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
'@
  git push origin main
}
```

---

### Task 3: Widen the codegen value representation to `Int` and `Bool`

Deliberately no new control flow: this task changes how values are *typed* in the fold and nothing about what it can lower. The 5b-1 corpus is the regression suite, and it must pass untouched.

The key move is that `require_int` stops guarding every node and starts guarding one thing only — `main`'s **return** type. `@elya_main` returns i64 and the print shim's format string is `%lld`; a `Bool`-bodied `main` is a representable value in an unrepresentable *place*, so it is refused at the boundary rather than inside the fold. That keeps `rejects_bool_literal_specifically`'s exact message.

**Files:**
- Modify: [crates/codegen/src/lib.rs](../../../crates/codegen/src/lib.rs) — `require_int` (`:110-120`), `lower_expr` signature and body (`:126-176`), `build_module` (`:182-192`), and the `mod tests` block

**Interfaces:**
- Consumes: `CoreExpr { span, ty, kind }` with the type carried inline (Shape C); `Ty::Base(TyCon::{Int, Bool, Str, Float, Unit})`; `Ty::Var(u32)`.
- Produces: `fn repr_ty<'ctx>(ctx: &'ctx Context, ty: &Ty) -> Result<IntType<'ctx>, CodegenError>` — `Int` → `i64`, `Bool` → `i1`, everything else `Unsupported("unrepresentable type")`. And `lower_expr<'ctx>(ctx: &'ctx Context, b: &Builder<'ctx>, e: &CoreExpr, env: &mut HashMap<String, IntValue<'ctx>>) -> Result<IntValue<'ctx>, CodegenError>` — the `i64t` parameter is gone, replaced by `ctx`, because a node's width now depends on its own type. Task 4 adds one more parameter.

- [x] **Step 1: Write the failing test**

In [crates/codegen/src/lib.rs](../../../crates/codegen/src/lib.rs)'s `mod tests`, after `rejects_bool_literal_specifically`:

```rust
    #[test]
    fn a_bool_binding_is_representable_even_though_a_bool_main_is_not() {
        // Layer-1 teeth only — an i1 constant costs no instruction, so this
        // proves the type mapping accepts Bool, not that anything computes with
        // it. The real proof is Task 5's execution corpus. Hand-built because no
        // source program can produce a Bool-typed node until Task 4 lands `if`.
        let m = main_fn(CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Let(
                "b".into(),
                Rc::new(CoreExpr {
                    span: Span::EMPTY,
                    ty: Ty::Base(TyCon::Bool),
                    kind: CoreKind::Lit(CoreLit::Bool(true)),
                }),
                Rc::new(int_lit(1)),
            ),
        });
        emit_ir(&m).expect("a Bool binding must verify");
    }

    #[test]
    fn rejects_a_string_typed_node_by_name() {
        // The widening is exactly two widths wide. Str is not one of them, and
        // it is refused as an unrepresentable *type*, distinct from the
        // "non-Int value" that guards main's return type.
        let m = main_fn(CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Let(
                "s".into(),
                Rc::new(CoreExpr {
                    span: Span::EMPTY,
                    ty: Ty::Base(TyCon::Str),
                    kind: CoreKind::Lit(CoreLit::Str("a".into())),
                }),
                Rc::new(int_lit(1)),
            ),
        });
        let err = emit_ir(&m).unwrap_err();
        assert!(
            matches!(err, CodegenError::Unsupported("unrepresentable type")),
            "{err:?}"
        );
    }
```

- [x] **Step 2: Run them and watch them fail**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --lib -j 2
```

Expected: `a_bool_binding_is_representable_even_though_a_bool_main_is_not` FAILS with `Unsupported("non-Int value")` (today's `require_int` rejects the Bool node inside the fold), and `rejects_a_string_typed_node_by_name` FAILS on the message, getting `"non-Int value"` where it wants `"unrepresentable type"`.

- [x] **Step 3: Replace `require_int`'s role with `repr_ty`**

In [crates/codegen/src/lib.rs](../../../crates/codegen/src/lib.rs), replace the `require_int` block at `:110-120` with both functions:

```rust
/// §3.1 type mapping, widened for N3: `Int` → i64, `Bool` → i1. Reads the
/// INLINE `ty` field on each Core node (Shape C — the reason this fold needs no
/// side-table lookups). Everything else is refused by name, `Ty::Var(_)`
/// included; when that fires, that is N7 knocking.
///
/// A toe-in, not the value-representation decision: two integer widths is the
/// least that lets a branch have a condition. Heap values arrive with N4.
fn repr_ty<'ctx>(ctx: &'ctx Context, ty: &Ty) -> Result<IntType<'ctx>, CodegenError> {
    match ty {
        Ty::Base(TyCon::Int) => Ok(ctx.i64_type()),
        Ty::Base(TyCon::Bool) => Ok(ctx.bool_type()),
        _ => Err(CodegenError::Unsupported("unrepresentable type")),
    }
}

/// `main` returns i64: `@elya_main`'s signature says so and the print shim's
/// format string is `%lld`. A `Bool`-bodied main is a representable value in an
/// unrepresentable *place*, so it is refused at the module boundary rather than
/// inside the fold — which is why this message stayed "non-Int value" when the
/// fold widened.
fn require_int(ty: &Ty) -> Result<(), CodegenError> {
    if matches!(ty, Ty::Base(TyCon::Int)) {
        Ok(())
    } else {
        Err(CodegenError::Unsupported("non-Int value"))
    }
}
```

- [x] **Step 4: Rewrite the fold's signature and its type-dependent leaves**

In [crates/codegen/src/lib.rs](../../../crates/codegen/src/lib.rs), change `lower_expr`'s doc comment, signature, and the three arms that depend on the node's width. Every recursive call site inside the function loses its `i64t` argument and gains `ctx`; the `Let` and `Prim` arms are otherwise untouched.

```rust
/// §3.3 expression lowering: a recursive fold returning an `IntValue`, threading
/// a binding environment. NO alloca, NO mem2reg — bindings are immutable and
/// values map directly to SSA registers; the save/restore around `Let` is what
/// makes shadowing correct. Each node's width comes from its own inline type, so
/// an i1 and an i64 register coexist without a wrapper enum: inkwell's
/// `IntValue` already carries its width.
fn lower_expr<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    e: &CoreExpr,
    env: &mut HashMap<String, IntValue<'ctx>>,
) -> Result<IntValue<'ctx>, CodegenError> {
    let node_ty = repr_ty(ctx, &e.ty)?;
    match &e.kind {
        CoreKind::Lit(CoreLit::Int(n)) => Ok(node_ty.const_int(*n as u64, true)),
        CoreKind::Lit(CoreLit::Bool(v)) => Ok(node_ty.const_int(u64::from(*v), false)),
        CoreKind::Lit(_) => Err(CodegenError::Unsupported("non-Int literal")),
        CoreKind::Var(x) => env
            .get(x)
            .copied()
            .ok_or(CodegenError::Unsupported("unbound var")),
        CoreKind::Let(x, rhs, body) => {
            let v = lower_expr(ctx, b, rhs, env)?;
            let prev = env.insert(x.clone(), v);
            let out = lower_expr(ctx, b, body, env);
            // Restore any shadowed binding — `let x = 1; let x = x + 1` stays correct.
            match prev {
                Some(p) => {
                    env.insert(x.clone(), p);
                }
                None => {
                    env.remove(x);
                }
            }
            out
        }
        CoreKind::Prim(op, args) => {
            if args.len() != 2 {
                return Err(CodegenError::Unsupported("binary Prim arity"));
            }
            let l = lower_expr(ctx, b, &args[0], env)?;
            let r = lower_expr(ctx, b, &args[1], env)?;
            // Deliberately NO nsw/nuw flags: defined two's-complement wrapping
            // (§3.4). Overflow reconciliation with the evaluator is tracked in
            // spec §11 — not silently decided here.
            let built = match op {
                BinOp::Add => b.build_int_add(l, r, "add"),
                BinOp::Sub => b.build_int_sub(l, r, "sub"),
                BinOp::Mul => b.build_int_mul(l, r, "mul"),
                other => return Err(CodegenError::Unsupported(op_label(*other))),
            };
            built.map_err(internal)
        }
        CoreKind::If(..) => Err(CodegenError::Unsupported("If")),
        CoreKind::App(..) => Err(CodegenError::Unsupported("App")),
        CoreKind::Lambda(..) => Err(CodegenError::Unsupported("Lambda")),
        CoreKind::Match(..) => Err(CodegenError::Unsupported("Match")),
    }
}
```

The `CoreKind::Lit(_)` arm survives, now narrowed to `Str` and `Unit`. It is reachable only for a `Str`/`Unit`-typed literal, which `repr_ty` refuses one line earlier — kept as a belt-and-braces boundary rather than deleted, so a future widening of `repr_ty` cannot silently mis-lower a literal.

- [x] **Step 5: Move the Int guard to `build_module`**

In [crates/codegen/src/lib.rs](../../../crates/codegen/src/lib.rs)'s `build_module` (`:182-192`), replace the `i64t`/`lower_expr` lines:

```rust
    let f = validate_module(core)?;
    // §3.1: `main` returns i64. The fold now speaks two widths, so this is the
    // one place that still insists on Int.
    require_int(&f.body.ty)?;
    let i64t = ctx.i64_type();
    let module = ctx.create_module("elya");
    let func = module.add_function("elya_main", i64t.fn_type(&[], false), None);
    let entry = ctx.append_basic_block(func, "entry");
    let b = ctx.create_builder();
    b.position_at_end(entry);
    let mut env = HashMap::new();
    let result = lower_expr(ctx, &b, &f.body, &mut env)?;
    b.build_return(Some(&result)).map_err(internal)?;
```

- [x] **Step 6: Run the tests and watch them pass**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen -j 2
```

Expected: PASS — the two new tests, plus every 5b-1 test unchanged. `rejects_bool_literal_specifically` still gets `Unsupported("non-Int value")`, now from `build_module` instead of from the fold; `corpus_verifies` and all four execution tests (`spine_prints_three` → "3", `lets_and_mul_print_forty_two` → "42", `nesting_and_sub_print_fifteen` → "15", `negative_result_prints_minus_seven` → "-7") are the regression suite for this refactor.

- [x] **Step 7: Format, gate, commit**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo fmt --all
$env:CARGO_INCREMENTAL="0"; pwsh scripts/check.ps1
if ($LASTEXITCODE -eq 0) {
  graphify update .
  git add crates/codegen/src/lib.rs
  git commit -m @'
refactor(codegen): widen the value representation to Int and Bool (5b-2 Task 3)

The fold typed every node i64 via `require_int`. It now reads each node's
inline type through `repr_ty` — Int to i64, Bool to i1, everything else
refused as an unrepresentable type. `IntValue` already carries its width, so
two widths coexist without a wrapper enum.

`require_int` keeps its message and moves to where it is actually true:
main's return type, because @elya_main returns i64 and the shim prints %lld.
No new control flow; the 5b-1 execution corpus is the regression suite.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
'@
  git push origin main
}
```

---

### Task 4: The diamond, the comparisons, and strict `&&`/`||`

The back end learns to branch. Three pieces land together because none of them is testable alone: an `if` with no comparison has no condition to compute, and a comparison with no `if` produces an i1 that `main` cannot return.

**Files:**
- Modify: [crates/codegen/src/lib.rs](../../../crates/codegen/src/lib.rs) — module doc (`:1-10`), imports (`:15-24`), `lower_expr` (signature, `Prim` arm, `If` arm), and the `mod tests` block

**Interfaces:**
- Consumes: `repr_ty`, `require_int`, `op_label`, `internal` from Task 3; `CoreKind::If(cond, then, else)` from Task 1.
- Produces: `lower_expr<'ctx>(ctx: &'ctx Context, func: FunctionValue<'ctx>, b: &Builder<'ctx>, e: &CoreExpr, env: &mut HashMap<String, IntValue<'ctx>>) -> Result<IntValue<'ctx>, CodegenError>` — one more parameter than Task 3, because appending a basic block needs the function to append it to. Also `fn eq_operand_label(op: BinOp) -> &'static str`, returning one of two fixed strings; `CodegenError::Unsupported` holds `&'static str`, so these cannot be `format!`ed.

- [x] **Step 1: Write the failing tests**

In [crates/codegen/src/lib.rs](../../../crates/codegen/src/lib.rs)'s `mod tests`, add the `if_expr` helper beside `prim` and `int_lit`:

```rust
    fn if_expr(c: CoreExpr, t: CoreExpr, e: CoreExpr) -> CoreExpr {
        CoreExpr {
            span: Span::EMPTY,
            ty: t.ty.clone(),
            kind: CoreKind::If(Rc::new(c), Rc::new(t), Rc::new(e)),
        }
    }

    fn cmp(op: BinOp, l: CoreExpr, r: CoreExpr) -> CoreExpr {
        CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Bool),
            kind: CoreKind::Prim(op, vec![l, r].into()),
        }
    }
```

Then **delete `rejects_and_specifically`** (`:349-353`) — `And` is supported from this task on, so the test now asserts the opposite of the truth — and add:

```rust
    #[test]
    fn an_if_diamond_verifies() {
        let m = main_fn(if_expr(
            cmp(BinOp::Lt, int_lit(1), int_lit(2)),
            int_lit(10),
            int_lit(20),
        ));
        emit_ir(&m).expect("the diamond must produce verifier-clean IR");
    }

    #[test]
    fn a_nested_if_in_a_branch_verifies() {
        // The phi trap (§3.2): the inner `if` leaves the builder in ITS join
        // block, so the outer phi must name that block, not `then`. Getting it
        // wrong is a verifier error, which is why this is worth a Layer-1 test
        // even though execution is the real proof.
        let inner = if_expr(
            cmp(BinOp::Gt, int_lit(7), int_lit(5)),
            int_lit(100),
            int_lit(50),
        );
        let m = main_fn(if_expr(
            cmp(BinOp::Gt, int_lit(7), int_lit(0)),
            inner,
            int_lit(0),
        ));
        emit_ir(&m).expect("a nested diamond must produce verifier-clean IR");
    }

    #[test]
    fn strict_and_or_verify() {
        let m = main_fn(if_expr(
            CoreExpr {
                span: Span::EMPTY,
                ty: Ty::Base(TyCon::Bool),
                kind: CoreKind::Prim(
                    BinOp::And,
                    vec![
                        cmp(BinOp::Lt, int_lit(1), int_lit(2)),
                        cmp(BinOp::Gt, int_lit(3), int_lit(4)),
                    ]
                    .into(),
                ),
            },
            int_lit(1),
            int_lit(0),
        ));
        emit_ir(&m).expect("strict and must produce verifier-clean IR");
    }

    #[test]
    fn rejects_eq_on_an_unrepresentable_operand_type() {
        // Eq/Ne are the only fully polymorphic operators in the subset: the
        // checker is happy with `Str == Str` and there is no representation for
        // it. Dispatched on the OPERAND type before the operands are lowered, so
        // the message names the operator (§3.3) rather than the operand. Wrapped
        // in an `if` because a Bool-bodied main is refused earlier, by
        // `require_int`, with a different message.
        let s = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Str),
            kind: CoreKind::Lit(CoreLit::Str("a".into())),
        };
        let bad = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Bool),
            kind: CoreKind::Prim(BinOp::Eq, vec![s.clone(), s].into()),
        };
        let err = emit_ir(&main_fn(if_expr(bad, int_lit(1), int_lit(0)))).unwrap_err();
        assert!(
            matches!(
                err,
                CodegenError::Unsupported("Eq on an unrepresentable operand type")
            ),
            "{err:?}"
        );
    }

    #[test]
    fn accepts_eq_on_bool_operands() {
        // The other side of the same door: Bool is a concrete type with a
        // representation, so `True == False` is an icmp on i1, not a refusal.
        let t = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Bool),
            kind: CoreKind::Lit(CoreLit::Bool(true)),
        };
        let f = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Bool),
            kind: CoreKind::Lit(CoreLit::Bool(false)),
        };
        let m = main_fn(if_expr(cmp(BinOp::Eq, t, f), int_lit(1), int_lit(2)));
        emit_ir(&m).expect("Eq on Bool operands must verify");
    }
```

- [x] **Step 2: Run them and watch them fail**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --lib -j 2
```

Expected: the four new `verifies`/`accepts` tests FAIL with `Unsupported("If")` (Task 1's temporary arm), and `rejects_eq_on_an_unrepresentable_operand_type` FAILS the same way rather than with its expected message.

- [x] **Step 3: Add the imports**

In [crates/codegen/src/lib.rs](../../../crates/codegen/src/lib.rs), alongside the existing inkwell imports at `:15-24`:

```rust
use inkwell::values::FunctionValue;
use inkwell::IntPredicate;
```

- [x] **Step 4: Add the operand-refusal label**

In [crates/codegen/src/lib.rs](../../../crates/codegen/src/lib.rs), after `op_label` (`:68-89`):

```rust
/// `CodegenError::Unsupported` carries a `&'static str`, so the Eq/Ne operand
/// refusal is a fixed pair of strings rather than a formatted type name. Losing
/// the type name is the price of refusing by name at all — and the type is one
/// line up in any backtrace the user would be reading.
fn eq_operand_label(op: BinOp) -> &'static str {
    match op {
        BinOp::Ne => "Ne on an unrepresentable operand type",
        _ => "Eq on an unrepresentable operand type",
    }
}
```

- [x] **Step 5: Thread the function through the fold**

In [crates/codegen/src/lib.rs](../../../crates/codegen/src/lib.rs), add `func` to `lower_expr`'s signature and to every recursive call inside it (the `Let` arm's two calls and the `Prim` arm's two):

```rust
fn lower_expr<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    e: &CoreExpr,
    env: &mut HashMap<String, IntValue<'ctx>>,
) -> Result<IntValue<'ctx>, CodegenError> {
```

and in `build_module`, `let result = lower_expr(ctx, func, &b, &f.body, &mut env)?;`.

- [x] **Step 6: Extend the `Prim` arm with comparisons and strict logicals**

Replace the `CoreKind::Prim` arm's body in [crates/codegen/src/lib.rs](../../../crates/codegen/src/lib.rs):

```rust
        CoreKind::Prim(op, args) => {
            if args.len() != 2 {
                return Err(CodegenError::Unsupported("binary Prim arity"));
            }
            // Eq/Ne are fully polymorphic (src/types.rs:920-922 unifies the two
            // operands and pins neither), so the refusal is dispatched on the
            // OPERAND type — and BEFORE the operands are lowered, so the message
            // names the operator rather than reporting the operand's type as
            // unrepresentable. Every other operator in the subset is monomorphic
            // by the time it reaches here.
            if matches!(op, BinOp::Eq | BinOp::Ne)
                && !matches!(
                    args[0].ty,
                    Ty::Base(TyCon::Int) | Ty::Base(TyCon::Bool)
                )
            {
                return Err(CodegenError::Unsupported(eq_operand_label(*op)));
            }
            let l = lower_expr(ctx, func, b, &args[0], env)?;
            let r = lower_expr(ctx, func, b, &args[1], env)?;
            // Deliberately NO nsw/nuw flags: defined two's-complement wrapping
            // (§3.4). Overflow reconciliation with the evaluator is tracked in
            // spec §11 — not silently decided here.
            //
            // Comparisons are SIGNED: Elya's Int is i64 two's-complement, so
            // `(0 - 1) < 1` must be true. `and`/`or` are strict and bit-wise on
            // i1 because BOTH evaluators are strict (spec §4.1) — a
            // short-circuit diamond here would make native less-undefined than
            // `elya run`, which is the mirror image of the Div trade.
            let built = match op {
                BinOp::Add => b.build_int_add(l, r, "add"),
                BinOp::Sub => b.build_int_sub(l, r, "sub"),
                BinOp::Mul => b.build_int_mul(l, r, "mul"),
                BinOp::Lt => b.build_int_compare(IntPredicate::SLT, l, r, "lt"),
                BinOp::Le => b.build_int_compare(IntPredicate::SLE, l, r, "le"),
                BinOp::Gt => b.build_int_compare(IntPredicate::SGT, l, r, "gt"),
                BinOp::Ge => b.build_int_compare(IntPredicate::SGE, l, r, "ge"),
                BinOp::Eq => b.build_int_compare(IntPredicate::EQ, l, r, "eq"),
                BinOp::Ne => b.build_int_compare(IntPredicate::NE, l, r, "ne"),
                BinOp::And => b.build_and(l, r, "and"),
                BinOp::Or => b.build_or(l, r, "or"),
                other => return Err(CodegenError::Unsupported(op_label(*other))),
            };
            built.map_err(internal)
        }
```

- [x] **Step 7: Replace the `If` refusal with the diamond**

In [crates/codegen/src/lib.rs](../../../crates/codegen/src/lib.rs), replace `CoreKind::If(..) => Err(CodegenError::Unsupported("If")),`:

```rust
        CoreKind::If(cond, then_e, else_e) => {
            let c = lower_expr(ctx, func, b, cond, env)?;
            // The checker unifies the condition with Bool, so §3.1's mapping
            // makes it i1. Anything else is a bug in our own lowering, surfaced
            // rather than handed to `build_conditional_branch`.
            if c.get_type().get_bit_width() != 1 {
                return Err(CodegenError::Unsupported("non-Bool if condition"));
            }
            let then_bb = ctx.append_basic_block(func, "then");
            let else_bb = ctx.append_basic_block(func, "else");
            let join_bb = ctx.append_basic_block(func, "ifcont");
            b.build_conditional_branch(c, then_bb, else_bb)
                .map_err(internal)?;

            b.position_at_end(then_bb);
            let tv = lower_expr(ctx, func, b, then_e, env)?;
            // THE TRAP (§3.2): a nested `if` inside this branch left the builder
            // in ITS join block, not in `then_bb`. `phi` names the block control
            // actually flows FROM, so read the exit block back from the builder
            // instead of assuming it is the block we positioned at.
            let then_exit = b
                .get_insert_block()
                .ok_or(CodegenError::Unsupported("builder left no block"))?;
            b.build_unconditional_branch(join_bb).map_err(internal)?;

            b.position_at_end(else_bb);
            let ev = lower_expr(ctx, func, b, else_e, env)?;
            let else_exit = b
                .get_insert_block()
                .ok_or(CodegenError::Unsupported("builder left no block"))?;
            b.build_unconditional_branch(join_bb).map_err(internal)?;

            b.position_at_end(join_bb);
            // Exactly two incoming values, always: the AST's `else_block` is not
            // an Option, so there is no one-armed `if` to synthesize a Unit
            // branch for. Both branches carry the same type — the checker
            // unified them — so one phi type is correct.
            let phi = b.build_phi(tv.get_type(), "iftmp").map_err(internal)?;
            phi.add_incoming(&[(&tv, then_exit), (&ev, else_exit)]);
            Ok(phi.as_basic_value().into_int_value())
        }
```

- [x] **Step 8: Correct the module doc**

In [crates/codegen/src/lib.rs:1-10](../../../crates/codegen/src/lib.rs#L1-L10), replace the header comment:

```rust
//! Native codegen: Core → LLVM via inkwell. Slice 5b-1 covered the arithmetic
//! subset (Int literals, Var, Prim(Add/Sub/Mul), Let); Slice 5b-2 adds control
//! flow — `If` as a three-block diamond joined by `phi`, `Bool` as i1, the six
//! comparisons as signed `icmp`, and `&&`/`||` as bit-wise `and`/`or` on i1.
//! Two value widths (i64, i1), one function (`@elya_main`), no effects.
//!
//! Proof is EXECUTION (tests/native_codegen.rs), never IR inspection (spec §0):
//! `emit_ir` is a debugging aid and nothing in the suite asserts on its output.
//! Semantic-fidelity rule (§3.4, §4.1): native codegen must be neither more- nor
//! less-undefined than the tree evaluator. Hence no Div/Rem (UB on a zero
//! divisor, and it drags in the runtime-error path), Add/Sub/Mul emitted WITHOUT
//! nsw/nuw so overflow is defined two's-complement wrapping, and `&&`/`||`
//! STRICT rather than short-circuiting — because both of Elya's evaluators are
//! strict, so a short-circuit diamond would make native binaries *less*
//! undefined than `elya run`, observable the moment Div lands. Short-circuiting
//! is a front-end question, not a back-end one.
```

- [x] **Step 9: Run the tests and watch them pass**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen -j 2
```

Expected: PASS, all of it — the five new unit tests, the 5b-1 unit tests (`rejects_div_specifically`, `rejects_bool_literal_specifically`, `rejects_lambda_specifically`, `rejects_multi_function_module_specifically`, `rejects_parameterised_main_specifically`, `let_shadowing_restores_prior_binding`, `corpus_verifies`), and the four 5b-1 execution tests.

- [x] **Step 10: Format, gate, commit**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo fmt --all
$env:CARGO_INCREMENTAL="0"; pwsh scripts/check.ps1
if ($LASTEXITCODE -eq 0) {
  graphify update .
  git add crates/codegen/src/lib.rs
  git commit -m @'
feat(codegen): if-diamond, signed comparisons, strict and/or (5b-2 Task 4)

`If` lowers to a three-block diamond joined by a phi that reads its incoming
blocks back from the builder, not from the blocks it positioned at — a nested
`if` leaves the builder in the inner join block, and naming the wrong block is
the classic phi bug.

Comparisons are signed icmp, so (0 - 1) < 1 is true. Eq/Ne are the only
polymorphic operators in the subset, so they dispatch on the operand type and
refuse anything but Int/Bool by name, before lowering the operands.

`&&`/`||` are STRICT bit-wise and/or on i1. Both evaluators are strict, so a
short-circuit diamond would make native less-undefined than `elya run` — the
mirror image of the Div trade. Corrects the module doc, which had inherited
the same wrong short-circuit assumption from the 5b-1 spec.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
'@
  git push origin main
}
```

---

### Task 5: The execution corpus and the differential check

The deliverable. Seven source programs, each compiled to a real native binary, run, and checked twice: against a written-down expected value, and against what the evaluator says `main` is worth.

**Files:**
- Modify: [crates/codegen/tests/native_codegen.rs](../../../crates/codegen/tests/native_codegen.rs) — new corpus const and tests, reusing the existing `temp_dir`, `lower_src`, `compile_and_link`, `assert_runs` helpers
- Modify: [crates/codegen/src/lib.rs](../../../crates/codegen/src/lib.rs) — extend `corpus_verifies`'s inline corpus

**Interfaces:**
- Consumes: `elya::eval::run_module_value` (Task 2); `elya_codegen::{compile_module, link}`; `temp_dir(&str) -> PathBuf`, `lower_src(&str) -> CoreModule`, `compile_and_link(&CoreModule, &Path, &str) -> PathBuf`, `assert_runs(&Path, &str)` from the existing file.
- Produces: `const CONTROL_FLOW_CORPUS: &[(&str, &str, &str)]` — (tag, source, expected stdout); `fn eval_main_int(src: &str) -> String`.

- [x] **Step 1: Write the failing tests**

Append to [crates/codegen/tests/native_codegen.rs](../../../crates/codegen/tests/native_codegen.rs):

```rust
/// The 5b-2 §5 corpus: (tag, source, expected stdout). Seven programs, each
/// aimed at one thing the diamond can get wrong.
const CONTROL_FLOW_CORPUS: &[(&str, &str, &str)] = &[
    // 1-2: both directions of the same branch, so a diamond that always takes
    // one side fails one of them.
    (
        "if_true",
        "pub fn main() { if 1 < 2 { 10 } else { 20 } }\n",
        "10",
    ),
    (
        "if_false",
        "pub fn main() { if 2 < 1 { 10 } else { 20 } }\n",
        "20",
    ),
    // 3: a binding live across the branch — the env must survive the diamond.
    (
        "if_over_a_binding",
        "pub fn main() {\n  let x = 5\n  if x > 3 { x * 2 } else { 0 }\n}\n",
        "10",
    ),
    // 4: THE PHI TRAP, on both sides. `a` nests inside the then-branch, `b`
    // inside the else-branch; an outer phi that names `then`/`else` instead of
    // the inner join blocks miscompiles or fails the verifier.
    (
        "nested_if",
        "pub fn main() {\n  let x = 7\n  let a = if x > 0 { if x > 5 { 100 } else { 50 } } else { 0 }\n  let b = if x < 0 { 0 } else { if x > 5 { 7 } else { 3 } }\n  a + b\n}\n",
        "107",
    ),
    // 5: all six predicates, each at its boundary case, so swapping SLT for SLE
    // (or SGT for SGE) changes the answer. Plus one signedness probe: under an
    // unsigned compare `(0 - 1) < 1` is false, and the total drops to 6.
    (
        "predicates",
        "pub fn main() {\n  let a = if 1 < 1 { 1 } else { 0 }\n  let b = if 1 < 2 { 1 } else { 0 }\n  let c = if 1 <= 1 { 1 } else { 0 }\n  let d = if 2 <= 1 { 1 } else { 0 }\n  let e = if 1 > 1 { 1 } else { 0 }\n  let f = if 2 > 1 { 1 } else { 0 }\n  let g = if 1 >= 1 { 1 } else { 0 }\n  let h = if 1 >= 2 { 1 } else { 0 }\n  let i = if 1 == 1 { 1 } else { 0 }\n  let j = if 1 == 2 { 1 } else { 0 }\n  let k = if 1 != 2 { 1 } else { 0 }\n  let m = if 1 != 1 { 1 } else { 0 }\n  let n = if (0 - 1) < 1 { 1 } else { 0 }\n  a + b + c + d + e + f + g + h + i + j + k + m + n\n}\n",
        "7",
    ),
    // 6: Eq on Bool operands — the i1 path through the polymorphic operator.
    (
        "bool_equality",
        "pub fn main() { if True == False { 1 } else { 2 } }\n",
        "2",
    ),
    // 7: strict and/or over two comparisons, both truth values of each.
    (
        "and_or",
        "pub fn main() {\n  let a = if 1 < 2 && 3 > 4 { 1 } else { 0 }\n  let b = if 1 < 2 && 3 < 4 { 1 } else { 0 }\n  let c = if 1 > 2 || 3 > 4 { 1 } else { 0 }\n  let d = if 1 > 2 || 3 < 4 { 1 } else { 0 }\n  a + b + c + d\n}\n",
        "2",
    ),
];

/// The reference side of the differential check (§5): what the CEK evaluator
/// says `main` is worth, rendered the way the native print shim prints it
/// (`printf("%lld\n", …)`, trimmed by the caller).
///
/// `elya run` cannot serve here — it observes only `io.println` output and
/// discards main's value — which is why `run_module_value` exists.
fn eval_main_int(src: &str) -> String {
    let session = Session::new();
    let (m, pd) = parse_module(&session, src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (_, v) = elya::eval::run_module_value(&m).expect("evaluator must run corpus program");
    match v {
        elya::eval::Value::Int(n) => n.to_string(),
        other => panic!("corpus main must evaluate to an Int, got {other:?}"),
    }
}

#[test]
fn control_flow_corpus_compiles_links_and_runs() {
    let dir = temp_dir("control-flow");
    for (tag, src, expected) in CONTROL_FLOW_CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        assert_runs(&exe, expected);
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn native_output_matches_the_evaluator_across_the_control_flow_corpus() {
    // The fidelity teeth for the whole back-end arc: "native must match the
    // evaluator" stops being a remembered rule and becomes an enforced test.
    // This is what would have caught the strictness divergence automatically.
    let dir = temp_dir("differential");
    for (tag, src, _) in CONTROL_FLOW_CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        let out = Command::new(&exe).output().expect("run produced binary");
        assert!(out.status.success(), "{tag}: binary exited {:?}", out.status);
        let native = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(
            native,
            eval_main_int(src),
            "{tag}: native output diverges from the evaluator"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_differential_check_also_covers_the_arithmetic_corpus() {
    // The 5b-1 programs predate the check; running them through it costs one
    // loop and means the whole native surface is covered, not just the new part.
    let dir = temp_dir("differential-arith");
    for (tag, src) in CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        let out = Command::new(&exe).output().expect("run produced binary");
        assert!(out.status.success(), "{tag}: binary exited {:?}", out.status);
        let native = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(native, eval_main_int(src), "{tag}: native diverges");
    }
    std::fs::remove_dir_all(&dir).ok();
}
```

- [x] **Step 2: Run them and watch them fail**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen -j 2
```

Expected: PASS, in fact — Tasks 1–4 already built everything these tests exercise. That is the intended shape: this task's tests are the *deliverable*, written last because they are what the slice is for, and a green first run means Task 4's unit-level proofs did their job. **If any case fails, that is a real defect in Task 4, not in the test — fix the codegen, do not adjust the expected value.** The most likely failures and their meanings:
- `predicates` prints `6` → the signedness probe failed; a comparison is emitting an unsigned predicate.
- `predicates` prints something else → a boundary case; check SLT/SLE and SGT/SGE are not swapped.
- `nested_if` fails the verifier or prints `10x` → the phi is naming `then_bb`/`else_bb` instead of the exit blocks.
- `and_or` prints `1` or `3` → `and`/`or` swapped.

- [x] **Step 3: Extend the Layer-1 corpus check**

In [crates/codegen/src/lib.rs](../../../crates/codegen/src/lib.rs)'s `corpus_verifies` (`:319-341`), append the seven new sources to the inline `corpus` array. The existing comment already explains why these strings are duplicated from the integration test: integration targets cannot share consts, and a few lines of duplication is cheaper than new plumbing.

```rust
            "pub fn main() { if 1 < 2 { 10 } else { 20 } }\n",
            "pub fn main() { if 2 < 1 { 10 } else { 20 } }\n",
            "pub fn main() {\n  let x = 5\n  if x > 3 { x * 2 } else { 0 }\n}\n",
            "pub fn main() {\n  let x = 7\n  let a = if x > 0 { if x > 5 { 100 } else { 50 } } else { 0 }\n  let b = if x < 0 { 0 } else { if x > 5 { 7 } else { 3 } }\n  a + b\n}\n",
            "pub fn main() {\n  let a = if 1 < 1 { 1 } else { 0 }\n  let b = if 1 < 2 { 1 } else { 0 }\n  let c = if 1 <= 1 { 1 } else { 0 }\n  let d = if 2 <= 1 { 1 } else { 0 }\n  let e = if 1 > 1 { 1 } else { 0 }\n  let f = if 2 > 1 { 1 } else { 0 }\n  let g = if 1 >= 1 { 1 } else { 0 }\n  let h = if 1 >= 2 { 1 } else { 0 }\n  let i = if 1 == 1 { 1 } else { 0 }\n  let j = if 1 == 2 { 1 } else { 0 }\n  let k = if 1 != 2 { 1 } else { 0 }\n  let m = if 1 != 1 { 1 } else { 0 }\n  let n = if (0 - 1) < 1 { 1 } else { 0 }\n  a + b + c + d + e + f + g + h + i + j + k + m + n\n}\n",
            "pub fn main() { if True == False { 1 } else { 2 } }\n",
            "pub fn main() {\n  let a = if 1 < 2 && 3 > 4 { 1 } else { 0 }\n  let b = if 1 < 2 && 3 < 4 { 1 } else { 0 }\n  let c = if 1 > 2 || 3 > 4 { 1 } else { 0 }\n  let d = if 1 > 2 || 3 < 4 { 1 } else { 0 }\n  a + b + c + d\n}\n",
```

- [x] **Step 4: Run the whole codegen crate and watch it pass**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen -j 2
```

Expected: PASS. Confirm the count went up rather than a test silently vanishing: there should be three new integration tests plus the seven added `corpus_verifies` sources.

- [x] **Step 5: Format, gate, commit**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo fmt --all
$env:CARGO_INCREMENTAL="0"; pwsh scripts/check.ps1
if ($LASTEXITCODE -eq 0) {
  graphify update .
  git add crates/codegen/tests/native_codegen.rs crates/codegen/src/lib.rs
  git commit -m @'
test(codegen): the control-flow corpus runs natively, and matches the evaluator (5b-2 Task 5)

Seven programs compiled, linked, and executed: both branch directions, a
binding live across a diamond, nested ifs on both sides (the phi trap), all
six predicates at their boundary cases plus a signedness probe, Eq on Bool
operands, and strict and/or.

Each one is also checked against what the CEK evaluator says main is worth.
The differential check covers the 5b-1 arithmetic corpus too, so the whole
native surface is under it — "native must match the evaluator" is now an
enforced test rather than a remembered rule.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
'@
  git push origin main
}
```

---

### Task 6: Documentation and close-out

**Files:**
- Modify: [README.md:36-39](../../../README.md#L36-L39)
- Modify: [docs/superpowers/specs/2026-08-29-elya-slice-5b2-native-control-flow-design.md](../specs/2026-08-29-elya-slice-5b2-native-control-flow-design.md) — §12 checklist
- Modify: this plan — tick every checkbox

- [x] **Step 1: Update the README's subset paragraph**

Replace [README.md:36-39](../../../README.md#L36-L39):

```markdown
Output defaults to the input stem plus the platform executable suffix; `-o <out>`
overrides it. As of Slice 5b-2 the backend covers arithmetic and control flow
(`Int` and `Bool`, `let`, `+ - *`, the six comparisons, strict `&&`/`||`, and
`if`/`else`), so anything outside it is rejected by name rather than
mis-compiled — the tree-walking `elya run` remains the full language. Every
compiled program in the test corpus is additionally checked against what the
evaluator computes, so the two never drift apart silently.
```

- [x] **Step 2: Tick the spec's §12 milestone checklist**

Open the spec and mark each of the thirteen boxes. Where the implementation deviated — the five-site Core reach (§2.1) and the evaluator accessor (§6.3) — add a one-line parenthetical beside the box pointing at this plan's "Deliberate deviations" section, so the spec does not read as if it had been followed literally.

- [x] **Step 3: Tick this plan's checkboxes**

Every `- [ ]` in Tasks 1–6 becomes `- [x]`.

- [x] **Step 4: Run the full gate one final time, from clean**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo fmt --all
$env:CARGO_INCREMENTAL="0"; pwsh scripts/check.ps1
```

Expected: all five stages green. Read the output rather than only the exit code — confirm Configuration A ran with no `llvm_sys` in its graph and that no test reported as ignored or filtered out.

- [x] **Step 5: Commit**

```powershell
if ($LASTEXITCODE -eq 0) {
  graphify update .
  git add README.md docs/superpowers/specs/2026-08-29-elya-slice-5b2-native-control-flow-design.md docs/superpowers/plans/2026-08-29-elya-slice-5b2-native-control-flow.md
  git commit -m @'
docs: close out Slice 5b-2 — native control flow (arc node N3)

README states the backend subset as of 5b-2 and notes the differential check.
Spec §12 checklist ticked, with the two deviations from its letter marked
against the plan section that argues them.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
'@
  git push origin main
}
```

---

## Self-Review

**1. Spec coverage.**

| Spec section | Task |
|---|---|
| §1.1 Div/Rem stay out | Task 4 keeps `BinOp::Div`/`Rem` falling through to `op_label`; `rejects_div_specifically` unchanged |
| §1.2 N3 before N2 | Sequencing only; nothing to implement |
| §2.1 Front-end reach | Task 1 — **corrected to five sites**, see deviation #1 |
| §2.2 Not a recorder gap | Task 1 Step 1's `both_origin_checks` call is the executable form of this argument |
| §3.1 Type mapping widens to i1 | Task 3 (`repr_ty`) |
| §3.2 Diamond + two phi obligations | Task 4 Step 7 (`then_exit`/`else_exit` read back from the builder; exactly two incoming) |
| §3.3 Comparisons + Eq/Ne guard | Task 4 Step 6 |
| §3.4 Strict and/or | Task 4 Step 6, plus the module-doc correction in Step 8 |
| §4.1 Fidelity: both evaluators strict | Task 4 Step 6's comment and Task 5's differential check |
| §4.2 Tail-call obligation deferred to N2 | Carried forward; no task |
| §5 Seven-case corpus + differential check | Task 5 — **needs Task 2**, see deviation #2 |
| §6.1 Feature gate | Unchanged; enforced by the gate's Configuration A stages |
| §6.2 The Core change is additive | Task 1; the "find the tests that assert `Unsupported("If")`" step resolved to zero |
| §6.3 Evaluator untouched | **Deviated** — Task 2, additive only, `elya run` unchanged and tested |
| §7 Pipeline/module changes | Tasks 3 and 4 (`lower_expr` signature); no CLI change — `elya build` already routes here |
| §8 Testing strategy | Task 4 (Layer 1, verifier-clean IR), Task 5 (Layer 2, execution) |
| §9 Build order | Tasks 1–6; **six, not five**, because §9's Task 1 splits into the Core change and the evaluator accessor |
| §10 Risks | The phi trap → Task 4 Step 7 and Task 5's `nested_if`; signedness → Task 5's `predicates`; strictness → Task 5's differential check |
| §11 Deferred obligations | No task; Task 6 Step 2 records status |
| §12 Milestone checklist | Task 6 Step 2 |

No spec section is left without a task.

**2. Placeholder scan.** No "TBD", no "similar to Task N", no "add appropriate error handling". Every code step carries the literal text to write. The two steps that are genuinely edits rather than new code (Task 5 Step 3's corpus extension, Task 6 Step 2's checklist ticking) name the exact file, the exact lines, and the exact content.

**3. Type consistency.** Checked across tasks:
- `CoreKind::If(Rc<CoreExpr>, Rc<CoreExpr>, Rc<CoreExpr>)` — the same three-Rc shape in Task 1's enum, Task 1's lowering, Task 1's `pretty_expr`, Task 1's `walk`, Task 4's `if_expr` helper, and Task 4's diamond.
- `lower_expr` grows in two steps and every call site moves with it: Task 3 makes it `(ctx, b, e, env)`, Task 4 makes it `(ctx, func, b, e, env)`, and both tasks list the recursive call sites and the `build_module` call site explicitly.
- `repr_ty` is introduced in Task 3 and used unchanged in Task 4.
- `require_int` keeps its signature and its `"non-Int value"` message across Task 3's move, which is what keeps `rejects_bool_literal_specifically` passing.
- `run_module_value` has one signature — `Result<(Interp, Value), RuntimeError>` — in Task 2's `cek` definition, Task 2's re-export, Task 2's tests, and Task 5's `eval_main_int`.
- `CONTROL_FLOW_CORPUS`'s three-field tuple is destructured as `(tag, src, expected)` and `(tag, src, _)` consistently; `CORPUS` (5b-1) stays two-field and is destructured as `(tag, src)`.

One thing deliberately left inconsistent, and it is not a bug: `eq_operand_label` returns `"Eq on an unrepresentable operand type"` while `repr_ty` returns `"unrepresentable type"`. Two distinct messages for two distinct refusals — the operator's, and the value's.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-08-29-elya-slice-5b2-native-control-flow.md`.

Two execution options:

**1. Subagent-Driven (recommended)** — a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — execute tasks in this session using executing-plans, batch execution with checkpoints for review.

Which approach?
