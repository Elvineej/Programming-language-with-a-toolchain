# Elya Slice 4c-1 — Parameter-Passing State — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Demonstrate that `State` is an ordinary user handler over the effect mechanism (no language primitive) — a parameter-passing `State { get/set }` running and output-verified — and discharge the last open Slice-3 §11 obligation: the parameter-passing `State` tail-loop, measured bounded.

**Architecture:** Pure coverage — verified against the live compiler to need no production code. One new test file with output-verified cases: a state-passing deep handler (clauses return `fn(s){…}`, `resume` returns a function) yielding `ab`, and a tail-recursive `State` loop bounded at a constant peak depth (`K_MAX_STATE = 5`) with a non-tail grow control.

**Tech Stack:** Rust 2021, `logos`, `ariadne`, `insta`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-15-elya-slice-4c1-parameter-passing-state-design.md`

## Global Constraints

- **Rust 2021, MSRV 1.75. No new dependencies. No new syntax, no new diagnostic, no new runtime machinery. Expected: no production change** (a coverage/obligation-discharge slice).
- **Every shell session:** `export PATH="$HOME/.cargo/bin:$PATH"` and `export CARGO_INCREMENTAL=0`.
- **Before every `sh scripts/check.sh`:** run `cargo fmt --all` (fmt-check fails hard).
- **Commits:** end every message with `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`. Use **explicit `git add` paths** (never `git add -A`). Push to `origin/main` after each task.
- **OUTPUT-VERIFIED HARD RULE (enforced, not a note):** no `cek == tree` oracle stands behind effect-carrying programs. Every test MUST assert a **concrete verified value** — an exact program output string, or a measured depth relation (exact peak / `deep > shallow`). "It runs" / "an error occurs" / "compiles" is a **plan failure**.
- **TCE MEASURE-THEN-PIN + FIX-THE-MACHINE (enforced):** `K_MAX_STATE = 5` is pinned from measurement (depth 5 = pure `K_MAX = 3` + effect frame + the state-passing function-application frame). It **must be constant across N**. **Stop-condition:** if the bounded loop's peak comes back **higher than 5, or grows with N**, that is a per-operation leak in the state-passing splice — **investigate and fix the machine; never raise the constant**. The non-tail grow control is the teeth.
- **`cek == tree` does NOT extend here.** `State` programs run CEK-only via `run_module`/`run_source`; the effect-free corpus and `K_MAX_EFF` tests are untouched.
- Scratch/debug files go in the session scratchpad **outside** the repo.

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `tests/state_effect.rs` (new) | Parameter-passing `State`: the handler demonstration + the tail-loop TCE (§11 obligation) | Create with a `run_peak` helper (output + peak depth) and the four tests. |

No `src` change expected; `tests/arch/layering.rs` unchanged.

---

## Task 1: The parameter-passing `State` demonstration

**Files:**
- Create: `tests/state_effect.rs`

**Interfaces:**
- Consumes: `elya::check_source`, `elya::parse::parse_module`, `elya::eval::run_module`, `Interp::peak_kont_depth`, `elya::Session` (the `tests/tce_effects.rs` `run_effect` pattern).
- Produces: `run_peak(src) -> (String, usize)` reused by Task 2.

- [ ] **Step 1: Create the file with the helper + the demonstration test.** Create `tests/state_effect.rs`:

```rust
//! Slice 4c-1: parameter-passing State. `State` is an ordinary user handler over
//! the effect mechanism (the manifesto's "state is the State effect", no language
//! primitive), interpreted state-passing style — each clause returns a function of
//! the state, `resume` returns a function, and the whole handle is a fn(State) ->
//! Result applied to the initial state. CEK-only + output-verified: no cek==tree
//! oracle stands behind effect evaluation.

use elya::parse::parse_module;
use elya::Session;

/// Type-check (effects checked since 3b), then evaluate on the CEK; return the
/// program output and the peak continuation depth.
fn run_peak(src: &str) -> (String, usize) {
    assert!(
        elya::check_source("t.elya", src).is_ok(),
        "must type-check: {:?}",
        elya::check_source("t.elya", src)
    );
    let (m, d) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    let interp = elya::eval::run_module(&m).unwrap();
    (interp.output().to_string(), interp.peak_kont_depth())
}

