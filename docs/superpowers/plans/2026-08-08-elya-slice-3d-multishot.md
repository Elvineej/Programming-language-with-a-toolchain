# Elya Sub-Slice 3d — Multi-Shot `resume` + Cleanup Lint (`E0426`)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cash out the "multi-shot needs no design change" claim from the effect-design review (design spec §8.4). Turn on `with multi`: a handler clause may call `resume` **more than once**, each resumption an independent run of the **same immutable captured continuation** — demonstrated by a genuinely output-verified nondeterminism/collect-all program (spec §4.5). Add the best-effort `E0426` cleanup lint. Deep handlers only; effect-TCE bounds remain 3e.

**Architecture — the whole point is how *little* code this needs.** 3c already made `resume` re-enter the continuation by **cloning** the captured frames (`resume_apply` folds `rd.captured.iter().rev()` with `f.clone()` onto a fresh `Kont`), *never moving* `rd.captured`. The only thing preventing a second resumption is the one-shot `consumed` `Cell` guard. So multi-shot is **one conditional**: skip that guard when `rd.handler.multi`. The re-push path is byte-for-byte unchanged — which is the demonstration, not an assertion, that the persistent `Rc`-`Kont` + `Clone` frames were the right representation. Separately, `E0426` is a **warning** (non-fatal lint), which requires the pipeline to stop treating every diagnostic as fatal — a small, correct, generally-useful change (compilers don't fail on warnings).

**Tech Stack:** Rust 2021; existing deps only. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-06-elya-slice-3-effects.md` (approved), §4.4–4.5 (multi-shot machine), §5 (`E0426`), §8 (golden corpus row (c)), §9 (3d gate: *a working, output-verified multi-shot program*). Grounded in the current `src/eval.rs` (`ResumeData { captured: Vec<Frame>, handler: Rc<Handler>, ret_env, consumed: Cell<bool> }`, `resume_apply`, `Handler.multi: bool`) and `src/types.rs`/`src/diag.rs`/`src/lib.rs` (`Severity::{Error,Warning}` exist; `Diagnostic::error` only; `fail_if_errors` treats any diag as fatal).

---

## Global Constraints

- **Rust edition 2021**, toolchain `stable`, MSRV 1.75. No new dependencies.
- **Module layer map (pinned):** `eval = 6`, `types = 4`. No new module; `tests/arch/layering.rs` still guards it.
- **Diagnostic codes:** **3d adds `E0426`** — a **`Severity::Warning`** "cleanup/effects may run more than once under a multi-shot handler" (design spec §8.6; a lint, not an error — §11). `E0425` (one-shot double-resume, 3c) is **retained** for non-`multi` handlers, unchanged. No other codes touched.
- **Warnings are non-fatal (new, correct behavior):** today `fail_if_errors` fails compilation on *any* diagnostic. 3d makes it fail only on `Severity::Error`, so an `E0426` warning does not block type-checking or running. This changes no existing behavior — every current diagnostic is an `Error` — and is required for a lint to be a lint. CLI surfacing of warnings on a successful compile is **deferred/flagged** (§Deferrals): the warning lives in the diagnostic stream and is asserted by a direct-`infer` test.
- **Ordering discipline (unchanged):** `cargo fmt --all` first, then `sh scripts/check.sh`, commit only on gate exit 0. The `scripts/githooks/pre-commit` fmt guard stays installed. PATH prepends `~/.cargo/bin` ([[cargo-on-windows-path]], [[fmt-before-gate]]); push to `origin/main` after each commit ([[github-remote-workflow]]).
- **Preservation gate (every task):** the entire suite stays green and byte-identical — cross-check (`cek == tree`, effect-free), `examples`, `tce.rs` (`K_MAX = 3`), the 3c effect goldens (one-shot/Exn/nested/tail-resumptive), and **`E0425` still fires for non-`multi` double-resume**.
- **3d exit gate (spec §9):** a working, **output-verified multi-shot program** — `resume` re-invoked and its results combined over `String` — producing the expected output. Not a demo that merely runs.

---

## File Structure

| File | Responsibility | 3d change |
|---|---|---|
| `src/eval.rs` | CEK machine | `resume_apply`: skip the one-shot guard when `rd.handler.multi` (the entire multi-shot change) |
| `src/diag.rs` | diagnostics | add `Diagnostic::warning(code, msg)` (`Severity::Warning`) |
| `src/lib.rs` | pipeline | `fail_if_errors` fails only on `Severity::Error` (warnings non-fatal) |
| `src/types.rs` | inference | emit `E0426` (warning) at a `with multi` handle whose body performs `{IO}` (observable duplication) |
| `tests/effects_run.rs` | golden corpus | add the multi-shot row (c): collect-all over `String`, output-verified |
| `tests/effect_types.rs` | type/warn tests | `E0426` is emitted (severity Warning, rendered wording), and the program still runs (non-fatal) |

No parser/resolver change (`with multi` parsed since 3a; `Handler.multi` already carried).

---

## Task 1: Multi-shot `resume` — honor the `multi` flag (the "no design change" cash-out)

**Files:** Modify `src/eval.rs` (`resume_apply` only). Test: `tests/effects_run.rs`.

**The change is one conditional.** Current `resume_apply` (3c):
```rust
if rd.consumed.get() {
    return Err(/* E0425 */);
}
rd.consumed.set(true);
// ... unchanged: k = k_now; push HandleK; for f in rd.captured.iter().rev() { k = push(f.clone(), k); }
```
becomes:
```rust
// One-shot handlers enforce single use; `with multi` permits re-entry.
if !rd.handler.multi {
    if rd.consumed.get() {
        return Err(/* E0425 */);
    }
    rd.consumed.set(true);
}
// ... the re-push below is IDENTICAL — this is the whole point.
```

**Why re-pushing the same `rd.captured` more than once is sound (demonstrated, spec §4.4–4.5).** Three properties, all already true from 3c:
1. **The capture is an immutable, owned snapshot.** `rd.captured: Vec<Frame>` is materialized once at perform time (the frames above the handler, cloned out of the persistent `Kont`). Nothing ever mutates it — `resume_apply` reads it via `iter().rev()` and clones each frame; it is behind an `Rc<ResumeData>`, shared read-only.
2. **Re-push is clone-not-move.** `for f in rd.captured.iter().rev() { k = push(f.clone(), k) }` **clones** each `Frame` into a *fresh* `Kont`. `Frame: Clone`, and every field is either a persistent `Rc` (AST nodes, `Rc<Handler>`) or a persistent parent-pointer `Env` — cloning shares immutable structure and bumps refcounts; it never aliases mutable state. So two resumptions build two **independent** `Kont`s from the same snapshot.
3. **The `Kont` is persistent.** `Kont = Option<Rc<KontNode>>` is an immutable linked stack; `push` never mutates an existing node. The two resumptions' stacks diverge without touching each other; `Env` is copy-on-write (a new binding creates a new `Scope`, never edits an old one).

Together: resumption *n* and resumption *n+1* run the captured continuation over **disjoint, immutable** machine state — so a value fed in by the first resumption cannot leak into the second. This is exactly the "immutable K-stack, multi-shot at no design change" claim. The `consumed` guard was the *only* thing making it one-shot.

**Worked trace — collect-all over `String` (the §4.5 program).**
```elya
effect Flip { fn flip() -> Bool }
fn choose() { if flip() { "hello" } else { "bye" } }
pub fn main() {
  io.println(handle choose() with multi {
    Flip.flip() -> resume(True) <> "/" <> resume(False)
    return(x)   -> x
  })
}
```
- `choose()` performs `flip()` under `HandleK{multi}`; `k_cap = [IfBranch{then:"hello", else:"bye"}]`, `k_rest` = after the handle.
- Clause body `resume(True) <> "/" <> resume(False)`:
  - `resume(True)`: re-push `k_cap` over a re-installed `HandleK` onto `k_now` (which holds the pending `<> "/" <> resume(False)`). `IfBranch` with `True` → `"hello"` → through `HandleK`/return → flows to the pending `<>`.
  - `... <> "/"` → `"hello/"`, then `resume(False)`: re-push the **same** `k_cap` (cloned again) with `False`. `IfBranch` with `False` → `"bye"` → `"hello/" <> "bye"` → `"hello/bye"`.
- `io.println` → `"hello/bye\n"`. The **same** immutable `IfBranch` frame ran twice, taking a different branch each time because the incoming value differed — no mutation, two independent runs.

- [ ] **Step 1: Failing golden** (`tests/effects_run.rs`): the program above asserts `run(src) == "hello/bye\n"`. (It currently fails at runtime with `E0425` — the second `resume` — proving one-shot was the only blocker.)
- [ ] **Step 2: Implement** the one-conditional relaxation in `resume_apply`.
- [ ] **Step 3: Preservation.** `E0425` must still fire for a **non-`multi`** double-resume (the 3c `resuming_twice_in_a_one_shot_handler_is_e0425` test stays green — the guard still runs when `!multi`).
- [ ] **Step 4: Gate + commit** (`feat(eval): multi-shot resume — with multi permits re-entry of the immutable k_cap (E0425 only for one-shot)`); push.

---

## Task 2: Warnings are non-fatal + `E0426` cleanup lint

**Files:** Modify `src/diag.rs`, `src/lib.rs`, `src/types.rs`. Test: `tests/effect_types.rs`.

**(a) `Diagnostic::warning`** (`src/diag.rs`): mirror `error`, set `Severity::Warning`. `render` already maps it to `ReportKind::Warning`.

**(b) Warnings non-fatal** (`src/lib.rs`): `fail_if_errors` returns `Some(render(diags))` **only if** `diags.iter().any(|d| d.severity == Severity::Error)`, else `None`. (Behavior-identical today — all diags are errors.) Rename to `fail_if_errors` kept; the filter is the change.

**(c) `E0426` — cleanup/effects may run more than once (warning), spec §5 / design §8.6.** Emitted in `types::infer_handle` when the handler is `multi` **and** the handled computation performs `{IO}` — the observable effect that a multi-shot re-run would repeat. Concretely: after inferring the body under `amb_in`, `if handler.multi && self.resolve_row(&EffectRow::open(amb_in)).labels.contains_key("IO") { push E0426 }`. This is a **best-effort, deliberately coarse** lint (it flags the presence of `IO` in the multi-handled computation, not a proof that a specific `io.println` is post-perform); the honest framing is in the help text and §Deferrals. Fixture wording (zonked, named labels, **no `%r`/`%row`** — the effect-diagnostic discipline):
```
warning[E0426]: a multi-shot handler may run its continuation's effects more than once
  ┌─ prog.elya:6:3
  = the handled computation performs {IO}; resuming more than once repeats those effects
  = help: best-effort lint — a full guarantee awaits linear/affine types
```
The label points at the `handle` expression's span; the row is rendered via `resolve_row` + the existing `write_row` (named labels only).

- [ ] **Step 1: Failing tests** (`tests/effect_types.rs`, using a direct `types::infer` call since a warning no longer surfaces through `check_source`):
  - `multi_over_io_warns_e0426`: a `with multi` handle whose body performs `io.println` yields a diagnostic with code `E0426` **and** `severity == Warning`; render it and assert it names `IO` and contains no `%` token.
  - `multi_over_io_still_runs`: the same program **type-checks and runs** (`check_source(..).is_ok()` and `run_source(..)` succeeds) — the lint is non-fatal.
  - `one_shot_over_io_does_not_warn` / `multi_pure_body_does_not_warn`: no `E0426` when the handler isn't `multi`, or when the body performs no `IO`.
- [ ] **Step 2: Implement** `Diagnostic::warning`, the `fail_if_errors` error-filter, and the `E0426` emission in `infer_handle`.
- [ ] **Step 3: Preservation.** Every existing error-producing UI fixture (`E0420`/`E0421`/`E0423`/`E0424`/…) still fails `check_source` exactly as before (they're `Error` severity). No warning is emitted by any pre-3d program.
- [ ] **Step 4: Gate + commit** (`feat(types): E0426 cleanup lint (warning) for multi-shot over {IO}; warnings are non-fatal`); push.

---

## Task 3: Golden corpus gains the multi-shot row (c) + 3d exit gate

**Files:** `tests/effects_run.rs` (corpus header + the multi-shot row), confirm retained nets.

- [ ] **Step 1: Close the deferred corpus row.** 3c's `tests/effects_run.rs` header listed **(c) multi-shot re-invocation** as *deliberately not covered — lands in 3d*. Task 1 added `multi_shot_collects_both_branches` (`"hello/bye\n"`); update the header so (c) is now a covered, output-verified row alongside (a)/(b)/(d)/(e). Add a second multi-shot shape if useful (e.g. three-way collect producing `"a|b|c"`), so (c) isn't a single data point.
- [ ] **Step 2: Confirm the retained nets** are unchanged and green: cross-check (`cek == tree`, effect-free), `tce.rs` (`K_MAX = 3`), and the one-shot `E0425` enforcement. State in the PR that multi-shot is covered by the golden corpus (no host-stack oracle exists — spec §4.6).
- [ ] **Step 3: The 3d exit gate.** `cargo fmt --all && sh scripts/check.sh` fully green: the multi-shot collect-all prints the expected combined string; `E0426` warns (non-fatal) on multi-over-`IO`; `E0425` still errors on one-shot double-resume; all retained nets byte-identical; layering unchanged. Push.
- [ ] **Step 4: Commit + push** (`test(effects): 3d golden corpus — multi-shot collect-all over String (output-verified); exit gate`).

---

## Self-Review

**1. Spec coverage (spec §9 sub-slice 3d → tasks).**
- Multi-shot `resume` re-entry of the immutable `k_cap` (spec §4.4–4.5) → **Task 1**, one conditional, with the soundness argument spelled against the real `Frame`/`Kont`/`Env` and a worked `"hello/bye"` trace — the "no design change" claim *demonstrated*.
- `E0426` cleanup lint, zonked/named-label wording, no-`%r` (spec §5) → **Task 2**.
- Output-verified multi-shot gate (spec §9 3d gate) → **Task 1 golden + Task 3 corpus row (c)** — a program whose *output* is checked (`"hello/bye"`), not a demo that runs.

**2. Every deferral flagged.**
- **Effect-TCE bounds** (tail-resume splice, `K_MAX_EFF`, grow control) → **3e**; 3d does not assert bounded depth (multi-shot re-runs legitimately grow).
- **CLI surfacing of warnings** on a successful compile → deferred: `E0426` lives in the diagnostic stream and is asserted by a direct-`infer` test; wiring warnings to stderr in `main.rs` is future work (flagged in Global Constraints).
- **Sound `E0426` precision** (proving a specific effect is genuinely post-perform, not just present) → deferred with the static-`E0425` obligation (needs CFA / a Core IR, spec §11). 3d ships the coarse best-effort lint, honestly labeled.
- **Full cleanup-safety guarantee** (linear/affine types) → spec §11, unchanged.

**3. Design calls for the reviewer.**
- **(a) `E0426` trigger = `multi` + body performs `{IO}`.** Chosen because `IO` is the one observably-duplicated effect in 3d's surface (user effects are re-handled). Alternative: warn on *any* non-empty residual under `multi`. The `IO`-only rule is less noisy and matches the "observable duplication" intent; flagged in case you want the broader trigger.
- **(b) `E0426` severity = Warning, non-fatal.** Requires the `fail_if_errors` filter. This is the correct lint semantics (and lets multi-over-`IO` programs run), but it introduces the first non-fatal diagnostic and a (deferred) CLI-surfacing gap. Confirm you want warnings non-fatal now rather than making `E0426` a hard error.

**4. The claim, settled.** Task 1's diff is a single `if !rd.handler.multi` guard around code 3c already wrote; the re-push is untouched. That the machine needs no structural change to go from one-shot to multi-shot **is** the payoff of the persistent-`Kont` / `Clone`-frame design chosen back in Slice 2 — this slice proves it by construction, with an output-verified program as the receipt.

---

## Execution Handoff

Plan complete — **paused for review; no code written.** 3d is the promised cash-out of the multi-shot claim, so it goes to you before any implementation. On approval, two execution options:

1. **Subagent-Driven** — dispatch a fresh subagent per task, review between tasks.
2. **Inline Execution** — execute Tasks 1–3 in this session with checkpoints.

Which approach — and any change to the two flagged design calls (3a `E0426` trigger breadth, 3b warnings-non-fatal)?
