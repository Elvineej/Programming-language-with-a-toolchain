# Elya Slice 4b-2 — Effect-Carrying Closures & Higher-Order Relay — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give concrete-effect lambdas minimal closed effect rows (the one precision fix), then discharge the dormant effects-spec §11 relay obligation with a fully output-verified test corpus proving effect-carrying closures relay, execute, discharge, and stay row-polymorphic.

**Architecture:** One type-system change — apply the existing `close_unrelayed_residual` to a lambda's latent row so a concrete-effect lambda closes to `{Log}` while a relay lambda stays row-polymorphic. Everything else is coverage: a CEK-only golden corpus (the tree-walker does not evaluate effects), because there is **no differential oracle** behind effect evaluation.

**Tech Stack:** Rust 2021, `logos` lexer, `ariadne` diagnostics, `insta` snapshots. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-14-elya-slice-4b2-effect-closures-design.md`

## Global Constraints

- **Rust 2021, MSRV 1.75. No new dependencies. No new syntax, no new diagnostic code, no new runtime machinery** (4b-2 is precision + coverage).
- **Every shell session:** `export PATH="$HOME/.cargo/bin:$PATH"` and `export CARGO_INCREMENTAL=0`.
- **Before every `sh scripts/check.sh`:** run `cargo fmt --all` (fmt-check fails hard).
- **Commits:** end every message with `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`. Use **explicit `git add` paths** (never `git add -A`). Push to `origin/main` after each task.
- **OUTPUT-VERIFIED HARD RULE (this slice's backstop):** there is no `cek == tree` oracle behind effect-carrying programs. Every test MUST assert a **concrete verified value** — an exact program output string (`run` result) or an exact inferred scheme/row string (`schemes`) or a specific diagnostic code+severity. Never assert merely "it runs", "compiles", or "the fixture exists". A test that does not pin a concrete value is a plan failure.
- **`cek == tree` does NOT extend here.** Effect-carrying programs run **CEK-only** via `run_source`. The effect-free `cek == tree` corpus (4b-1) is untouched.
- **TCE discipline unchanged; measure-then-pin any measured value.**
- Scratch/debug files go in the session scratchpad **outside** the repo.

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `src/types.rs` | HM inference | **One change:** call `close_unrelayed_residual(lam_amb, &param_tys)` in the `Expr::Lambda` arm. |
| `tests/effect_types.rs` | Scheme pins + E0426 (existing `schemes`/`infer_diags` helpers) | Add: `make_logger` closed-row pin, `relay` unchanged pin, E0426 × closures, Fork-C row pin. |
| `tests/effect_closures.rs` (new) | CEK execution corpus for effect-carrying closures | Relay output, E0420-when-unhandled, row-poly earn-out, dynamic scoping, Fork-C runtime. |

No `src` module added; `tests/arch/layering.rs` unchanged.

---

## Task 1: Fork A — close lambda residuals (the precision fix)

**Files:**
- Modify: `src/types.rs` (the `Expr::Lambda` inference arm, ~lines 838-842)
- Test: `tests/effect_types.rs` (via the existing `schemes` helper)

**Interfaces:**
- Consumes: `self.close_unrelayed_residual(amb: RowVar, params: &[Ty])` (existing, [types.rs:1149](../../../src/types.rs#L1149)); `schemes(src) -> (HashMap<String,String>, Vec<String>)` (existing helper in `tests/effect_types.rs`).
- Produces: minimal closed rows for concrete-effect lambdas; relay lambdas unchanged.

- [ ] **Step 1: Write the failing scheme pins.** In `tests/effect_types.rs`, add:

```rust
#[test]
fn concrete_effect_lambda_row_is_closed() {
    // A returned concrete-effect lambda infers a MINIMAL closed row {Log} —
    // not a spurious `forall a. ... {Log | a}` from an unclosed tail (Fork A).
    let src = "effect Log { fn log(msg: String) -> Unit }\n\
               fn make_logger() { fn(n) { log(n) } }\n";
    let (s, d) = schemes(src);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(s["make_logger"], "fn() -> fn(String) / {Log} -> Unit");
}

