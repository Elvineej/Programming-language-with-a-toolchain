# Elya Sub-Slice 3e — TCE Through Effects (tail-resume splice, bounded to a pinned constant)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the design-spec §11.4 TCE cases Slice 2 deferred: **tail-resumptive handlers and tail calls in handler clauses run in bounded continuation depth**, asserted as pinned-constant regression tests (`K_MAX_EFF`) with the same discipline as `tests/tce.rs` (`K_MAX = 3`). Million-deep self- and mutual-tail-resumptive loops stay bounded and produce the **correct output**; a non-tail-resumptive grow control must **increase** with depth, so the bound has teeth. This closes Slice 3.

**Architecture — measurement-first, likely no machine change.** The 3c machine already re-enters a continuation by re-pushing the immutable `k_cap` in `resume_apply`: `Kont' = k_cap ++ [HandleK] ++ k_now`. For a **tail** resume (the clause body *is* `resume(x)`), `k_now` is the clause's own continuation = the `k_rest` captured at perform time — so `Kont'` is structurally identical to the continuation at the perform point, and the subsequent recursive call is a tail call that reuses `k` (the Slice-2 TCE lever). No net growth per operation: the re-push **splices** `k_cap` back into the same slot it came from (spec §6.1). So the expected outcome is that effect-TCE is **already bounded by construction** — 3e's job is to *measure it, pin `K_MAX_EFF`, and prove it* with a two-sided test set. Only if measurement shows growth does explicit tail-position detection get added (§Task 1, conditional path, fully specified).

**Tech Stack:** Rust 2021; existing deps only. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-06-elya-slice-3-effects.md` (approved), §6 (TCE through effects), §9 (exit criterion). Grounded in `src/eval.rs` (`resume_apply` re-push; `apply_callee` reuses `k` on a tail call — no frame; `Interp::peak_kont_depth()`) and `tests/tce.rs` (`peak(src)` helper, `K_MAX = 3`, bounded-vs-grow discipline, "fix the machine, don't raise the constant").

---

## Global Constraints

- **Rust edition 2021**, toolchain `stable`, MSRV 1.75. No new dependencies.
- **Module layer map (pinned):** `eval = 6`. No new module; `tests/arch/layering.rs` still guards it.
- **No new diagnostic codes.** 3e is measurement + (conditional) a machine refinement; no user-facing errors.
- **Pin-from-first-measurement + "fix the machine, don't raise the constant"** (the `tests/tce.rs` doctrine, verbatim): `K_MAX_EFF` is pinned from the first observed peak of the bounded loops (expected a small single/low-double-digit constant). A per-operation splice leak drives the peak toward the iteration count; the assertion `peak ≤ K_MAX_EFF` catches it. **Never raise `K_MAX_EFF` to hide a regression — fix the splice.**
- **Two-sided teeth:** the bounded assertions (`≤ K_MAX_EFF`) and the non-tail **grow control** (`peak(deep) > peak(shallow)`) together lock the machine's tail-vs-non-tail distinction. A change that makes everything grow fails the bounded tests; a change that makes everything bounded (dropping frames that must persist) fails the grow control.
- **Output-verified, not just depth-measured:** every bounded effect-TCE golden asserts the program's **correct output** (`run(1_000_000) == "end"`, mutual `== "done"`) *and* the depth bound — a broken splice that also corrupts results can't pass by being merely shallow.
- **Ordering discipline (unchanged):** `cargo fmt --all` first, then `sh scripts/check.sh`, commit only on gate exit 0. `scripts/githooks/pre-commit` fmt guard stays installed. PATH prepends `~/.cargo/bin` ([[cargo-on-windows-path]], [[fmt-before-gate]]); push to `origin/main` after each commit ([[github-remote-workflow]]). **Debug/render scratch work runs in the scratchpad, never under repo `examples/`** ([[never-rm-rf-examples]]).
- **Preservation gate (every task):** the whole suite stays green — including `tests/tce.rs` (`K_MAX = 3`, pure tail recursion unaffected), the effect goldens, cross-check, and the multi-shot/`E0425`/`E0426` behavior.

---

## The tail-resume splice rule, spelled against the real frames

**Perform** (3c): at `op()`'s perform point the continuation is `k = k_cap ++ [HandleK] ++ k_rest`, where `k_cap` = the frames above the nearest matching `HandleK` (materialized as an owned `Vec<Frame>`), and `k_rest` = that `HandleK`'s `rest`. The clause runs at `k_rest` with `$resume = Value::Resume(rd { captured: k_cap, handler, ret_env, consumed })`.

