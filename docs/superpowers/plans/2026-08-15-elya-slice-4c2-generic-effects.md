# Elya Slice 4c-2 — Generic (Parametric) Effects — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let one `effect State(s)` serve any state type — effect declarations carry type parameters, the effect row carries type arguments, and `unify_row` reconciles them — while every existing (monomorphic) effect program stays byte-for-byte unchanged.

**Architecture:** Design B: the effect's type argument rides in the row (a soundness requirement — a function can use it concretely without it appearing elsewhere in its type). The change is the 4a `Ty::Con` pattern one level up: a nominal head + `Vec<Ty>` args, unified by head then **pairwise args**; the monomorphic path is the arity-0 case. The representation + `unify_row` change lands **arity-0-only first**, behind a hard regression gate, before any parametric surface consumes it.

**Tech Stack:** Rust 2021, `logos`, `ariadne`, `insta`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-15-elya-slice-4c2-generic-effects-design.md`

## Global Constraints

- **Rust 2021, MSRV 1.75. No new dependencies. No new diagnostic code** — a conflicting instantiation reuses **`E0423`** (§8 of the spec).
- **Every shell session:** `export PATH="$HOME/.cargo/bin:$PATH"` and `export CARGO_INCREMENTAL=0`.
- **Before every `sh scripts/check.sh`:** run `cargo fmt --all` (fmt-check fails hard).
- **Commits:** end every message with `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`. Use **explicit `git add` paths** (never `git add -A`). Push to `origin/main` after each task.
- **TASK-1 REGRESSION GATE (load-bearing, strict):** the representation + `unify_row` change is **behavior-preserving on the existing corpus**. **The entire existing suite must pass UNTOUCHED — no editing an existing test to accommodate the change.** If a current test needs editing to pass, that is a **regression in the code to fix**, not a test to adjust. Monomorphic inferred-scheme snapshots (`tests/effect_types.rs`) must be **unchanged** (a monomorphic effect still prints `Log`, `IO`, `State` — never `Log()`).
- **OUTPUT-VERIFIED for runs:** effect programs run CEK-only via `run_source`; assert exact output or a specific diagnostic code — never "it runs".
- **Runtime is untouched:** effects are erased at runtime (type arguments are static). Do not modify `eval.rs`.
- Scratch/debug files go in the session scratchpad **outside** the repo.

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `src/types.rs` | HM + effect-row inference | **The substance:** `EffectLabel { args, span }` + fan-out; `unify_row` pairwise-arg arm; `OpInfo.effect_params` + perform/handle instantiation; op-signature elaboration under a param-env. |
| `src/ast.rs` | AST | `EffectDecl.params: Vec<String>`. |
| `src/parse.rs` | Parser | `effect Name(p, …)` param list (mirror `type_decl`). |
| `tests/generic_effects.rs` (new) | Parametric demonstration + soundness negative | The `State(s)`-at-two-types run, the cross-instantiation `E0423`. |
| `tests/effect_types.rs` | Scheme pins | Add the inferred-row pins (`{State(String)}` concrete, polymorphic relay); existing pins UNCHANGED. |

`eval.rs`, `resolve.rs` (beyond effect-param scope), and `tests/arch/layering.rs` are unchanged.

---

## Task 1: `EffectLabel` representation + `unify_row` arg arm — arity-0 only, behind the regression gate

**Files:**
- Modify: `src/types.rs`
- Test: `src/types.rs` inline unit test (the existing `unify_row` unit-test neighborhood, ~line 1998)

**Interfaces:**
- Produces: `pub struct EffectLabel { pub args: Vec<Ty>, pub span: Span }`; `EffectRow.labels: BTreeMap<String, EffectLabel>`; `add_effect(&mut self, amb, op, args: Vec<Ty>, span)`.

- [ ] **Step 1: Change the representation.** In `src/types.rs`, add the struct and change the field:

```rust
/// One effect in a row: its type arguments (empty for a monomorphic effect —
/// the arity-0 case that reproduces Slice-3 behavior) and the span that
/// introduced it.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectLabel {
    pub args: Vec<Ty>,
    pub span: Span,
}