#[test]
fn relay_lambda_stays_row_polymorphic() {
    // The relay path must be preserved: a function that relays a callback's
    // effects keeps its open, row-polymorphic tail.
    let src = "fn relay(f, x) { f(x) }\n";
    let (s, d) = schemes(src);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(
        s["relay"],
        "forall a b c. fn(fn(a) / {c} -> b, a) / {c} -> b"
    );
}
```

- [ ] **Step 2: Run — expect `concrete_effect_lambda_row_is_closed` to FAIL** (currently `forall a. fn() -> fn(String) / {Log | a} -> Unit`), `relay_lambda_stays_row_polymorphic` to PASS.

Run: `cargo test --test effect_types concrete_effect_lambda_row_is_closed relay_lambda_stays_row_polymorphic -- --nocapture 2>&1 | grep -E "test result|FAILED|left|right"`
Expected: the concrete-effect pin FAILS showing `left: "forall a. ... {Log | a} ..."`; relay PASSES.

- [ ] **Step 3: Apply Fork A.** In `src/types.rs`, in the `Expr::Lambda` arm, add the `close_unrelayed_residual` call between `env.pop();` and `let row = …`:

```rust
                let body_ty = self.infer_block(&body.node, env, lam_amb);
                env.pop();
                // Fork A (4b-2 §2): close the lambda's residual tail unless it is
                // relayed through a parameter — same discipline as top-level fns.
                // A concrete-effect lambda gets a minimal closed row; a relay
                // lambda keeps its open, row-polymorphic tail.
                self.close_unrelayed_residual(lam_amb, &param_tys);
                let row = self.resolve_row(&EffectRow::open(lam_amb));
                Ty::Fn(param_tys, row, Box::new(body_ty))
```

- [ ] **Step 4: Run — expect both pins PASS.**

Run: `cargo test --test effect_types concrete_effect_lambda_row_is_closed relay_lambda_stays_row_polymorphic 2>&1 | grep -E "test result|FAILED"`
Expected: PASS. (If `make_logger`'s printed form differs only in spacing/format, pin the *actual* string — the invariant is: **no `forall`, closed `{Log}`, no `| <var>` tail**. `relay` must be byte-for-byte unchanged.)

- [ ] **Step 5: Regression — full suite.** The fix must not alter any relay row or already-closed row.

Run: `cargo test 2>&1 | grep -E "test result: FAILED|FAILED" | head || echo GREEN`
Expected: all green (145 prior + 2 new).

- [ ] **Step 6: Commit + push.**

```bash
cargo fmt --all && sh scripts/check.sh
git add src/types.rs tests/effect_types.rs
git commit -m "$(printf 'feat(types): close a lambda residual row (concrete-effect lambdas get minimal rows)\n\nApply close_unrelayed_residual to the Expr::Lambda arm (4b-2 Fork A): a\nconcrete-effect lambda now infers a closed {Log} row instead of a spurious\nforall a. {Log | a}; a relay lambda keeps its open, row-polymorphic tail\n(pinned unchanged). make_logger: forall a. fn()->fn(String)/{Log|a}->Unit\nbecomes fn()->fn(String)/{Log}->Unit.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

---

## Task 2: Relay execution, discharge, and the row-poly earn-out

**Files:**
- Create: `tests/effect_closures.rs`
- Test: same

**Interfaces:**
- Consumes: `elya::run_source`, `elya::check_source`, `elya::Session` (the `effects_run.rs` pattern).
- Produces: `run(src) -> String` and `check_err(src) -> String` helpers used by Tasks 2-4.

- [ ] **Step 1: Create the file with helpers + the three failing tests.** Create `tests/effect_closures.rs`:

```rust
//! Slice 4b-2: effect-carrying closures relayed through higher-order functions.
//! CEK-only (the tree-walker does not evaluate effects), so every case is
//! output-verified — a concrete program output or diagnostic — because there is
//! no cek==tree oracle behind effect evaluation.

use elya::{check_source, run_source, Session};

/// Type-check (must be clean), then run on the CEK; return the output.
fn run(src: &str) -> String {
    assert!(
        check_source("t.elya", src).is_ok(),
        "should type-check: {:?}",
        check_source("t.elya", src)
    );
    let _ = Session::new();
    run_source("t.elya", src).expect("should run")
}

/// The rendered compile error for a program that must fail the front end.
fn check_err(src: &str) -> String {
    check_source("t.elya", src).expect_err("should fail to type-check")
}

#[test]
fn effectful_closure_relayed_and_handled_at_call_site() {
    // `apply` relays the closure's Log; the handler at the call site resumes
    // log("hi") with "hi!", which flows back through the closure and `apply`.
    let src = "effect Log { fn log(msg: String) -> String }\n\
               fn apply(f, x) { f(x) }\n\
               pub fn main() {\n\
                 let r = handle {\n\
                   apply(fn(n) { log(n) }, \"hi\")\n\
                 } with {\n\
                   Log.log(m) -> resume(m <> \"!\")\n\
                   return(x) -> x\n\
                 }\n\
                 io.println(r)\n\
               }\n";
    assert_eq!(run(src), "hi!\n");
}

#[test]
fn unhandled_relayed_effect_is_e0420() {
    // No handler anywhere: the relayed Log survives to main and is E0420.
    let src = "effect Log { fn log(msg: String) -> String }\n\
               fn apply(f, x) { f(x) }\n\
               pub fn main() { let _ = apply(fn(n) { log(n) }, \"hi\")\n io.println(\"x\") }\n";
    let err = check_err(src);
    assert!(err.contains("E0420"), "expected E0420 (unhandled effect): {err}");
    assert!(err.contains("Log"), "should name the effect: {err}");
}

#[test]
fn let_bound_hof_lambda_is_row_polymorphic() {
    // The earn-out: a LET-BOUND `apply` (non-top-level) used at a pure row AND
    // at {Log} in one program. Only type-checks if its row var is generalized.
    let src = "effect Log { fn log(msg: String) -> String }\n\
               pub fn main() {\n\
                 let apply = fn(f, x) { f(x) }\n\
                 let a = apply(fn(n) { n + 1 }, 10)\n\
                 let r = handle {\n\
                   apply(fn(n) { log(n) }, \"hi\")\n\
                 } with {\n\
                   Log.log(m) -> resume(m <> \"!\")\n\
                   return(x) -> x\n\
                 }\n\
                 if a == 11 { io.println(r) } else { io.println(\"no\") }\n\
               }\n";
    assert_eq!(run(src), "hi!\n");
}
```

- [ ] **Step 2: Run — expect all three PASS** (verified during brainstorming; this task *locks* the behavior).

Run: `cargo test --test effect_closures 2>&1 | grep -E "test result|FAILED|panicked|left|right" | head`
Expected: PASS (3 tests). If `unhandled_relayed_effect_is_e0420` fails because E0420 surfaces at runtime rather than compile time, switch it to `run_err` (assert check ok, then `run_source(...).expect_err()`) — but the discharge pass runs at inference, so `check_err` is correct.

- [ ] **Step 3: Commit + push.**

