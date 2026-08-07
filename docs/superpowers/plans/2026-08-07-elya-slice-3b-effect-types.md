# Elya Sub-Slice 3b — Effect-Row Types, Inference & Discharge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make effects *mean something to the type system*. Add a **second union-find over effect rows**, **row-polymorphic effect inference** (ambient-row threading, Algorithm-J-conservative), typing for `handle`/`resume` with **type-level discharge**, and the effect-row diagnostics `E0420`/`E0421`/`E0423`/`E0424` — all under the same zonk-before-print / no-`%`-token discipline Slice 2 established. **No machine changes:** `check_source` type-checks effect programs; `run_source` still rejects them at eval with the 3c placeholder. This is the type-system heart of Slice 3 (spec §3, §5, §9 sub-slice 3b).

**Architecture:** A layered enrichment of `types.rs` only (plus a small, additive AST/parser change to record effect *annotations* precisely, and UI fixtures). `Ty::Fn` grows an `EffectRow`; a `row_subst` union-find sits alongside the type `subst`; inference threads an **ambient row** that every performed effect is unified into; `handle` subtracts the handled effect; a **separate discharge pass** (not the unifier) reports `E0420`/`E0421`/`E0423`. Every stage keeps the effect-free program behavior byte-identical — pure functions print `fn(a) -> b` exactly as today.