// in `struct EffectRow`:
pub labels: BTreeMap<String, EffectLabel>,
```

- [ ] **Step 2: Build; fix each compile error per the pattern (compiler-driven fan-out).**

Run: `cargo build 2>&1 | grep -E "^error" | head -40`
The struct change flags every site. Apply, at each:
- **`EffectRow::pure`/`open`, `bind_row`'s ErrorRow, `absorb`'s reconstruction** — `labels: BTreeMap::new()` stays (empty map, unchanged).
- **`resolve_row`** (~176): when copying/merging labels, `resolve` each label's `args` (they contain type vars): map each `EffectLabel { args, span }` to `EffectLabel { args: args.iter().map(|a| self.resolve(a)).collect(), span }`.
- **`subst_vars`** (~1436): replace `labels: row.labels.clone()` with a map that runs `subst_vars(arg, m, rm)` over each label's `args`.
- **`add_effect`** (~422): new signature `add_effect(&mut self, amb, op: &str, args: Vec<Ty>, span)`; build the target label `EffectLabel { args, span }`.
- **`add_row`** (~436): `for (label, l) in &eff.labels { self.add_effect(amb, label, l.args.clone(), l.span)?; }`.
- **`unify_row` only1/only2** (~325): the `filter`/`map` now carries the label; keep the residual-tail logic (`absorb` operates on the label set — pass `(name, args, span)` triples or keep names+spans and look args up; simplest: `absorb` reconstructs `EffectLabel`s).
- **All `add_effect(amb, "IO"/op.effect/e, span)` call sites** (io.println builtin ~884; perform ~969; `infer_handle` seed ~1002) — insert `Vec::new()` as the args argument for now (arity-0; Task 3 makes perform/handle pass real args).
- **`emit_row_mismatch`, `check_exact_row`, `check_main_discharge`, the `E0426` dup filter** — these iterate label **names/keys**; adjust the tuple pattern (`(k, l)` instead of `(k, sp)`, use `l.span`) but keep name-based logic.
- **`write_ty` row printing** (~596): print each label with its args — see Step 4.
- **Test helpers** (`row(...)` ~1998, any `labels.insert`/`contains_key`): construct `EffectLabel { args: Vec::new(), span: … }`; `contains_key` by name is unchanged.

- [ ] **Step 3: Add the type-var and row-var descents into label args (the flagged fan-out).** In `free_vars`, the `Ty::Fn(ps, row, r)` arm currently ignores `row`; make it descend:

```rust
        Ty::Fn(ps, row, r) => {
            for p in &ps {
                free_vars(inf, p, acc);
            }
            // A type variable may appear ONLY in an effect argument (e.g. the `s`
            // of `{State(s)}`); it must still be generalized.
            for l in row.labels.values() {
                for a in &l.args {
                    free_vars(inf, a, acc);
                }
            }
            free_vars(inf, &r, acc);
        }
```

In `free_row_vars`, the `Ty::Fn` arm likewise descends into label args (an argument may itself be a function type carrying a row):

```rust
            for l in row.labels.values() {
                for a in &l.args {
                    free_row_vars(inf, a, acc);
                }
            }
```

- [ ] **Step 4: Row printing.** In `write_ty` (~596), render each label with its args (empty args ⇒ bare name, so monomorphic output is unchanged):

```rust
    for (i, (name, l)) in row.labels.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(name);
        if !l.args.is_empty() {
            out.push('(');
            for (j, a) in l.args.iter().enumerate() {
                if j > 0 {
                    out.push_str(", ");
                }
                write_ty(a, names, out);
            }
            out.push(')');
        }
    }
```

(Adjust to the actual local variable names in `write_ty`; the invariant is empty args print exactly as today.)

- [ ] **Step 5: The `unify_row` pairwise-arg reconciliation arm.** In `unify_row`, after resolving `r1`/`r2` and before/alongside the `only1`/`only2` split, reconcile labels present on both sides:

```rust
        // Design B: a label on BOTH sides must agree on its type arguments.
        // (Monomorphic effects have empty args -> this is a no-op, exactly the
        // Slice-3 behavior.)
        let shared: Vec<(String, Vec<Ty>, Vec<Ty>)> = r1
            .labels
            .iter()
            .filter_map(|(k, l1)| r2.labels.get(k).map(|l2| (k.clone(), l1.args.clone(), l2.args.clone())))
            .collect();
        for (effect, a1, a2) in shared {
            self.unify_effect_args(&effect, &a1, &a2, span);
        }