**Tail resume** (the clause body is exactly `resume(x)`): `resume_apply(rd, u, k_now)` rebuilds
```
Kont' = k_cap ++ [HandleK{rd.handler, rd.ret_env}] ++ k_now   (deepest-first: k_now, then HandleK, then k_cap reversed)
```
Because the `resume(x)` is in **tail position of the clause**, `k_now` — the continuation of that `resume` expression — **is** the clause's continuation, i.e. `k_rest`. So `Kont' = k_cap ++ [HandleK] ++ k_rest`, byte-for-byte the perform-point shape. The resumed computation runs and, when it tail-calls the next iteration, `apply_callee` reuses `k` (no frame). At the next `op()`, the machine splits at the same single `HandleK` and re-splices. **Net frames per operation: zero.** The only transients are the `ResumeApply` frame during `resume`'s argument evaluation and the one `k_cap` slice (a handful of frames) — both reclaimed within the iteration. Hence a constant peak.

**Why the `HandleK` count stays 1:** the perform *consumes* the installed `HandleK` (splits below it); `resume_apply` *re-installs exactly one*. Consume-one / install-one ⇒ the live handler count is invariant across iterations. (A bug that installed without consuming, or vice-versa, would show as linear growth or premature unhandled-effect — caught by the bounded test / the goldens respectively.)

**Non-tail resume** (work sequenced after `resume` in the clause, e.g. `resume(x) <> suffix`): now `k_now = [BinRight{suffix}, k_rest]` — the extra frame persists beneath the re-pushed `k_cap`/`HandleK` until the resumed computation completes, and each further operation adds another. This **legitimately grows** and is out of scope for the bound; it is exactly the grow control (Task 2).

**Conditional implementation (only if measurement shows growth).** If Task 1 measures a peak that grows with the iteration count, the emergent splice is leaking a frame; implement explicit tail detection: when a clause body is syntactically `Expr::Resume { .. }` (tail), have the machine reuse the clause's continuation slot rather than layering (e.g. avoid the transient `ResumeApply` push by evaluating the resume argument first, or mark the clause `HandleK` for in-place replacement). The plan does **not** presuppose this is needed — §6.1 and the trace say it is not — but specifies it so implementation is unblocked either way.

---

## Task 1: Effect-TCE bounded assertions + pin `K_MAX_EFF`

**Files:** Create `tests/tce_effects.rs`. (Conditional: `src/eval.rs`, only if measurement shows growth.)

**Interfaces:** a helper mirroring `tests/tce.rs`'s `peak`, but returning both output and depth (effect programs are type-checked first, then evaluated for the `Interp`):
```rust
fn run_effect(src: &str) -> (String, usize) {
    assert!(elya::check_source("t.elya", src).is_ok(), "must type-check");
    let (m, d) = elya::parse::parse_module(&elya::Session::new(), src);
    assert!(d.is_empty());
    let interp = elya::eval::run_module(&m).unwrap();
    (interp.output().to_string(), interp.peak_kont_depth())
}
const K_MAX_EFF: usize = /* pinned from first measurement — a small constant */;
```

- [ ] **Step 1: Write the bounded goldens (output-verified + depth-bounded).**
  - **Self tail-resumptive** — a `Gen.yield_()` handler resuming in tail position drives a million-deep countdown:
    ```elya
    effect Gen { fn yield_() -> Bool }
    fn run(n) { if n == 0 { "end" } else { if yield_() { run(n - 1) } else { "stop" } } }
    pub fn main() { io.println(handle run(1000000) with { Gen.yield_() -> resume(True) }) }
    // output "end\n", and peak ≤ K_MAX_EFF
    ```
  - **Mutual tail-resumptive** — two mutually-recursive functions, each performing an op resumed in tail position:
    ```elya
    effect Tick { fn tick() -> Bool }
    fn ev(n) { if n == 0 { "done" } else { if tick() { od(n - 1) } else { "stop" } } }
    fn od(n) { if n == 0 { "done" } else { if tick() { ev(n - 1) } else { "stop" } } }
    pub fn main() { io.println(handle ev(1000000) with { Tick.tick() -> resume(True) }) }
    // output "done\n", and peak ≤ K_MAX_EFF
    ```
  - Each test asserts **both** `output == expected` **and** `peak ≤ K_MAX_EFF`.
- [ ] **Step 2: Measure and pin.** Run once; read the observed peak; set `K_MAX_EFF` to that constant (expected small — the tail-resume splice keeps it flat). Add the `tests/tce.rs`-style header comment: *"pinned from first measurement; a per-operation splice leak drives the peak toward the iteration count; do NOT raise `K_MAX_EFF` to hide a regression — fix the machine."*
- [ ] **Step 3: Branch on the measurement.**
  - **If bounded (expected):** the splice is emergent from the persistent `Kont` + tail-call lever; no machine change. Record in the commit that effect-TCE is bounded by construction and `K_MAX_EFF` is pinned.
  - **If growing:** implement the explicit tail-position detection (§"Conditional implementation"), re-measure until flat, then pin. Do not weaken the assertion.
- [ ] **Step 4: Gate + commit** (`test(eval): effect-TCE — tail-resumptive loops bounded at pinned K_MAX_EFF (output-verified)` — or `feat(eval): tail-resume splice …` if the conditional path was taken); push.

---

## Task 2: Non-tail grow control + Slice-3 exit gate

**Files:** `tests/tce_effects.rs` (grow control); confirm the full Slice-3 exit criterion.

- [ ] **Step 1: Non-tail grow control — must increase.** A handler whose clause sequences work **after** `resume` (so the continuation accumulates a frame per operation), driving a K-deep loop, with `peak` growing in K:
  ```elya
  effect Tick { fn tick() -> Unit }
  fn count(n) { if n == 0 { "x" } else { let _ = tick()  count(n - 1) } }
  // handler: Tick.tick() -> resume(Unit) <> "."   // resume NOT in tail position
  ```
  Assert `peak(deep) > peak(shallow)` (e.g. K=50 vs K=5) **and** `peak(deep) ≥ ~K` (proportional), mirroring `tce.rs::non_tail_recursion_grows_with_depth`. This proves the machine distinguishes tail-resume (flat) from non-tail (grows) — the bound has teeth and can't be loosened to mask a splice leak. (Output may also be asserted, but growth is the point.)
- [ ] **Step 2: Slice-3 exit criterion (spec §9).** 3e is the last sub-slice; confirm the whole exit criterion holds:
  - handler-semantics golden corpus — non-resuming, one-shot, **multi-shot (output-verified)**, nested, tail-resumptive — all pass (`tests/effects_run.rs`);
  - row-poly inference + `E042x` fixtures green (`tests/effect_types.rs`, `tests/ui/*`);
  - **tail-resumptive loops bounded at the pinned constant** (Task 1) + the grow control (this task);
  - effect-free cross-check still green (`tests/crosscheck.rs`); `tce.rs` `K_MAX = 3` intact;
  - full gate green; layering unchanged; pushed.
- [ ] **Step 3: Gate + commit** (`test(eval): non-tail grow control + Slice-3 exit gate (effect-TCE two-sided)`); push.

---

## Self-Review

**1. Spec coverage (spec §6, §9 sub-slice 3e → tasks).**
- Tail-resume splice / tail call in handler clause bounded (spec §6.1) → the splice rule spelled against real `k_cap`/`HandleK`/`k_rest`/tail-call-lever; **Task 1** measures + pins (implements only if growth is observed).
- Pinned `K_MAX_EFF` bounded assertions, million-deep self + mutual, output-verified (spec §6.2) → **Task 1**.
- Grow control that must increase (spec §6.2 case 3) → **Task 2**.
- Slice-3 exit criterion (spec §9) → **Task 2 Step 2**, since 3e closes the slice.

**2. Every deferral flagged.**
- **Cross-module TCE case** (design-spec §11.4) → still deferred: needs multi-file modules (not built); 3e's tail-resume/tail-call machinery is single-file (spec §11).
- **A real parameter-passing `State` handler** (threading state through `resume`) → not expressible without lambdas; 3e's tail-resumptive loops thread the counter as the recursion argument with a constant-resuming handler (the expressible form), which exercises the same splice machinery. Flagged so "tail-resumptive State loop" isn't over-claimed.
- **`K_MAX_EFF` exact value** → pinned from first measurement, not guessed in this plan (the `tce.rs` discipline).

**3. Design calls for the reviewer.**
- **(a) Measurement-first, likely no machine change.** The plan's expected outcome is that effect-TCE is already bounded (the persistent-`Kont` re-push splices in tail position), so Task 1 is measure + pin, with explicit tail-detection specified but only implemented on evidence of growth. Confirm you're good with "prove-then-fix-only-if-needed" rather than writing machine code speculatively.
- **(b) Iteration count `1_000_000`.** Matches `tce.rs`. Effect iterations do more per step (perform/capture/re-push, all O(1)); if wall-clock is excessive, `100_000` still distinguishes bounded from linear by orders of magnitude — flagged in case you want the smaller N for CI speed.

**4. Two carry-forwards from 3d, recorded.**
- **`rm -rf examples`** → hardened to a prevention-by-location rule ([[never-rm-rf-examples]]): scratch/debug runs in the scratchpad, never under repo `examples/`; debug renders use a throwaway `#[test] -- --nocapture`, not `cargo run --example`.
- **ariadne `with_help` replaces** → tracked in spec §11 and [[ariadne-with-help-replaces]]: `render` shows only the last help; multi-note diagnostics must first fix `render` to combine helps.

---

## Execution Handoff

Plan complete — **paused for review; no code written.** 3e closes Slice 3, so it goes to you before any implementation. On approval, two execution options:

1. **Subagent-Driven** — dispatch a fresh subagent per task, review between tasks.
2. **Inline Execution** — execute Tasks 1–2 in this session with checkpoints.

Which approach — and any change to the two flagged design calls (measurement-first vs. speculative splice code; `1_000_000` vs. smaller N)?