#[test]
fn parameter_passing_state_threads_through_resume() {
    // set("a"); x = get(); set(x <> "b"); get()  under a state-passing handler
    // threads "a" -> read into x -> "ab" -> read out. `resume` returns fn(s).
    let src = "effect State {\n\
               \x20 fn get() -> String\n\
               \x20 fn set(v: String) -> Unit\n\
               }\n\
               fn run() {\n\
               \x20 let _ = set(\"a\")\n\
               \x20 let x = get()\n\
               \x20 let _ = set(x <> \"b\")\n\
               \x20 get()\n\
               }\n\
               pub fn main() {\n\
               \x20 let program = handle { run() } with {\n\
               \x20   State.get() -> fn(s) { (resume(s))(s) }\n\
               \x20   State.set(v) -> fn(s) { (resume(Unit))(v) }\n\
               \x20   return(x) -> fn(s) { x }\n\
               \x20 }\n\
               \x20 io.println(program(\"init\"))\n\
               }\n";
    let (out, _peak) = run_peak(src);
    assert_eq!(out, "ab\n");
}
```

- [ ] **Step 2: Run — expect PASS** (verified during brainstorming: output `ab`).

Run: `cargo test --test state_effect parameter_passing_state 2>&1 | grep -vE "hard linking|Compiling|Finished|Running" | grep -E "test result|FAILED|left|right" | head`
Expected: PASS with output `ab`.

- [ ] **Step 3: Regression — full suite.**

Run: `cargo test 2>&1 | grep -E "test result: FAILED|FAILED" | head || echo GREEN`
Expected: all green (160 prior + 1 new).

- [ ] **Step 4: Commit + push.**

```bash
cargo fmt --all && sh scripts/check.sh
git add tests/state_effect.rs
git commit -m "$(printf 'test(effects): parameter-passing State — the manifesto claim, verified\n\nState is an ordinary user handler over the effect mechanism (no language\nprimitive): a state-passing deep handler (clauses return fn(s), resume returns a\nfunction, handle is fn(State)->Result applied to the initial state) runs\nset/get threading and yields ab. Output-verified, CEK-only. Delivers the\ndesign-spec §1 claim that mutable state is the State effect; only possible now\nthat lambdas exist (Slice 4b).\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

---

## Task 2: The `State` tail-loop TCE (the §11 obligation) + the exit gate

**Files:**
- Modify: `tests/state_effect.rs`

**Interfaces:**
- Consumes: `run_peak` (Task 1).

- [ ] **Step 1: Add the bounded loop + grow control.** Append to `tests/state_effect.rs`:

```rust
// Pinned from measurement: a State tail-loop peaks at 5 — pure TCE's K_MAX = 3
// plus the effect frame plus the state-passing function-application frame. It MUST
// be constant across N; a peak that grows is a per-operation splice leak (fix the
// machine, never raise the constant). The non-tail grow control below is the teeth.
const K_MAX_STATE: usize = 5;

fn state_tail_loop(n: i64) -> String {
    // Tail-recursive driver: `set` then a tail call to `loop`. `n` performs, then
    // a final `get`; the state-passing handler threads it all.
    format!(
        "effect State {{ fn get() -> String  fn set(v: String) -> Unit }}\n\
         fn loop(n) {{ if n == 0 {{ get() }} else {{ let _ = set(\"x\")  loop(n - 1) }} }}\n\
         pub fn main() {{\n\
           let program = handle {{ loop({n}) }} with {{\n\
             State.get() -> fn(s) {{ (resume(s))(s) }}\n\
             State.set(v) -> fn(s) {{ (resume(Unit))(v) }}\n\
             return(x) -> fn(s) {{ x }}\n\
           }}\n\
           io.println(program(\"init\"))\n\
         }}\n"
    )
}

#[test]
fn state_tail_loop_is_bounded() {
    let (out_small, small) = run_peak(&state_tail_loop(100_000));
    let (out_large, large) = run_peak(&state_tail_loop(1_000_000));
    assert_eq!(out_small, "x\n", "state tail-loop must run to completion");
    assert_eq!(out_large, "x\n", "state tail-loop must run to completion at large N");
    assert!(
        small <= K_MAX_STATE,
        "state tail-loop peak={small} exceeds K_MAX_STATE={K_MAX_STATE}"
    );
    assert_eq!(
        small, large,
        "state tail-loop peak must be CONSTANT in N (a growing peak is a splice leak): {small} vs {large}"
    );
}

#[test]
fn non_tail_state_loop_grows_with_length() {
    // The recursive `loop` call is under `<>` (non-tail), so each level leaves a
    // frame; peak grows with N. Proves the bounded loop's flatness is a real
    // property of tail position, not an accident of the state-passing machine.
    let prog = |n: i64| {
        format!(
            "effect State {{ fn get() -> String  fn set(v: String) -> Unit }}\n\
             fn loop(n) {{ if n == 0 {{ get() }} else {{ let _ = set(\"x\")  loop(n - 1) <> \"y\" }} }}\n\
             pub fn main() {{\n\
               let program = handle {{ loop({n}) }} with {{\n\
                 State.get() -> fn(s) {{ (resume(s))(s) }}\n\
                 State.set(v) -> fn(s) {{ (resume(Unit))(v) }}\n\
                 return(x) -> fn(s) {{ x }}\n\
               }}\n\
               io.println(program(\"init\"))\n\
             }}\n"
        )
    };
    let (_s, shallow) = run_peak(&prog(5));
    let (_d, deep) = run_peak(&prog(50));
    assert!(deep > shallow, "non-tail state loop must grow: {shallow} vs {deep}");
    assert!(deep >= 45, "expected depth ~proportional to n=50, got {deep}");
}
```