```

and the helper (reuses `E0423`, poisons to avoid an `E0400` cascade):

```rust
    /// Reconcile the type arguments of one effect present in two rows. A pair of
    /// concrete, unequal arguments is an effect used at conflicting types -> E0423.
    fn unify_effect_args(&mut self, effect: &str, a1: &[Ty], a2: &[Ty], span: Span) {
        for (x, y) in a1.iter().zip(a2) {
            let rx = self.resolve(x);
            let ry = self.resolve(y);
            let both_concrete = !matches!(rx, Ty::Var(_) | Ty::Error)
                && !matches!(ry, Ty::Var(_) | Ty::Error);
            if both_concrete && rx != ry {
                self.diags.push(
                    Diagnostic::error("E0423", "effect row mismatch")
                        .with_label(span, "the same effect is used at different types here")
                        .with_help(format!(
                            "effect `{effect}` is used at conflicting type arguments: `{}` vs `{}`",
                            display_ty(self, &rx),
                            display_ty(self, &ry)
                        )),
                );
                return; // poison: one row-level diagnostic, no cascading E0400
            }
            self.unify(x, y, span);
        }
    }
```

- [ ] **Step 6: Unit-test the arg arm directly** (exercises the core change with no parser/parametric surface). In the `unify_row` unit-test neighborhood (`src/types.rs` tests), add — using the local `row` helper extended to accept args, or constructing `EffectRow` inline:

```rust
    #[test]
    fn unify_row_reconciles_matching_effect_args() {
        let mut inf = Infer::new();
        // {State(Int)} unifies with {State(Int)} -> ok.
        let r_int = EffectRow {
            labels: [("State".to_string(), EffectLabel { args: vec![Ty::int()], span: Span::EMPTY })]
                .into_iter().collect(),
            tail: RowTail::Closed,
        };
        assert!(inf.unify_row(&r_int, &r_int.clone(), Span::EMPTY).is_ok());
        assert!(inf.diags.is_empty(), "matching args must not diagnose");
    }

    #[test]
    fn unify_row_rejects_conflicting_effect_args_e0423() {
        let mut inf = Infer::new();
        let r_int = EffectRow {
            labels: [("State".to_string(), EffectLabel { args: vec![Ty::int()], span: Span::EMPTY })]
                .into_iter().collect(),
            tail: RowTail::Closed,
        };
        let r_str = EffectRow {
            labels: [("State".to_string(), EffectLabel { args: vec![Ty::str()], span: Span::EMPTY })]
                .into_iter().collect(),
            tail: RowTail::Closed,
        };
        let _ = inf.unify_row(&r_int, &r_str, Span::EMPTY);
        assert!(inf.diags.iter().any(|d| d.code == "E0423"),
            "State(Int) vs State(String) must be E0423: {:?}", inf.diags);
    }
```

- [ ] **Step 7: THE REGRESSION GATE (strict).** Build clean, then run the entire suite. **No existing test may be edited.**

Run: `cargo fmt --all && sh scripts/check.sh 2>&1 | grep -E "test result: FAILED|FAILED|Diff in|error" | head || echo GREEN`
Expected: the entire existing suite passes **untouched** (163 prior + 2 new unit tests), and monomorphic scheme snapshots are unchanged (`fn(String) / {Log} -> Unit` etc. still print with bare effect names). If any existing test fails, the representation change is not behavior-preserving — **fix the code, not the test** (most likely: a printing site emitting `Log()` instead of `Log`, or an arg descent misfiring).

- [ ] **Step 8: Commit + push.**

```bash
git add src/types.rs
git commit -m "$(printf 'feat(types): effect labels carry type arguments; unify_row reconciles them (arity-0)\n\nDesign B, the 4a Ty::Con pattern at the row level: EffectRow labels become\nEffectLabel { args, span }, unify_row reconciles a shared label pairwise\n(conflict -> E0423), free_vars/free_row_vars/subst descend into effect args.\nArity-0 only (no parametric surface yet): the entire existing suite passes\nUNTOUCHED and monomorphic snapshots are unchanged -- the representation change\nis behavior-preserving, proven before any parametric surface consumes it.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

---

## Task 2: Parametric effect declarations (parser + `OpInfo.effect_params` + param-env elaboration)

**Files:**
- Modify: `src/ast.rs` (`EffectDecl.params`), `src/parse.rs` (`effect_decl`), `src/types.rs` (`OpInfo.effect_params`, op-sig elaboration under a param-env)
- Test: `src/parse.rs` inline (parses); `tests/generic_effects.rs` (new — declaration accepted)