```bash
cargo fmt --all && sh scripts/check.sh
git add tests/effect_closures.rs
git commit -m "$(printf 'test(effects): effect-carrying closure relay — execution, discharge, row-poly earn-out\n\nCEK-only, output-verified: apply(fn(n){log(n)}, ...) handled at the call site\nyields hi!; an unhandled relayed effect is E0420; a let-bound HOF lambda used\nat a pure row and {Log} in one program type-checks and runs (row-poly earns out\non a non-top-level function). Discharges the effects-spec §11 relay obligation.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

---

## Task 3: Dynamic call-site scoping + E0426 × closures

**Files:**
- Modify: `tests/effect_closures.rs` (dynamic-scoping test)
- Modify: `tests/effect_types.rs` (E0426 × closures, near `multi_over_io_warns_e0426`)

**Interfaces:**
- Consumes: `run` (Task 2); `infer_diags(src) -> Vec<Diagnostic>` and `Severity` (existing in `tests/effect_types.rs`).

- [ ] **Step 1: Write the dynamic-scoping test.** In `tests/effect_closures.rs`, add:

```rust
#[test]
fn closure_performs_against_call_site_handler_not_definition() {
    // `g` is DEFINED in main with no handler around it, then CALLED inside
    // `call_it`'s handle. Its Log resolves to the call-site handler (dynamic
    // scoping), which appends "-A" — so the output is "inner-A".
    let src = "effect Log { fn log(msg: String) -> String }\n\
               fn call_it(f) {\n\
                 handle { f(\"inner\") } with {\n\
                   Log.log(m) -> resume(m <> \"-A\")\n\
                   return(x) -> x\n\
                 }\n\
               }\n\
               pub fn main() {\n\
                 let g = fn(n) { log(n) }\n\
                 io.println(call_it(g))\n\
               }\n";
    assert_eq!(run(src), "inner-A\n");
}
```

- [ ] **Step 2: Run — expect PASS** (verify the exact output; if the machine returns a different concrete string, that is a real dynamic-scoping finding — investigate, do not just update the expected value).

Run: `cargo test --test effect_closures closure_performs_against_call_site_handler 2>&1 | grep -E "test result|FAILED|left|right"`
Expected: PASS with output `inner-A\n`.

- [ ] **Step 3: Write the E0426 × closures tests.** In `tests/effect_types.rs`, after `multi_over_io_warns_e0426`, add:

```rust
// A closure that performs {Flip} + {IO}, relayed through `run` and handled by a
// `multi` handler: the observable IO is duplicated across resumes -> E0426. The
// same closure under a one-shot handler must NOT warn.
const MULTI_OVER_CLOSURE_IO: &str = "effect Flip { fn flip() -> Bool }\n\
    fn run(f) { f() }\n\
    pub fn main() {\n\
      let _ = handle run(fn() { let x = flip()  let _ = io.println(\"tick\")  x }) with multi { Flip.flip() -> resume(True) }\n\
      io.println(\"done\")\n\
    }\n";

#[test]
fn multi_over_closure_io_warns_e0426() {
    let diags = infer_diags(MULTI_OVER_CLOSURE_IO);
    let w = diags
        .iter()
        .find(|d| d.code == "E0426")
        .expect("expected E0426 for a closure performing IO under multi");
    assert_eq!(w.severity, Severity::Warning, "E0426 must be a warning");
}

#[test]
fn one_shot_over_closure_io_does_not_warn() {
    // Same closure, but a default (one-shot) handler: no duplication, no E0426.
    let src = "effect Flip { fn flip() -> Bool }\n\
        fn run(f) { f() }\n\
        pub fn main() {\n\
          let _ = handle run(fn() { let x = flip()  let _ = io.println(\"tick\")  x }) with { Flip.flip() -> resume(True) }\n\
          io.println(\"done\")\n\
        }\n";
    let diags = infer_diags(src);
    assert!(
        !diags.iter().any(|d| d.code == "E0426"),
        "one-shot handler must not warn E0426: {diags:?}"
    );
}
```

- [ ] **Step 4: Run — expect both PASS.**

Run: `cargo test --test effect_types multi_over_closure_io one_shot_over_closure_io 2>&1 | grep -E "test result|FAILED|panicked"`
Expected: PASS. If `multi_over_closure_io_warns_e0426` fails (no E0426), the multi-shot lint is not seeing a closure-performed observable effect — a real gap; investigate `add_effect`/ambient flow through the closure call before adjusting the test.

- [ ] **Step 5: Commit + push.**

```bash
cargo fmt --all && sh scripts/check.sh
git add tests/effect_closures.rs tests/effect_types.rs
git commit -m "$(printf 'test(effects): dynamic call-site scoping + E0426 through a closure\n\nOutput-verified: a closure defined without a handler and called inside one\nperforms against the call-site handler (inner-A), locking dynamic (not lexical)\neffect scoping. A closure performing observable IO under a multi handler warns\nE0426; under a one-shot handler it does not.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

