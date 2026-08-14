# Elya Slice 4b-3 — Row-Polymorphic Combinators & the Value-Restriction Row Teeth — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lock the closure arc's soundness capstone — a two-sided, output-verified proof that the value restriction keeps a non-value's effect row monomorphic (row-dimension teeth), plus row-polymorphic `map`/`fold` relaying an effectful callback.

**Architecture:** Pure coverage — verified against the live compiler to need no production code. One new test file with output-verified cases: a matched value/non-value pair isolating the effect-row dimension (positive runs `hi!`; negative is `E0423`), and two recursive combinators relaying an effectful callback (`a!b!c!`, `a.b.c.`).

**Tech Stack:** Rust 2021, `logos`, `ariadne`, `insta`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-15-elya-slice-4b3-row-polymorphism-design.md`

## Global Constraints

- **Rust 2021, MSRV 1.75. No new dependencies. No new syntax, no new diagnostic, no new runtime machinery. Expected: no production change** (a capstone coverage slice).
- **Every shell session:** `export PATH="$HOME/.cargo/bin:$PATH"` and `export CARGO_INCREMENTAL=0`.
- **Before every `sh scripts/check.sh`:** run `cargo fmt --all` (fmt-check fails hard).
- **Commits:** end every message with `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`. Use **explicit `git add` paths** (never `git add -A`). Push to `origin/main` after each task.
- **OUTPUT-VERIFIED HARD RULE (enforced, not a note):** there is no `cek == tree` oracle behind effect-carrying programs. Every test MUST assert a **concrete verified value** — an exact program output string or a specific diagnostic code. "It runs" / "an error occurs" / "compiles" is a **plan failure**.
- **NEGATIVE-TEST SPECIFICITY (enforced):** the row negative control must assert the **exact code `E0423`** (an effect-row mismatch) — *and* assert `E0400` is **absent** (proving the types unified cleanly, so the sole conflict is the row, at the second use site). Asserting `is_err()` is a plan failure; the proof rests on it being a *row* conflict.
- **`cek == tree` does NOT extend here.** Effectful programs run CEK-only via `run_source`; the effect-free `cek == tree` corpus is untouched.
- Scratch/debug files go in the session scratchpad **outside** the repo.

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `tests/row_polymorphism.rs` (new) | The closure-arc row capstone: value-restriction row teeth + row-poly combinators | Create with `run`/`check_err` helpers and four output-verified tests. |

No `src` change expected; `tests/arch/layering.rs` unchanged.

---

## Task 1: The value-restriction row teeth (two-sided)

**Files:**
- Create: `tests/row_polymorphism.rs`

**Interfaces:**
- Consumes: `elya::check_source`, `elya::run_source`, `elya::Session` (the `effect_closures.rs` pattern).
- Produces: `run(src) -> String` and `check_err(src) -> String` helpers reused by Task 2.

- [ ] **Step 1: Create the file with helpers + the matched pair.** Create `tests/row_polymorphism.rs`:

```rust
//! Slice 4b-3: the closure-arc row capstone. Proves the value restriction keeps a
//! non-value's EFFECT ROW monomorphic (two-sided teeth), and that row polymorphism
//! composes through recursive combinators. CEK-only + output-verified: every case
//! pins a concrete output or a specific diagnostic code, because there is no
//! cek==tree oracle behind effect evaluation.

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
fn value_bound_relay_is_row_polymorphic() {
    // POSITIVE teeth: `g` is a lambda (a VALUE), so its effect-row variable is
    // generalized -> row-polymorphic -> usable at a pure row AND at {Log} in one
    // program. Both thunks return String (so only the ROW varies across uses).
    let src = "effect Log { fn log(msg: String) -> String }\n\
               pub fn main() {\n\
                 let g = fn(thunk) { thunk() }\n\
                 let a = g(fn() { \"pure\" })\n\
                 let r = handle {\n\
                   g(fn() { log(\"hi\") })\n\
                 } with {\n\
                   Log.log(m) -> resume(m <> \"!\")\n\
                   return(x) -> x\n\
                 }\n\
                 if a == \"pure\" { io.println(r) } else { io.println(\"no\") }\n\
               }\n";
    assert_eq!(run(src), "hi!\n");
}