**Interfaces:**
- Consumes: `elaborate_adt_ty`'s param-env pattern (~1254); `type_decl`'s param parsing (~320).
- Produces: `EffectDecl { name, params: Vec<String>, ops }`; `OpInfo { effect, effect_params: Vec<u32>, params, ret }`.

- [ ] **Step 1: AST — `EffectDecl.params`.** In `src/ast.rs`, add `pub params: Vec<String>` to `EffectDecl` (default empty for a monomorphic effect). Update the `EffectDecl { .. }` constructions (the parser and any pretty-print/test) to set `params`.

- [ ] **Step 2: Parser — `effect Name(p, …)`.** In `src/parse.rs` `effect_decl` (~289), after reading the effect name, parse an optional parenthesized lowercase type-parameter list, mirroring `type_decl` (~320-345):

```rust
        let mut params = Vec::new();
        if self.eat(&TokenKind::LParen) {
            loop {
                match self.peek()?.clone() {
                    TokenKind::Lower(p) => { self.bump(); params.push(p); }
                    _ => { self.error(self.peek_span(), "expected a type parameter name"); return None; }
                }
                if !self.eat(&TokenKind::Comma) { break; }
            }
            if !self.eat(&TokenKind::RParen) {
                self.error(self.peek_span(), "expected `)`");
                return None;
            }
        }
```

Set `params` in the `EffectDecl { … }` it builds.

- [ ] **Step 3: Parser round-trip test.** In `src/parse.rs` tests:

```rust
#[test]
fn effect_decl_with_type_param_parses() {
    let src = "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\npub fn main() { io.println(\"x\") }\n";
    let (m, d) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    let Decl::Effect(e) = &m.decls[0].node else { panic!("expected effect") };
    assert_eq!(e.params, vec!["s".to_string()]);
}
```

- [ ] **Step 4: `OpInfo.effect_params` + param-env elaboration.** In `src/types.rs`: add `effect_params: Vec<u32>` to `OpInfo`. Where effect decls are elaborated (~1518), build a **param-env** for each effect (`{ "s" -> fresh Ty::Var }`), record the var ids as `effect_params`, and elaborate op signatures under it — replacing the E0404-rejecting `elaborate_ty` with a param-env-aware elaboration (the `elaborate_adt_ty` pattern: a bare name found in the param-env resolves to its var; base names resolve as today; anything else is the existing unknown-type error). A monomorphic effect has an empty param-env and `effect_params = []` — unchanged.

```rust
        if let Decl::Effect(e) = &d.node {
            // The effect's type parameters -> fresh type variables, shared by all
            // its operation signatures.
            let mut param_env: HashMap<String, Ty> = HashMap::new();
            let mut effect_params: Vec<u32> = Vec::new();
            for p in &e.params {
                let v = inf.fresh();
                if let Ty::Var(id) = v { effect_params.push(id); }
                param_env.insert(p.clone(), v);
            }
            for op in &e.ops {
                let sig = &op.node;
                let params: Vec<Ty> = sig.param_tys.iter()
                    .map(|t| elaborate_op_ty(&mut inf, t, &param_env)).collect();
                let ret = elaborate_op_ty(&mut inf, &sig.ret, &param_env);
                inf.ops.insert(sig.name.clone(), OpInfo {
                    effect: e.name.clone(),
                    effect_params: effect_params.clone(),
                    params,
                    ret,
                });
            }
        }
```

where `elaborate_op_ty` is the param-env-aware elaborator (a bare name in `param_env` resolves to its var; else base types; else the existing unknown-type / unsupported-generic error). Model it on `elaborate_adt_ty`.

- [ ] **Step 5: Build + run — declaration accepted.** Create `tests/generic_effects.rs` with a `run`/`check_ok` helper and a decl-acceptance test:

```rust
use elya::{check_source, run_source, Session};

fn run(src: &str) -> String {
    assert!(check_source("t.elya", src).is_ok(), "should type-check: {:?}", check_source("t.elya", src));
    let _ = Session::new();
    run_source("t.elya", src).expect("should run")
}

#[test]
fn generic_effect_declaration_is_accepted() {
    // Declaring `effect State(s)` no longer trips E0404; a program that only
    // declares it (and does something unrelated) type-checks and runs.
    let src = "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
               pub fn main() { io.println(\"ok\") }\n";
    assert_eq!(run(src), "ok\n");
}
```