**Tech Stack:** Rust 2021; existing deps only (`logos`, `ariadne`, `insta`). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-06-elya-slice-3-effects.md` (approved). "spec §X" refers there. Grounded in the current `src/types.rs` (Algorithm J: `subst: Vec<Option<Ty>>`, `resolve`/`occurs`/`bind`/`unify`, `Scheme{vars,ty}`, `infer_expr(&e,&mut env)->Ty`, SCC-ordered `infer_schemes`, zonk-before-print `display_ty`/`display_scheme`).

---

## Global Constraints

- **Rust edition 2021**, toolchain `stable`, MSRV 1.75. No new dependencies.
- **Pass-signature rule** (unchanged): every pass is `fn(&Session, In) -> Vec<Diagnostic>` (or `-> (Out, Vec<Diagnostic>)`); no globals/`thread_local`. The row union-find lives inside `Infer`, exactly as the type `subst` does.
- **Module layer map (pinned, unchanged):** `span/diag=0, lex=1, ast=1, parse=2, resolve=3, types=4, core=5, eval=6, main=7`. No new module; `tests/arch/layering.rs` still guards it.
- **Diagnostic codes:** `E042x` reserved for effects (design spec §9). **3b delivers exactly:** `E0420` (effect never handled), `E0421` (must be pure here), `E0423` (effect-row mismatch), `E0424` (cyclic effect row). **`E0422` is intentionally unused** — the spec §5 sequence skips it; do **not** allocate it here. `E0425`/`E0426` are 3c/3d. The temporary `E0499` placeholder introduced in 3a is **removed** for `Handle`/`Resume` in Task 5 (that is the feature landing, not an accommodation — §"Deliberate test changes").
- **Diagnostic discipline (spec §5, non-negotiable):** *discharge is a separate pass from unification*; **zonk rows before printing**; an `EffectRow` renders as **named labels** (sorted) plus, only when a genuinely-polymorphic tail is relevant, a readable `| e` residual — **never** a raw `%r`/`%e`/`%row` token. `unify_row` is a **diagnostic-free primitive** that *returns* a structured `RowConflict`; the **caller** (inference site or discharge pass) chooses `E0420`/`E0421`/`E0423` and attaches provenance. (The row occurs-check `E0424` is the one exception — pushed inside the primitive, mirroring how `E0401` is pushed inside `bind`.)
- **Ordering discipline (fixes the 3a slip):** at each task, run `cargo fmt --all` (write mode) **first**, then `sh scripts/check.sh`, and **only commit if the gate exits 0** — never in the same command that runs the gate without checking its result. **Task 0 installs a `pre-commit` hook** that runs `cargo fmt --all -- --check`, so an unformatted commit is *structurally blocked* even without the discipline note. Toolchain PATH: prepend `~/.cargo/bin` (project memory: [[cargo-on-windows-path]], [[fmt-before-gate]]). Push to `origin/main` after each commit (standing rule, [[github-remote-workflow]]).
- **Effect-free preservation gate (every task):** the entire pre-3b suite that exercises *pure* programs stays **green and byte-identical** — including printed types (`fn(a) -> b`, `forall a. fn(a) -> a`). Rows are invisible in pure output. The **only** permitted edits to existing tests are the two shape-forced ones enumerated in Tasks 1 and 5 (annotation-shape; E0499→typed), each preserving semantic content.
- **`Ty::Fn` arity of the change:** `Ty::Fn(Vec<Ty>, Box<Ty>)` → `Ty::Fn(Vec<Ty>, EffectRow, Box<Ty>)`. Introduced pure-by-default in Task 2 (mechanical, suite green), made meaningful in Task 4.

---

## File Structure

| File | Responsibility | 3b change |
|---|---|---|
| `scripts/githooks/pre-commit` | fmt guard | **new** — `cargo fmt --all -- --check`; installed via `core.hooksPath` |
| `src/ast.rs` | AST + pretty | `FnDecl.effect_row: Vec<String>` → `Option<Vec<Spanned<String>>>` (absent vs explicit `/ {}`); pretty unchanged (already ignores the row) |
| `src/parse.rs` | parser | `effect_row()` returns spanned labels; `fn_decl` records `None` when absent, `Some(labels)` when `/ {…}` present (incl. empty) |
| `src/types.rs` | HM + rows | **the bulk**: `EffectRow`/`RowTail`/`RowVar`, `row_subst`, `resolve_row`, `unify_row`, `add_effect`/`add_row`, ambient threading, op-signature table, `handle`/`resume` typing, row generalize/instantiate, discharge pass, `E0420`/`E0421`/`E0423`/`E0424`, row rendering |
| `src/eval.rs` | evaluators | **unchanged** — `Handle`/`Resume` still return the 3c "not evaluated yet" runtime error; `Ty::Fn` is not referenced by eval |
| `tests/ui/*.elya` | effect-error fixtures | **new**: `unhandled_effect.elya` (E0420), `impure_pure_fn.elya` (E0421), `row_mismatch.elya` (E0423), `cyclic_row.elya` (E0424) |
| `tests/effect_syntax.rs` | integration | `effects_are_not_type_checked_yet` → `effects_type_check_but_dont_evaluate_yet`; add row-poly + discharge assertions |
| `tests/effect_types.rs` | **new** — inferred-scheme snapshots (`insta`) incl. rows | new |

No lexer change. No resolver change (3a already resolves operations + `resume`). No machine change (3c).

---

## Task 0: Per-task fmt guard (pre-commit hook)

**Files:** Create `scripts/githooks/pre-commit`. Configure local git.

**Rationale:** the 3a slip (an unformatted `tests/effect_syntax.rs` reached `origin/main` because the commit ran in the same block as a failing gate). A committed hook makes it structurally impossible, per the user's request — "so it can't recur even without the memory note."

- [ ] **Step 1: Write the hook.** `scripts/githooks/pre-commit`:
```sh
#!/usr/bin/env sh
# Block commits that aren't rustfmt-clean. Cargo isn't on the default PATH here.
export PATH="$HOME/.cargo/bin:$PATH"
if ! cargo fmt --all -- --check; then
  echo "pre-commit: rustfmt check failed — run 'cargo fmt --all' and re-stage." >&2
  exit 1
fi
```
- [ ] **Step 2: Install + verify.** `git config core.hooksPath scripts/githooks` (local config; note in the plan that a fresh clone re-runs this one-liner — the hook file itself is tracked/documented). `chmod +x scripts/githooks/pre-commit`. Verify: introduce a deliberate misformat, `git commit` → blocked; `cargo fmt --all` → commit succeeds.
- [ ] **Step 3: Commit + push** (`chore(dev): pre-commit rustfmt guard (core.hooksPath=scripts/githooks)`).

> Scope note: the hook checks **only fmt** (fast, the exact recurring slip). Clippy + tests remain the per-task `scripts/check.sh` gate — a pre-commit full build would be too slow to run on every commit.

---

## Task 1: Record effect *annotations* precisely (AST + parser)

**Why first:** `E0421`/`E0423` (Task 6) require distinguishing **no annotation** (infer the row, may be row-polymorphic) from an **explicit** `/ {…}` (exact-match the declared row) — and `/ {}` (explicitly pure). Today both absent and `/ {}` produce `effect_row: Vec<String> == vec![]`. Labels also need **spans** to point diagnostics at the declared effect. This is a small, additive change; it changes no *semantics* (nothing yet reads the row meaningfully) and keeps every pure program's output identical.

**Files:** Modify `src/ast.rs`, `src/parse.rs`.

**Interfaces (produces):**
- `FnDecl.effect_row: Option<Vec<Spanned<String>>>` — `None` = unannotated (infer), `Some(vec![])` = explicit pure `/ {}`, `Some([Log@s, …])` = declared exactly those.
- Parser: `fn_decl` sets `None` when no `/` follows the signature; `Some(effect_row())` when `/` is present (even if the brace list is empty). `effect_row()` returns `Vec<Spanned<String>>` (each label carries its span).

- [ ] **Step 1: Failing test.** Add to `src/parse.rs` tests:
```rust
    #[test]
    fn distinguishes_absent_from_explicit_pure_row() {
        let (m1, _) = parse_module(&Session::new(), "fn f() { 1 }\n");
        let Decl::Fn(f1) = &m1.decls[0].node else { panic!() };
        assert!(f1.effect_row.is_none(), "unannotated => None");
        let (m2, _) = parse_module(&Session::new(), "fn f() / {} { 1 }\n");
        let Decl::Fn(f2) = &m2.decls[0].node else { panic!() };
        assert_eq!(f2.effect_row.as_ref().map(|r| r.len()), Some(0), "explicit pure => Some([])");
    }
```
- [ ] **Step 2: Change the AST field + parser.** `ast.rs`: `effect_row: Option<Vec<Spanned<String>>>`; the two `#[cfg(test)]` builders set `effect_row: None`. `parse.rs`: change `effect_row()` to collect `Vec<Spanned<String>>` (attach `self.peek_span()` per label); in `fn_decl`, `let effect_row = if self.eat(&TokenKind::Slash) { Some(self.effect_row()) } else { None };`.
- [ ] **Step 3: One permitted existing-test edit.** `parse::tests::parses_hello_world_module` asserts `f.effect_row == vec!["IO".to_string()]`. Update the **shape only** to the `Option<Spanned>` form, asserting the **same label set** (`Int`-value-identical): e.g. `let labels: Vec<&str> = f.effect_row.as_ref().unwrap().iter().map(|l| l.node.as_str()).collect(); assert_eq!(labels, ["IO"]);`. This is the shape-forced analogue of 3a's one permitted edit; the asserted content (`{IO}` is declared) is unchanged. No other existing test touches `effect_row`.
- [ ] **Step 4: Gate + commit.** `cargo fmt --all && sh scripts/check.sh` → green (`pretty_decl` ignores the row, so pretty snapshots are byte-identical). Commit `feat(ast): record effect-row annotations as Option<Vec<Spanned<String>>> (absent vs explicit pure)`; push.

---

## Task 2: `EffectRow` + row union-find scaffolding; `Ty::Fn` grows a pure row (mechanical)

**Discipline:** the 3a-style behavior-preserving move. Introduce the row types and the `row_subst` union-find, grow `Ty::Fn`, and default **every** constructed `Fn` to a **pure closed** row. `unify_row` is **not** called yet; rows render as nothing when pure. Existing type output stays byte-identical.

**Files:** Modify `src/types.rs`.

**Interfaces (produces):**
```rust
pub type RowVar = u32;                       // indexes row_subst, distinct space from Ty::Var
#[derive(Clone, Debug, PartialEq)]
pub enum RowTail { Closed, Open(RowVar), ErrorRow }
#[derive(Clone, Debug, PartialEq)]
pub struct EffectRow {
    /// label -> the span that introduced it (provenance for E0420/E0423).
    pub labels: std::collections::BTreeMap<String, Span>,
    pub tail: RowTail,
}
impl EffectRow {
    pub fn pure() -> EffectRow { EffectRow { labels: BTreeMap::new(), tail: RowTail::Closed } }
    pub fn open(tail: RowVar) -> EffectRow { EffectRow { labels: BTreeMap::new(), tail: RowTail::Open(tail) } }
}
```
- `Ty::Fn(Vec<Ty>, EffectRow, Box<Ty>)`.
- `Infer` gains `row_subst: Vec<Option<EffectRow>>` and `pub fn fresh_row(&mut self) -> RowVar`.
- `pub fn resolve_row(&self, r: &EffectRow) -> EffectRow` — follow `Open(v)` through `row_subst`, **merging** any labels found along the chain (union of labels; deepest tail wins). Deep-zonk analogue of `resolve`.

- [ ] **Step 1: Grow the type, keep it pure.** Add the types above. Change `Ty::Fn` construction sites to insert `EffectRow::pure()`: `binop`/`unary` (none — they're base), `infer_call` (`Ty::Fn(arg_ts, EffectRow::pure(), Box::new(result))`), `io.println` want/got, `infer_schemes` member `Ty::Fn(params, EffectRow::pure(), Box::new(result))`, and the four `#[cfg(test)]` constructors in the `types::tests` module (`Ty::Fn(vec![...], EffectRow::pure(), Box::new(...))`).
- [ ] **Step 2: Mechanical `Ty::Fn` fan-out.** Update every match on `Ty::Fn(ps, r)` to `Ty::Fn(ps, _row, r)` (or bind `row`): `resolve` (recurse into `row`? — for Task 2 just carry it: `Ty::Fn(ps.map(resolve), self.resolve_row(row), Box::new(resolve(r)))`), `occurs` (recurse params+ret; rows in Task 3), `unify` (the `Fn,Fn` arm: unify params+ret as today; **row unification is Task 4** — for Task 2 leave rows ununified, they're all `pure()` so trivially equal), `free_vars`, `subst_vars`, `write_ty`.
- [ ] **Step 3: Render pure rows as nothing.** `write_ty` `Ty::Fn` arm: after params, **only if** the (resolved) row is non-pure, emit ` / ` + `write_row`. Pure ⇒ emit `") -> "` exactly as today. Add `fn write_row(row:&EffectRow, names:&mut HashMap<u32,String>, out:&mut String)` (labels sorted, joined `{A, B}`; `Open` tail with a relevant var ⇒ ` | <letter>` sharing the `names` pool). In Task 2 all rows are pure so `write_row` is exercised only by its own unit test.
- [ ] **Step 4: Gate + commit.** `cargo fmt --all && sh scripts/check.sh` → green; **all printed types byte-identical** (every function is pure so no row prints). Add a focused unit test `pure_fn_prints_without_row` (`display_ty` of `Ty::Fn(vec![int], pure, unit)` == `"fn(Int) -> Unit"`). Commit `feat(types): EffectRow + row_subst union-find; Ty::Fn carries a (pure-by-default) row`; push.

---

## Task 3: `unify_row` (row rewriting) + `add_effect`/`add_row` + occurs-check (`E0424`)

**Files:** Modify `src/types.rs`. Unit-tested in isolation — **no inference wiring yet**.

**Interfaces (produces):**
```rust
pub struct RowConflict {
    pub only1: Vec<(String, Span)>,   // in r1, not absorbable by r2 (r2 closed)
    pub only2: Vec<(String, Span)>,   // in r2, not absorbable by r1 (r1 closed)
}
/// Rewriting unification of simple rows (Rémy/Leijen/Koka). Diagnostic-free
/// except the row occurs-check (E0424), which is pushed here like E0401 in bind.
pub fn unify_row(&mut self, r1: &EffectRow, r2: &EffectRow, span: Span) -> Result<(), RowConflict>;
/// amb := amb ∪ {op@span}: force `op ∈ amb`, rewriting amb's tail to expose it.
pub fn add_effect(&mut self, amb: RowVar, op: &str, span: Span) -> Result<(), RowConflict>;
/// amb := amb ∪ eff: fold all of eff's labels into amb.
pub fn add_row(&mut self, amb: RowVar, eff: &EffectRow, span: Span) -> Result<(), RowConflict>;
fn bind_row(&mut self, v: RowVar, r: &EffectRow, span: Span); // occurs-check → E0424, poison ErrorRow
```

**Algorithm (spec §3.2), spelled out:**
1. `r1 = self.resolve_row(r1); r2 = self.resolve_row(r2)`.
2. `common = labels present in both` — compatible (idempotent; monomorphic ops ⇒ no payloads to reconcile in 3b). Drop from further consideration.
3. `only1 = r1.labels \ r2.labels`; `only2 = r2.labels \ r1.labels`.
4. **Absorb each side's extras into the other's tail:**
   - `only1` into `r2.tail`: if `Open(σ)` ⇒ `bind_row(σ, { only1 } | Open(fresh σ'))`; if `Closed` and `only1` non-empty ⇒ collect into `conflict.only1` (do **not** push a diag — return it); if `ErrorRow` ⇒ ok (poison absorbs).
   - `only2` into `r1.tail`: symmetric.
5. **Unify the residual tails:** `Open(a) ~ Open(b)` ⇒ `bind_row(a, Open(b))`; `Closed ~ Closed` ⇒ ok; `Open(a) ~ Closed` ⇒ `bind_row(a, Closed)` (close it); any `ErrorRow` ⇒ ok.
6. If `conflict` accumulated any `only1`/`only2`, return `Err(conflict)`; else `Ok(())`.
- **`bind_row(v, r)`**: if `r`'s resolved tail is `Open(v)` **or** `v` appears in a tail-cycle ⇒ push `E0424` ("an effect row would contain itself"), set `row_subst[v] = ErrorRow`, return. Else `row_subst[v] = r`.
- **`add_effect(amb, op, span)`** = `unify_row(&EffectRow::open(amb), &{ op@span } | Open(fresh), span)`. **`add_row(amb, eff, span)`** = `unify_row(&EffectRow::open(amb), eff, span)`.

- [ ] **Step 1: Failing unit tests** (in `types::tests`), asserting the primitive directly:
```rust
    #[test] fn add_effect_extends_open_row() { /* fresh amb; add_effect Log; resolve_row(amb) has {Log}, tail Open */ }
    #[test] fn two_open_rows_reconcile_by_mutual_extension() { /* {Log}|ρ ~ {Net}|σ => Ok, both now ⊇ {Log,Net} */ }
    #[test] fn closed_row_meeting_extra_is_conflict_not_diag() { /* {Log}Closed ~ {Log,Net}… => Err(only={Net}); inf.diags empty */ }
    #[test] fn cyclic_row_is_e0424() { /* bind σ := {A}|Open(σ) => one diag E0424 */ }