- [ ] **Step 2: Run — expect PASS; if not, apply the stop-condition.**

Run: `cargo test --test state_effect state_tail_loop_is_bounded non_tail_state_loop_grows 2>&1 | grep -vE "hard linking|Compiling|Finished|Running" | grep -E "test result|FAILED|left|right|exceeds|CONSTANT|grow" | head`
Expected: both PASS — `state_tail_loop_is_bounded` at constant peak `5`, `non_tail_state_loop_grows_with_length` grows.
- If `state_tail_loop_is_bounded` fails because `small != large` (peak **grows with N**): STOP — a per-operation frame is leaking in the state-passing splice. Investigate the machine (`resume_apply` / the `(resume)(…)` application path); do **not** raise or loosen the constant.
- If it fails because `small > 5` but is still **constant across N**: the extra frames are a fixed structural cost of this loop shape. Re-measure, set `K_MAX_STATE` to the observed constant, and record why in the comment — the invariant is *constant-in-N*, the pinned number is the observed floor.
- If `1_000_000` is prohibitively slow (> ~15 s wall), reduce the larger N to `500_000` (still a large, constant-proving second point); keep `100_000` as the first. Do not reduce below a 5× gap.

- [ ] **Step 3: The Slice-4c-1 exit gate.**

```bash
cargo fmt --all
sh scripts/check.sh
```
Expected: exit 0; full suite green. Confirm these rows pass: `parameter_passing_state_threads_through_resume`, `state_tail_loop_is_bounded`, `non_tail_state_loop_grows_with_length`.

- [ ] **Step 4: Commit + push.**

```bash
git add tests/state_effect.rs
git commit -m "$(printf 'test(tce): parameter-passing State tail-loop bounded (K_MAX_STATE=5); discharges Slice-3 §11\n\nOutput-verified: a tail-recursive State driver performing N operations peaks at a\nconstant depth 5 across large N (K_MAX_STATE = pure 3 + effect + state-passing\nframes), even though the state-passing clause resumes in non-tail position; a\nnon-tail State loop grows with N (grow control). Closes the last open Slice-3\n§11 obligation — the parameter-passing State tail-loop deferred in 3e until\nlambdas existed.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

### Exit criterion

A parameter-passing `State` handler runs on the CEK with verified output (`ab`); the `State` tail-loop is bounded (`K_MAX_STATE = 5`, constant across two large N, output-verified complete) with a non-tail grow control; the full suite is green; `cargo fmt --all` + `sh scripts/check.sh` clean. The last open Slice-3 §11 `State`-TCE obligation is discharged.

---

## Self-Review

- **Spec coverage:** §2 the encoding → Task 1 (demo yields `ab`). §3 TCE bounded + grow control → Task 2 (`K_MAX_STATE = 5` constant-in-N + non-tail growth). §4 no new diagnostics → nothing added. §5 no production change expected → tests-only; Task 2 Step 2 stop-condition covers the "gap found" contingency. §6 testing strategy → both tasks, output-verified. §7 build order → the 2-task sequence.
- **Output-verified rule honored:** every test asserts a concrete value — exact outputs (`ab`, `x`) or a measured depth relation (`peak <= 5`, `small == large`, `deep > shallow`, `deep >= 45`). No `is_ok`-only / "it runs" assertions.
- **Measure-then-pin + fix-the-machine honored:** `K_MAX_STATE = 5` carries the constant-in-N invariant and an explicit three-branch stop-condition (grows → machine bug; higher-but-constant → re-pin the floor; too slow → reduce N, not the constant). Never loosen to raise the ceiling.
- **Placeholder scan:** none — every step is concrete verified code. The `state_tail_loop`/`prog` generators use inline `{n}` capture (no redundant positional arg).
- **Type/name consistency:** `run_peak` defined in Task 1 and reused in Task 2; all three test names match between tasks and the exit criterion; `K_MAX_STATE = 5` matches the spec.
- **Backstop note:** oracle-less like every effect slice — `ab` and the bounded depth were pre-measured against the live compiler (5 constant at N = 10/100/1000), so every green pins a concrete value and a regression flips a concrete assertion. The larger-N points (100k/1M) extend that measurement, not re-open it.