Run: `cargo test --test generic_effects --test parse effect_decl_with_type_param 2>&1 | grep -E "test result|FAILED" | head`
Expected: PASS. Full suite still green.

- [ ] **Step 6: Commit + push.**

```bash
cargo fmt --all && sh scripts/check.sh
git add src/ast.rs src/parse.rs src/types.rs tests/generic_effects.rs
git commit -m "$(printf 'feat(effects): parametric effect declarations (effect State(s))\n\nEffectDecl.params + parser (mirrors type_decl); op signatures elaborate under a\nper-effect type-parameter env (the elaborate_adt_ty pattern), lifting the old\nE0404 generic-op rejection; OpInfo carries effect_params. Monomorphic effects\nhave an empty param-env (unchanged). Declaration parses, resolves, elaborates.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

---

## Task 3: Perform + handle instantiation — the parametric `State` demonstration

**Files:**
- Modify: `src/types.rs` (the operation-perform branch ~959; `infer_handle` seed + clause typing ~1000-1025)
- Test: `tests/generic_effects.rs`

**Interfaces:**
- Consumes: `OpInfo.effect_params` (Task 2); `add_effect(.., args, ..)` (Task 1).
- Produces: a helper `instantiate_op(&mut self, op: &OpInfo) -> (Vec<Ty> /*args*/, Vec<Ty> /*params*/, Ty /*ret*/)` that maps `effect_params` to fresh vars and substitutes.

- [ ] **Step 1: Write the failing demonstration.** In `tests/generic_effects.rs`:

```rust
#[test]
fn one_generic_state_used_at_int_and_string() {
    // ONE `effect State(s)` used at Int (a counter) AND String (append) in one
    // program. Proves the effect is parametric over its state type.
    let src = "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
               fn count() {\n\
                 let program = handle { let _ = set(7)  get() } with {\n\
                   State.get() -> fn(st) { (resume(st))(st) }\n\
                   State.set(v) -> fn(st) { (resume(Unit))(v) }\n\
                   return(x) -> fn(st) { x }\n\
                 }\n\
                 program(0)\n\
               }\n\
               fn label() {\n\
                 let program = handle { let _ = set(\"a\")  let x = get()  let _ = set(x <> \"b\")  get() } with {\n\
                   State.get() -> fn(st) { (resume(st))(st) }\n\
                   State.set(v) -> fn(st) { (resume(Unit))(v) }\n\
                   return(x) -> fn(st) { x }\n\
                 }\n\
                 program(\"init\")\n\
               }\n\
               pub fn main() { if count() == 7 { io.println(label()) } else { io.println(\"no\") } }\n";
    assert_eq!(run(src), "ab\n");
}
```

- [ ] **Step 2: Run — expect FAIL** (perform/handle don't yet instantiate `effect_params`, so `count` and `label` won't both type-check at distinct `s`).

Run: `cargo test --test generic_effects one_generic_state_used_at_int_and_string 2>&1 | grep -E "test result|FAILED|panicked" | head`
Expected: FAIL.

- [ ] **Step 3: Perform instantiation.** In `src/types.rs`, add the helper and use it in the operation-perform branch (~959):

```rust
    fn instantiate_op(&mut self, op: &OpInfo) -> (Vec<Ty>, Vec<Ty>, Ty) {
        let mut m: HashMap<u32, Ty> = HashMap::new();
        let mut args = Vec::with_capacity(op.effect_params.len());
        for &p in &op.effect_params {
            let fresh = self.fresh();
            m.insert(p, fresh.clone());
            args.push(fresh);
        }
        let params = op.params.iter().map(|t| subst_vars(t, &m, &HashMap::new())).collect();
        let ret = subst_vars(&op.ret, &m, &HashMap::new());
        (args, params, ret)
    }
```

```rust
        if let Expr::Var(name) = &callee.node {
            if let Some(op) = self.ops.get(name).cloned() {
                let (eff_args, op_params, op_ret) = self.instantiate_op(&op);
                let arg_ts: Vec<Ty> = args.iter().map(|a| self.infer_expr(a, env, amb)).collect();
                let want = Ty::Fn(op_params, EffectRow::pure(), Box::new(op_ret.clone()));
                let got = Ty::Fn(arg_ts, EffectRow::pure(), Box::new(op_ret.clone()));
                self.unify(&want, &got, span);
                let _ = self.add_effect(amb, &op.effect, eff_args, span);
                return op_ret;
            }
        }
```

- [ ] **Step 4: Handle instantiation.** In `infer_handle` (~1000-1025), instantiate the handled effect's params **once** (`τ̄_h`), seed the body's ambient with `Name(τ̄_h)`, and type each clause against that instantiation. Compute a per-effect instantiation shared by the seed and every clause of that effect:

```rust
        let effect = self.handler_effect(handler);
        let amb_in = self.fresh_row();
        // One instantiation of the handled effect's type params, shared by the
        // seeded ambient and every clause -> the body's performs unify against it.
        let eff_args: Vec<Ty> = match &effect {
            Some(e) => self
                .ops
                .values()
                .find(|o| &o.effect == e)
                .map(|o| o.effect_params.iter().map(|_| self.fresh()).collect())
                .unwrap_or_default(),
            None => Vec::new(),
        };
        if let Some(e) = &effect {
            let _ = self.add_effect(amb_in, e, eff_args.clone(), span);
        }
        let body_ty = self.infer_expr(body, env, amb_in);
```

and in the clause loop, substitute the effect's params with `eff_args` when reading `op.params`/`op.ret`:

```rust
            let (params, b) = match self.ops.get(&clause.op).cloned() {
                Some(op) => {
                    let m: HashMap<u32, Ty> = op.effect_params.iter().cloned()
                        .zip(eff_args.iter().cloned()).collect();
                    (
                        op.params.iter().map(|t| subst_vars(t, &m, &HashMap::new())).collect::<Vec<_>>(),
                        subst_vars(&op.ret, &m, &HashMap::new()),
                    )
                }
                None => (Vec::new(), Ty::Error),
            };
```

(Adjust to the existing local names; the invariant is: seed, clauses, and body performs of the handled effect all share `eff_args`.)

- [ ] **Step 5: Run — expect PASS.**

Run: `cargo test --test generic_effects one_generic_state_used_at_int_and_string 2>&1 | grep -E "test result|FAILED|left|right" | head`
Expected: PASS with output `ab`.

- [ ] **Step 6: Regression — full suite (monomorphic path still green).**

Run: `cargo test 2>&1 | grep -E "test result: FAILED|FAILED" | head || echo GREEN`
Expected: all green.

- [ ] **Step 7: Commit + push.**

```bash
cargo fmt --all && sh scripts/check.sh
git add src/types.rs tests/generic_effects.rs
git commit -m "$(printf 'feat(effects): generic operations perform and handle at any type\n\nA perform instantiates the effect type params (fresh, shared across a\ncomputation via the row); a handle fixes one instantiation for its scope,\nseeding the body ambient and typing clauses against it. One effect State(s)\nnow runs at Int AND String in one program -> ab. Monomorphic path unchanged.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

---

## Task 4: Soundness negative + inferred-row pins + the exit gate

**Files:**
- Modify: `tests/generic_effects.rs` (cross-instantiation `E0423`), `tests/effect_types.rs` (inferred-row scheme pins)

**Interfaces:**
- Consumes: `run`/`check_err` (Task 2/3); `schemes` (existing in `tests/effect_types.rs`).

- [ ] **Step 1: The soundness negative (the point of design B).** In `tests/generic_effects.rs`, add a `check_err` helper (if not present) and:

```rust
#[test]
fn same_effect_at_two_types_in_one_scope_is_e0423() {
    // `fs` performs State(String), `fi` performs State(Int); composed in one
    // computation the row carries State at two types -> E0423. A name-only row
    // would silently accept this -- the test proves the type arg rides in the row.
    let src = "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
               fn fs() { set(get() <> \"x\") }\n\
               fn fi() { set(get() + 1) }\n\
               pub fn main() {\n\
                 let _ = handle { let _ = fs()  fi() } with {\n\
                   State.get() -> fn(st) { (resume(st))(st) }\n\
                   State.set(v) -> fn(st) { (resume(Unit))(v) }\n\
                   return(x) -> fn(st) { x }\n\
                 }\n\
                 io.println(\"x\")\n\
               }\n";
    let err = check_source("t.elya", src).expect_err("conflicting instantiations must be rejected");
    assert!(err.contains("E0423"), "must be a row mismatch on conflicting State args: {err}");
}
```

- [ ] **Step 2: Inferred-row scheme pins.** In `tests/effect_types.rs` (using `schemes`), pin that a concrete performer names its argument in the row and a relay keeps it polymorphic:

```rust
#[test]
fn concrete_state_use_names_the_arg_in_the_row() {
    // `f` uses State at String concretely -> the row carries State(String).
    let src = "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
               fn f() { set(get() <> \"x\") }\n";
    let (s, d) = schemes(src);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(s["f"], "fn() / {State(String)} -> Unit");
}
```

(If the printed form differs only in spacing, pin the actual — the invariant is the row names `State(String)`, proving the argument is carried. Confirm a *polymorphic* relay over a generic effect keeps the arg as a quantified variable; add that pin if the exact string is stable.)

- [ ] **Step 3: Run the new tests.**

Run: `cargo test --test generic_effects same_effect_at_two_types --test effect_types concrete_state_use_names 2>&1 | grep -E "test result|FAILED|left|right" | head`
Expected: PASS. If `concrete_state_use_names_the_arg_in_the_row` fails, read the actual printed row and pin it (invariant: it names `State(String)`); if the row does **not** name the argument, the arg is not riding in the row — a real gap, investigate (do not weaken the assertion).

- [ ] **Step 4: The Slice-4c-2 exit gate.**

```bash
cargo fmt --all
sh scripts/check.sh
```
Expected: exit 0; full suite green. Confirm these rows pass: `unify_row_reconciles_matching_effect_args`, `unify_row_rejects_conflicting_effect_args_e0423`, `effect_decl_with_type_param_parses`, `generic_effect_declaration_is_accepted`, `one_generic_state_used_at_int_and_string`, `same_effect_at_two_types_in_one_scope_is_e0423`, `concrete_state_use_names_the_arg_in_the_row` — and every prior monomorphic effect/TCE/snapshot test **unchanged**.

- [ ] **Step 5: Commit + push.**

```bash
git add tests/generic_effects.rs tests/effect_types.rs
git commit -m "$(printf 'test(effects): generic-effect soundness negative + inferred-row pins; Slice-4c-2 exit gate\n\nTwo functions performing State at different concrete types in one scope is E0423\n(the proof the type arg rides in the row, per the soundness rationale); a\nconcrete performer infers {State(String)}. Slice 4c-2 complete -- generic\neffects sound and compositional, monomorphic path unchanged. Closes the\ngeneric-effects/State arc.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

### Exit criterion

One `effect State(s)` runs at `Int` and `String` in one program (`ab`); the same effect at conflicting types in one scope is `E0423`; a concrete performer's row names its argument (`{State(String)}`); the entire monomorphic path (effects, TCE, snapshots) is green and **unchanged**; `cargo fmt --all` + `sh scripts/check.sh` clean. The generic-effects/State arc closes.

---

## Self-Review

- **Spec coverage:** §2 rationale → the design (args in the row). §3 representation + fan-out → Task 1 Steps 1-4. §4 `unify_row` arm → Task 1 Step 5 + unit tests Step 6. §5 perform/handle instantiation → Task 3. §6 parser + param-env elaboration → Task 2. §7 monomorphic arity-0 preserved → the Task 1 Step 7 strict gate. §8 E0423 reused → Task 1 Step 5 + Task 4 negative. §9 pipeline (no `eval` change) → honored. §10 tests → all tasks, output/scheme-verified. §11 build order → the 4-task sequence (representation-first, arity-0-gated).
- **The load-bearing gate is explicit and strict:** Task 1 Step 7 forbids editing any existing test and requires snapshots unchanged; a failure means fix the code, not the test.
- **Compiler-driven fan-out is concrete, not a placeholder:** Step 2 enumerates every site with its transformation; the struct change makes the compiler list them, exactly how a representation change is done in Rust (the `Expr::Lambda` precedent).
- **Type/name consistency:** `EffectLabel { args, span }`, `add_effect(.., args, ..)`, `OpInfo.effect_params`, `instantiate_op`, `elaborate_op_ty` are introduced once and reused across tasks; `run`/`check_err` in `tests/generic_effects.rs`, `schemes` in `tests/effect_types.rs`.
- **Runtime untouched:** no `eval.rs` change — effects are erased; type args are static. Called out in constraints and §9.
- **Measure-then-pin honored:** the one printed-string pin (`{State(String)}`) carries a pin-the-actual instruction with the invariant (row names the argument), and a stop-condition if the row does *not* name it (a real gap, not a test to soften).