---

## Task 4: Fork C (relay + own effect) + the Slice-4b-2 exit gate

**Files:**
- Modify: `tests/effect_types.rs` (Fork-C row pin)
- Modify: `tests/effect_closures.rs` (Fork-C runtime)

**Interfaces:**
- Consumes: `schemes` (effect_types), `run` (effect_closures).

- [ ] **Step 1: Write the Fork-C row pin (measure-then-pin).** In `tests/effect_types.rs`, add:

```rust
#[test]
fn relay_plus_own_effect_row() {
    // Fork C (4b-2 §6): a function that BOTH performs its own effect (Log) AND
    // relays a callback. The relayed tail stays open (row-poly); Log is present.
    // Pin the exact inferred row and document it here.
    let src = "effect Log { fn log(msg: String) -> Unit }\n\
               fn both(f, x) { let _ = log(x)  f(x) }\n";
    let (s, d) = schemes(src);
    assert!(d.is_empty(), "{d:?}");
    // EXPECTED (pin the actual observed string on first run; the invariant is:
    // `Log` present AND the relayed callback's row var appears open in `both`'s
    // row — i.e. it is minimal: "Log plus whatever f does", not over-closed).
    assert_eq!(
        s["both"],
        "forall a b. fn(fn(String) / {b} -> a, String) / {Log | b} -> Unit"
    );
}
```

- [ ] **Step 2: Run to observe, then pin.** Run with `--nocapture` and read the actual `both` scheme:

Run: `cargo test --test effect_types relay_plus_own_effect_row -- --nocapture 2>&1 | grep -E "left|right|test result"`
Action: if the assertion fails, replace the expected string with the **actual** observed string, provided it satisfies the invariant (contains `Log`, and the callback's row var `b` appears open in `both`'s row `{Log | b}` — i.e. the row is minimal, not over-closed to just `{Log}` and not dropping `Log`). If the observed row violates the invariant (e.g. `Log` missing, or the relay tail wrongly closed), that is the genuine-wrong case Fork C reserves a fix for — stop and investigate `close_unrelayed_residual` before proceeding. Record the final pinned string in the test comment as the documented behavior.

- [ ] **Step 3: Write the Fork-C runtime (output-verified).** In `tests/effect_closures.rs`, add:

```rust
#[test]
fn relay_plus_own_effect_runs() {
    // `both` logs "own" itself, then relays f which logs "arg"; the handler
    // concatenates every logged message with a separator. Verifies the
    // relay-plus-own-effect closure executes and BOTH performs are handled.
    let src = "effect Log { fn log(msg: String) -> String }\n\
               fn both(f, x) { let _ = log(\"own\")  f(x) }\n\
               pub fn main() {\n\
                 let r = handle {\n\
                   both(fn(n) { log(n) }, \"arg\")\n\
                 } with {\n\
                   Log.log(m) -> resume(m <> \".\")\n\
                   return(x) -> x\n\
                 }\n\
                 io.println(r)\n\
               }\n";
    // `log("own")` -> "own." (discarded); `f("arg")` = log("arg") -> "arg.",
    // returned through both -> handle -> r.
    assert_eq!(run(src), "arg.\n");
}
```

- [ ] **Step 4: Run — expect PASS** (verify the exact output; if different, the relay-plus-own-effect execution differs from expectation — investigate before adjusting).