```
- [ ] **Step 2: Implement** per the algorithm above.
- [ ] **Step 3: Gate + commit.** Suite green (no inference path calls these yet). Commit `feat(types): unify_row (rewriting) + add_effect/add_row + row occurs-check E0424`; push.

---

## Task 4: Ambient-row inference — operations, calls, builtins; row generalize/instantiate

**Files:** Modify `src/types.rs`. (`Handle`/`Resume` remain E0499 until Task 5.)

**Interfaces / decisions:**
- **Thread the ambient explicitly** (matches spec §3.3 `infer_expr(e, env, amb)`): change signatures to `infer_expr(&mut self, e, env, amb: RowVar) -> Ty`, `infer_block(&mut self, b, env, amb) -> Ty`, `infer_call(&mut self, callee, args, span, env, amb) -> Ty`. `amb` is the ambient **row variable**; effects fold into it via `add_effect`/`add_row` (which re-resolve the current tail each call). Every existing call site passes the enclosing `amb` down (sequencing unions effects — spec §3.3 `if`/`let`/block row).
- **Operation-signature table.** Build once in `infer_schemes` from `Decl::Effect`: `ops: HashMap<String, OpInfo>` where `OpInfo { effect: String, params: Vec<Ty>, ret: Ty }`, elaborating each `OpSig`'s `param_tys`/`ret` `TypeAnn` via `fn elaborate_ty(&TypeAnn) -> Ty` (base names → `Base`; `Unit`→Unit; unknown base ⇒ push a diagnostic + `Ty::Error`; **monomorphic only** — a lowercase type-var name in an op sig is out of scope for 3b, spec §1.3 ⇒ diagnostic). Thread `ops` into `Infer` (a field, populated before inference).
- **Perform rule.** In `infer_call`, **before** the generic path: if `callee` is `Expr::Var(name)` and `name ∈ ops`, this is a *perform*: infer args into `amb`, unify each against `OpInfo.params`, `add_effect(amb, ops[name].effect, span)` (on `Err` → caller records; but at a perform the ambient is always open, so it never conflicts — the conflict surfaces at the function boundary/discharge, Task 6), result = `OpInfo.ret`.
- **Builtin `io.println`.** Re-typed with `{IO}`: infer arg (into `amb`), unify arg `: String`, `add_effect(amb, "IO", span)`, result `Unit`. (Replaces the pure `Fn` in `infer_call`'s qualified branch.)
- **Call rule (ordinary fn).** `f : Fn(params, ε_f, R)` from `instantiate` (now freshens row vars too — below): infer callee+args into `amb`; `unify` param/ret types as today; **`add_row(amb, ε_f, span)`** to pour the callee's latent effects into the ambient; result `R`.
- **Function definition (unannotated case here; annotated exact-match is Task 6).** In `infer_schemes` step 2, give each member a **fresh open ambient** `amb_f = self.fresh_row()`; the member's `Ty::Fn` uses `EffectRow::open(amb_f)` as its row; infer the body with `amb = amb_f`; unify body type with the result. In step 3, **generalize** over free type vars **and** free row vars (below). This yields row-polymorphic schemes for pure/relaying functions — the spec §3.5 `run_it` result.
- **Row generalization/instantiation.** `Scheme { vars: Vec<u32>, row_vars: Vec<RowVar>, ty: Ty }`. `instantiate`: freshen `vars`→fresh `Ty`, `row_vars`→fresh `RowVar`, substitute both (extend `subst_vars` to also rewrite rows via a `row_map`). `generalize`/`generalize_toplevel`: collect free row vars of the type not free in the env (add `free_row_vars`/`env_free_row_vars`, mirroring the type versions). `display_scheme`: assign letters to `vars` then continue the **same** counter for `row_vars` (no collision), so `run_it : forall a b. fn(fn() / {b} -> a) / {b} -> a` (a=type, b=row). Snapshot pins the exact letters.

- [ ] **Step 1: Failing tests.**
  - `tests/effect_types.rs` (new, `insta`): `run_it(g){ g() }` infers a **row-polymorphic** scheme; `greet` performing `Log` infers `fn(String) / {Log} -> Unit`; a pure `fn add(a,b){a+b}` still infers `fn(Int, Int) -> Int` (no row printed).
  - `types::tests`: perform threads the effect (`log("x")` under a fresh amb ⇒ `resolve_row(amb) ⊇ {Log}`); `io.println("x")` ⇒ amb ⊇ `{IO}`.
- [ ] **Step 2: Implement** signatures, `ops` table + `elaborate_ty`, perform/call/builtin rules, member open-ambient, row generalize/instantiate, `display_scheme` rows. Update all `infer_expr`/`infer_block` call sites (incl. `types::tests::infer_expr_str`, which now creates `let amb = inf.fresh_row();`).
- [ ] **Step 3: Gate + commit.** Pure-program output byte-identical (rows print only when non-empty); effect functions get rows; `run_it` is row-poly. Commit `feat(types): ambient-row inference — perform/call/builtin effect threading + row generalization`; push.

---

## Task 5: Type `handle` + `resume`; type-level discharge (remove `E0499`)

**Files:** Modify `src/types.rs`, `tests/effect_syntax.rs` (the one deliberate test change).

**Typing `handle e with H` (spec §3.4), spelled out:**
1. Determine the handled effect `E` from the clauses — every `OpClause.effect` must name the same effect (all clauses cover ops of one `E`); the resolver already parsed `Effect.op`. If clauses disagree or an op isn't in `E`'s declared ops ⇒ diagnostic (reuse `E0423` "effect row mismatch" with a clause-level message, or a dedicated resolve-time check; **decided here**: a types-level check emitting `E0423` naming the stray op — flag in Self-Review).
2. `amb_in = self.fresh_row()` then `add_effect(amb_in, E, span)` — `e` may perform `E` plus a polymorphic remainder.
3. `t_e = infer_expr(e, env, amb_in)`.
4. `amb_out` = the **enclosing** ambient (the `amb` passed to this `handle`). For each clause `Op(x) -> body` with `Op : (A…) -> B ∈ E`: push scope, bind each param to its declared `A`, **push resume type** `(B, R, amb_out)` on `Infer.resume_stack`, infer `body` with `amb = amb_out`, unify `body : R` (fresh `R = self.fresh()` shared across clauses + return), pop resume, pop scope.
5. Return clause `return(x) -> body`: bind `x : t_e`, infer `body : R` with `amb = amb_out`. Absent ⇒ `R = t_e`.
6. **Discharge:** the handled `E` is subtracted — `amb_in`'s residual (everything it performed *except* `E`) must flow into `amb_out`: `unify_row(EffectRow::open(amb_in-without-E), EffectRow::open(amb_out))`. Concretely: resolve `amb_in`, drop label `E`, `add_row(amb_out, that_residual)`. Result type of the whole `handle` is `R`.
- **`Expr::Resume { arg }` typing:** peek `Infer.resume_stack` top `(B, R, amb_out)`; infer `arg` into the current `amb`; unify `arg : B`; `add_row(current_amb, amb_out_row)`; result `R`. (Stack non-empty is guaranteed — the resolver's `E0210` already rejects resume outside a handler; if empty, return `Ty::Error` defensively.)

- [ ] **Step 1: Deliberate test change (flagged).** In `tests/effect_syntax.rs`, replace `effects_are_not_type_checked_yet` (which asserts `check_source` yields `E0499`) with `effects_type_check_but_dont_evaluate_yet`: `check_source(handle-program).is_ok()` **and** `run_source(same).unwrap_err().contains("not evaluated yet")` (the 3c eval placeholder). *This is the feature landing:* 3b's job is to make effects type-check, so the E0499 premise is intentionally obsoleted — not an accommodation. The three parse+resolve integration tests are unchanged.
- [ ] **Step 2: Implement** `Handle`/`Resume` arms (remove the `E0499` arm entirely); add `resume_stack: Vec<(Ty, Ty, EffectRow)>` to `Infer`; the single-effect-per-handler check (Step-1 of the typing).
- [ ] **Step 3: Type-only tests.** A `handle` over a `Log`-performing body type-checks and **discharges** `Log` (the enclosing function is pure again); `resume(v)`'s result type participates (`Flip.flip() -> resume(True)` type-checks with `B = Bool`).
- [ ] **Step 4: Gate + commit.** `sh scripts/check.sh` green (eval still errors on handle — but `check.sh`'s effect tests use `check_source`; `run_source` handle tests assert the 3c message). Commit `feat(types): type handle/resume with effect discharge; remove E0499 placeholder`; push.

---

## Task 6: Exact-match enforcement (`E0421`/`E0423`) + discharge pass (`E0420`)

**Files:** Modify `src/types.rs`; create four `tests/ui/*.elya`; register them in `tests/ui.rs`.

**Exact-match check (spec §3.6), per function, after body inference (`infer_schemes` step 2→3):**
Zonk the function's ambient to a **ground performed label-set `P`** (`resolve_row(amb_f).labels`). If the function is **annotated** with declared set `D` (from `Some(effect_row)` labels, Task 1):
- `D` empty and `P` non-empty ⇒ **`E0421`** — "declared pure but performs `{P}`" (point at the fn signature; the declared row is `{}`).
- else: `extra = P \ D`, `unused = D \ P`.
  - `extra` non-empty ⇒ **`E0423`** — "declared `{D}`, but the body also performs `{extra}`; the rows differ by exactly `{extra}`" (label span = the perform site via provenance).
  - `unused` non-empty ⇒ **`E0423`** — "declared `{D}`, but the body never performs `{unused}`; the rows differ by exactly `{unused}`" (label span = the annotation label). **Strict default** (spec §3.6, revisitable): declaring-more is an error, not a warning — a one-line policy that can later flip to `Severity::Warning` without touching the subtyping story.

**Discharge pass (`E0420`) for `main`:** after all schemes, find `main`; compute its residual performed set (annotated ⇒ `D`; unannotated ⇒ zonk `P`). Any label **outside `{IO}`** ⇒ `E0420` — "effect `{L}` is never handled … `main` may perform only `{IO}`", naming the operation call-site from **provenance** (the `Span` stored in `EffectRow.labels`), with a fix-it help (`handle … with { L.op(..) -> … }`). *(A user effect surviving to `main` is the canonical unhandled case in 3b; a `handle` that leaves an effect undischarged simply relays it outward until it hits `main` and fires here — the type-level discharge in Task 5 does the subtraction, this pass reports what's left.)*

**Rendering (all E042x):** every message runs rows through `resolve_row` then `write_row` (named, sorted labels; `| e` only when a polymorphic tail is genuinely relevant) — **never** a `%r`/`%e`/`%row` token. Concrete fixtures (exact wording pinned by the UI tests):
```
error[E0420]: effect `Log` is never handled
  = performed by operation `log` (row {Log}); `main` may perform only {IO}
  = help: handle it — handle <expr> with { Log.log(m) -> ... }

error[E0421]: this function is declared pure but performs `Log`
  = the declared row is {} (pure); the body performs {Log}

error[E0423]: effect row mismatch
  = declared {Log}; the body also performs {Net}
  = the rows differ by exactly: {Net}

error[E0424]: cyclic effect row
  = an effect row would contain itself
```

- [ ] **Step 1: Failing UI fixtures** (`//~ ERROR[Ennnn] substring`), registered in `tests/ui.rs`:
  - `unhandled_effect.elya` — `main` calls a `Log`-performing fn with no handler ⇒ `E0420`.
  - `impure_pure_fn.elya` — `fn f() / {} { log("x") }` ⇒ `E0421`.
  - `row_mismatch.elya` — `fn f() / {Log} { log("x") net() }` (performs `Net` too) ⇒ `E0423` "differ by exactly: {Net}".
  - `cyclic_row.elya` — a construction that forces a row occurs-check ⇒ `E0424` (may need a targeted `types::tests` unit rather than surface syntax if unconstructible from source; **flag**: if no surface program yields E0424, keep the Task-3 unit test as its coverage and drop the fixture).
  The existing `ui.rs` `%`-token scan already forbids `%r`/`%e` — extend the `bad` list with `"%row"` for belt-and-suspenders.
- [ ] **Step 2: Implement** the exact-match check + the `main` discharge pass; wire provenance spans into messages.
- [ ] **Step 3: Gate + commit.** `sh scripts/check.sh` green; each fixture asserts named labels + no `%` token. Commit `feat(types): effect discharge pass — E0420 unhandled, E0421 purity, E0423 exact-match mismatch`; push.

---

## Task 7: Integration + 3b exit gate

**Files:** `tests/effect_syntax.rs`, `tests/effect_types.rs` (snapshots), plus the effect-free cross-check confirmation.

- [ ] **Step 1: Integration assertions.**
  - **Row-poly, end to end:** a program with `run_it` applied to both a `Log`-performing fn and a pure fn type-checks; `run_it`'s scheme snapshot includes its row var.
  - **Discharge, end to end:** `handle greet("ada") with { Log.log(m) -> { io.println(m) resume(Unit) } return(x) -> x }` inside `main` **type-checks clean** (Log discharged; only `{IO}` remains at `main`); removing the handler ⇒ `E0420`.
  - **Effect-free preserved:** the Slice-2 cross-check (`cek == tree`) is unchanged and green — effect programs are **not** run here (still eval-gated to 3c); this task adds no `run_source` executions of handlers beyond asserting the 3c placeholder message.
- [ ] **Step 2: The 3b exit gate.** `cargo fmt --all && sh scripts/check.sh` fully green: every pure program's inferred type is byte-identical to pre-3b; effect functions carry correct rows; `handle` discharges; `E0420`/`E0421`/`E0423`/`E0424` fixtures pass with named labels and no `%`-token; `run_it` row-poly snapshot pinned; layering unchanged (`types=4`). Push.
- [ ] **Step 3: Commit + push** (`test(effects): 3b integration — row-poly inference, discharge, E042x fixtures; effect-free suite green`).

---

## Self-Review

**1. Spec coverage (spec §9 sub-slice 3b → tasks).**
- Row union-find + `unify_row` (rewriting) + occurs (`E0424`) — spec §3.1, §3.2 → **Task 3** (primitive, diagnostic-free except E0424, returns `RowConflict`).
- Ambient-row inference; builtins re-typed (`io.println / {IO}`); `run_it` row-poly; scheme+row snapshots — spec §3.3, §3.5 → **Task 4**.
- Typing `handle`/`resume` + type-level discharge — spec §3.4 → **Task 5** (removes E0499).
- Discharge pass: `E0420`/`E0421`/`E0423`, zonked named-label rendering, no-`%r` UI fixtures — spec §5 → **Task 6**.
- Exact-match annotation (`E0423`), strict-default, revisitable-to-warn — spec §3.6 → **Task 6** (`P` vs `D` set comparison; empty-`D` ⇒ `E0421`).
- **`E0422` deliberately unused** (spec §5 gap) — Global Constraints.

**2. Every deferral to 3c/3d/3e flagged.**
- **Handler/`resume` *evaluation*** (HandleK, perform/capture, deep-handler resume) — **3c**; eval keeps the "not evaluated yet" error, unchanged in 3b (`eval.rs` untouched). `check_source` types effects; `run_source` still errors — asserted in Task 5.
- **One-shot enforcement `E0425`** — 3c. **Multi-shot + `E0426`** — 3d. **Effect-TCE** (tail-resume splice, `K_MAX_EFF`) — 3e.
- **Generic/parametric effects & op payloads** — deferred (spec §1.3); `elaborate_ty` rejects non-base op signatures with a diagnostic rather than silently inferring.
- **Effect-annotation widening** — rejected as subtyping (spec §3.6/§11); exact-match is enforced, not relaxed.

**3. Deliberate (non-accommodation) test changes — enumerated, matching the strict-gate philosophy:**
- **Task 1:** `parses_hello_world_module`'s `effect_row` assertion — *shape-forced* to the `Option<Spanned>` type, same label content (`{IO}`). The only existing parser-test edit; analogous to 3a's single permitted construction edit.
- **Task 5:** `effects_are_not_type_checked_yet` → `effects_type_check_but_dont_evaluate_yet` — the E0499 premise is *obsoleted by the feature*, not bent to pass. Everything else (all pure-program tests, printed types, cross-check) stays byte-identical — the effect-free preservation gate.

**4. Type/name consistency:** `EffectRow{labels:BTreeMap<String,Span>, tail:RowTail}`, `RowTail::{Closed,Open(RowVar),ErrorRow}`, `RowVar=u32`, `row_subst`, `resolve_row`, `unify_row -> Result<(),RowConflict>`, `add_effect`/`add_row`, `Ty::Fn(Vec<Ty>,EffectRow,Box<Ty>)`, `Scheme{vars,row_vars,ty}`, `OpInfo{effect,params,ret}`, `Infer.resume_stack:Vec<(Ty,Ty,EffectRow)>`, `FnDecl.effect_row:Option<Vec<Spanned<String>>>` — used identically across Tasks 1–7. Diagnostic codes: `E0420`/`E0421`/`E0423`/`E0424` (this slice), `E0499` removed, `E0422` reserved-unused.

**5. Two design calls flagged for the reviewer:**
- **(a) Single-effect-per-handler check placement.** Task 5 puts "all clauses cover one effect `E`, ops belong to `E`" as a *types-level* `E0423` check. Alternative: a *resolve-level* check (3a already resolves `Effect.op`). Types-level keeps 3b self-contained and lets the message name the effect row; called out in case you prefer it in the resolver.
- **(b) `E0424` surface reachability.** A cyclic effect row may be unconstructible from the 3b surface syntax (no higher-order effect binders yet). If so, its coverage is the Task-3 **unit** test and the `cyclic_row.elya` fixture is dropped (noted in Task 6 Step 1). Flagged so the exit gate isn't blocked on an unreachable fixture.

---

## Execution Handoff

Plan complete — **paused for review; no code written.** This is the first sub-slice with real effect semantics in the type system (row unification, ambient inference, discharge, exact-match), so it goes to you before any implementation. On approval, two execution options:

1. **Subagent-Driven** — dispatch a fresh subagent per task, review between tasks.
2. **Inline Execution** — execute Tasks 0–7 in this session with checkpoints.

Which approach — and any changes to the two flagged design calls (5a, 5b) or the strict-default `E0423` severity?