#[test]
fn nonvalue_bound_relay_row_stays_monomorphic() {
    // NEGATIVE teeth (the guard): the SAME program, but `g`'s RHS is a CALL
    // (`make_relay()`) — a non-value. The value restriction must NOT generalize
    // its row, so `g` is monomorphic: the first use fixes the row to pure, the
    // second use needs {Log} and conflicts.
    //
    // ROW-DIMENSION ISOLATION: both thunks return String, so `g`'s result type
    // unifies cleanly (no E0400). The sole conflict is the effect row at the
    // SECOND use, so the diagnostic must be E0423 (effect-row mismatch) — that is
    // the proof the gate holds the ROW dimension specifically.
    let src = "effect Log { fn log(msg: String) -> String }\n\
               fn make_relay() { fn(thunk) { thunk() } }\n\
               pub fn main() {\n\
                 let g = make_relay()\n\
                 let a = g(fn() { \"pure\" })\n\
                 let r = handle {\n\
                   g(fn() { log(\"hi\") })\n\
                 } with {\n\
                   Log.log(m) -> resume(m <> \"!\")\n\
                   return(x) -> x\n\
                 }\n\
                 if a == \"pure\" { io.println(r) } else { io.println(\"no\") }\n\
               }\n";
    let err = check_err(src);
    assert!(
        err.contains("E0423"),
        "must be a ROW mismatch (the row is not generalized): {err}"
    );
    assert!(
        !err.contains("E0400"),
        "types must unify cleanly — the sole conflict is the effect row: {err}"
    );
}
```

- [ ] **Step 2: Run — expect both PASS** (verified during brainstorming: positive runs `hi!`, negative is `E0423`).

Run: `cargo test --test row_polymorphism value_bound_relay nonvalue_bound_relay 2>&1 | grep -vE "hard linking|Compiling|Finished|Running" | grep -E "test result|FAILED|left|right|panicked" | head`
Expected: PASS (2 tests). If `nonvalue_bound_relay_row_stays_monomorphic` fails because the error is **not** `E0423` (e.g. it type-checks — the row generalized — or it is `E0400`), STOP: either the value restriction leaks on the row dimension (a real, high-value gap in the gate) or the construction failed to isolate the row. Investigate before proceeding; do not weaken the assertion to `is_err()`.

- [ ] **Step 3: Regression — full suite.**

Run: `cargo test 2>&1 | grep -E "test result: FAILED|FAILED" | head || echo GREEN`
Expected: all green (156 prior + 2 new).

- [ ] **Step 4: Commit + push.**

```bash
cargo fmt --all && sh scripts/check.sh
git add tests/row_polymorphism.rs
git commit -m "$(printf 'test(types): value-restriction row teeth — two-sided, row-isolated\n\nOutput-verified matched pair: a value-bound relay is row-polymorphic (used at a\npure row and {Log}, runs hi!); the structurally identical non-value-bound relay\n(let g = make_relay()) keeps its row monomorphic and fails E0423 at the second\nuse. Both thunks return String so the type unifies — E0423-not-E0400 proves the\ngate holds the ROW dimension. The row-dimension analogue of 4b-1s type-dimension\ncontrol; a forward-looking regression-guard (spec §0).\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

---

## Task 2: Row-polymorphic combinators + the Slice-4b-3 exit gate

**Files:**
- Modify: `tests/row_polymorphism.rs`

**Interfaces:**
- Consumes: `run` (Task 1).

- [ ] **Step 1: Add the combinator tests.** Append to `tests/row_polymorphism.rs`:

```rust
#[test]
fn map_relays_effectful_callback() {
    // Row-poly `map` relays the callback's Log through recursion; each element is
    // logged and resumed with "!", producing ["a!","b!","c!"] -> "a!b!c!".
    let src = "effect Log { fn log(msg: String) -> String }\n\
               type List(a) { Nil, Cons(a, List(a)) }\n\
               fn map(xs, f) { match xs { Nil -> Nil  Cons(h, t) -> Cons(f(h), map(t, f)) } }\n\
               fn concat_all(xs) { match xs { Nil -> \"\"  Cons(h, t) -> h <> concat_all(t) } }\n\
               pub fn main() {\n\
                 let xs = Cons(\"a\", Cons(\"b\", Cons(\"c\", Nil)))\n\
                 let ys = handle {\n\
                   map(xs, fn(s) { log(s) })\n\
                 } with {\n\
                   Log.log(m) -> resume(m <> \"!\")\n\
                   return(x) -> x\n\
                 }\n\
                 io.println(concat_all(ys))\n\
               }\n";
    assert_eq!(run(src), "a!b!c!\n");
}

#[test]
fn fold_relays_effectful_callback() {
    // Row-poly `fold` relays the callback's Log while accumulating; each element
    // is logged and resumed with ".", producing "a.b.c.".
    let src = "effect Log { fn log(msg: String) -> String }\n\
               type List(a) { Nil, Cons(a, List(a)) }\n\
               fn fold(xs, acc, f) { match xs { Nil -> acc  Cons(h, t) -> fold(t, f(acc, h), f) } }\n\
               pub fn main() {\n\
                 let xs = Cons(\"a\", Cons(\"b\", Cons(\"c\", Nil)))\n\
                 let total = handle {\n\
                   fold(xs, \"\", fn(acc, s) { acc <> log(s) })\n\
                 } with {\n\
                   Log.log(m) -> resume(m <> \".\")\n\
                   return(x) -> x\n\
                 }\n\
                 io.println(total)\n\
               }\n";
    assert_eq!(run(src), "a.b.c.\n");
}
```

- [ ] **Step 2: Run — expect both PASS** (verified during brainstorming).

Run: `cargo test --test row_polymorphism map_relays fold_relays 2>&1 | grep -vE "hard linking|Compiling|Finished|Running" | grep -E "test result|FAILED|left|right" | head`
Expected: PASS with outputs `a!b!c!` and `a.b.c.`.

- [ ] **Step 3: The Slice-4b-3 exit gate.**

```bash
cargo fmt --all
sh scripts/check.sh
```
Expected: exit 0; full suite green. Confirm these rows pass: `value_bound_relay_is_row_polymorphic`, `nonvalue_bound_relay_row_stays_monomorphic`, `map_relays_effectful_callback`, `fold_relays_effectful_callback`.

- [ ] **Step 4: Commit + push.**

```bash
git add tests/row_polymorphism.rs
git commit -m "$(printf 'test(effects): row-poly map/fold relaying an effectful callback; Slice-4b-3 exit gate\n\nOutput-verified: map relays a logging callback through recursion -> a!b!c!; fold\naccumulates while relaying -> a.b.c.. With the value-restriction row teeth\n(Task 1) this closes Slice 4b — the closure arc: basic closures (4b-1),\neffect-carrying closures & relay (4b-2), the row capstone (4b-3).\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

### Exit criterion

The value restriction's row teeth are pinned two-sided (value row-poly runs `hi!`; matched non-value is `E0423` with `E0400` absent — row-isolated); `map` and `fold` relay an effectful callback with verified output (`a!b!c!`, `a.b.c.`); the full suite is green; `cargo fmt --all` + `sh scripts/check.sh` clean. **Slice 4b (the closure arc) is closed.**

---

## Self-Review

- **Spec coverage:** §2 value-restriction row teeth → Task 1 (positive `hi!` + negative `E0423`/no-`E0400`). §3 row-poly combinators → Task 2 (`map` `a!b!c!`, `fold` `a.b.c.`). §4 no new diagnostics → nothing added. §5 no production change expected → tests-only; the Task 1 Step 2 stop-condition covers the "gap found" contingency. §6 testing strategy → both tasks, output-verified. §7 build order → the 2-task sequence.
- **Output-verified rule honored:** every test asserts a concrete value — exact outputs (`hi!`, `a!b!c!`, `a.b.c.`) or the specific code `E0423` (with `E0400` asserted absent). No `is_err()` / "it runs" assertions.
- **Negative-test specificity honored:** `nonvalue_bound_relay_row_stays_monomorphic` asserts `E0423` present **and** `E0400` absent, with a comment tying the diagnostic to the second use site and the row-dimension isolation. A stop-condition (Step 2) forbids weakening to `is_err()`.
- **Placeholder scan:** none — every step is concrete verified code.
- **Type/name consistency:** `run`/`check_err` defined in Task 1, reused in Task 2; all four test names match between the tasks and the exit criterion.
- **Backstop note:** oracle-less like 4b-2 — all four outputs were pre-verified against the live compiler during brainstorming (`hi!`, `E0423`, `a!b!c!`, `a.b.c.`), so every green is a pinned concrete value and a future regression flips a concrete assertion rather than passing silently.