Run: `cargo test --test effect_closures relay_plus_own_effect_runs 2>&1 | grep -E "test result|FAILED|left|right"`
Expected: PASS with `arg.\n`.

- [ ] **Step 5: The Slice-4b-2 exit gate.**

Run:
```bash
cargo fmt --all
sh scripts/check.sh
```
Expected: exit 0; full suite green. Confirm these rows pass: `concrete_effect_lambda_row_is_closed`, `relay_lambda_stays_row_polymorphic`, `effectful_closure_relayed_and_handled_at_call_site`, `unhandled_relayed_effect_is_e0420`, `let_bound_hof_lambda_is_row_polymorphic`, `closure_performs_against_call_site_handler_not_definition`, `multi_over_closure_io_warns_e0426`, `one_shot_over_closure_io_does_not_warn`, `relay_plus_own_effect_row`, `relay_plus_own_effect_runs`.

- [ ] **Step 6: Commit + push.**

```bash
git add tests/effect_types.rs tests/effect_closures.rs
git commit -m "$(printf 'test(effects): relay-plus-own-effect pinned + documented; Slice-4b-2 exit gate\n\nFork C: `both(f,x){ log(x)  f(x) }` infers a minimal row (Log present, relayed\ntail open) and runs output-verified. Slice 4b-2 complete: concrete-effect\nlambdas get closed rows, the relay obligation is discharged with a CEK-only\noutput-verified corpus (execution, E0420, row-poly earn-out, dynamic scoping,\nE0426 x closures).\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

### Exit criterion

Concrete-effect lambdas infer minimal closed rows (`make_logger` pinned) while relay lambdas stay row-polymorphic (`relay` pinned byte-for-byte); effect-carrying closures relayed through HOFs run on the CEK with verified output and discharge (`E0420` when unhandled); a let-bound HOF lambda is row-polymorphic across two rows; dynamic call-site scoping and E0426 × closures are output-verified; the relay-plus-own-effect row is pinned and documented; full suite green; `cargo fmt --all` + `sh scripts/check.sh` clean.

---

## Self-Review

- **Spec coverage:** §2 Fork A → Task 1 (both scheme pins). §3 relay activation + row-poly earn-out → Task 2 (execution, E0420, let-bound row-poly). §4 dynamic scoping → Task 3. §5 E0426 × closures → Task 3 (multi warns, one-shot does not). §6 Fork C → Task 4 (row pin + runtime). §7 no new diagnostics → nothing to add. §8 pipeline (one `types` change) → Task 1. §9 testing strategy → all tasks, output-verified. §10 build order → the 4-task sequence.
- **Output-verified rule honored:** every test asserts a concrete value — exact scheme strings (`make_logger`, `relay`, `both`), exact outputs (`hi!`, `inner-A`, `arg.`), or a specific code+severity (`E0420`, `E0426` Warning / absence). No "it runs" / "fixture exists" assertions.
- **Placeholder scan:** none. The two measure-then-pin values (`make_logger` post-fix, `both`'s row) carry a concrete expected string **plus** an explicit invariant and a pin-the-actual instruction — the same measure-first discipline used for `K_MAX_CLOSURE`. Not a placeholder.
- **Type/name consistency:** `run`/`check_err` defined in Task 2 and reused in Tasks 3-4; `schemes`/`infer_diags`/`Severity` are existing `effect_types.rs` helpers; `close_unrelayed_residual(RowVar, &[Ty])` matches [types.rs:1149](../../../src/types.rs#L1149); `param_tys`/`lam_amb` match the current `Expr::Lambda` arm.
- **Backstop note:** because no differential oracle covers effect evaluation, Task 2/3/4 outputs were pre-verified against the live compiler during brainstorming (`hi!`, row-poly, relay) or are measure-then-pinned (`make_logger`, `both`) — every green is a pinned concrete value, so a future regression in effect evaluation flips a concrete assertion rather than passing silently.
