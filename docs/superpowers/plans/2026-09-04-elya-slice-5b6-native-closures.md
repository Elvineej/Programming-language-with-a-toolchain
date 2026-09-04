# Slice 5b-6 — Native Closures (arc node N5) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Compile Elya lambdas to native code — heap-allocated closures that capture their free variables, are called through a loaded code pointer under a guaranteed tail call, and are traced and reclaimed by the existing mark-sweep collector with no new dispatch path in `gc_mark`.

**Architecture:** A closure is one flat heap block `[tag][code_ptr][cap_0..]`, allocated by the same `elya_alloc` that allocates ADTs and described by one *synthetic constructor descriptor row* per lambda site — so the collector traces a closure with byte-identical code to how it traces a `Cons`. Each lambda is lifted to a top-level LLVM function whose parameter 0 is the closure pointer itself (the environment *is* the closure), leaving the tail-call boundary with nothing extra to root. Free-variable analysis and lambda-site collection live in a new LLVM-free module (`crates/codegen/src/closure.rs`) so they are unit-testable without a `Context`.

**Tech Stack:** Rust 2021 (workspace `elya` / `elya-codegen` / `elya-cli`), inkwell 0.5 over LLVM 18.1.6, `clang` as link driver and as the C driver for `crates/codegen/src/runtime.c`, target `x86_64-pc-windows-msvc`.

**Spec:** `docs/superpowers/specs/2026-09-03-elya-slice-5b6-native-closures-design.md` (committed at `c6342fc`)

## Global Constraints

- **Crate discipline.** `elya` (repo root `src/`) must stay LLVM-free — configuration A of the gate builds it with no `llvm_sys` anywhere in the graph. All LLVM lives in `crates/codegen`.
- **The gate is five stages, run as one command:** `powershell -NoProfile -File scripts/check.ps1` — `cargo fmt --all -- --check` → clippy A (`-p elya -p elya-cli`) → test A → clippy B (`--workspace --features elya-cli/codegen`) → test B. `pwsh` is not installed here; use `powershell`.
- **Run `cargo fmt --all` in *write* mode before every gate run.** Stage 1 is a `--check` and fails hard on a single stray space.
- **`CARGO_INCREMENTAL=0` for every cargo invocation.** The incremental cache hangs on this machine.
- **Proof is execution.** No `insta` snapshots of LLVM IR, no `#[ignore]`, no test that asserts on the shape of emitted IR. Every guarantee this slice claims gets a built-and-run negative control: a program that produces the wrong answer, or dies, on the un-fixed build.
- **Never edit a test or an expected value to make something pass**, and **never nudge a constant** (`GC_MAX_WORDS`, `GC_THRESHOLD_WORDS`, `MAX_PARAMS`, `MAX_LAMBDA_PARAMS`) to make a test pass. Constants are measured, then pinned. If a test wants a different constant, the test is wrong or the measurement is.
- **Elya surface syntax, as the corpus actually spells it** (`crates/codegen/tests/native_codegen.rs:587,594,600,644,778,832`): match arms and block statements are **whitespace/newline separated, never comma separated** (`match xs { Nil -> 0  Cons(h, t) -> h }`); constructor lists inside a `type` declaration **are** comma separated (`type L { Nil, Cons(Int, L) }`); boolean literals are `True` / `False` (lowercase dies at E0200 in the front end).
- **The arity cap is a measurement, not a convention.** Verbatim, and repeated at every site that states the cap:

  > **C-ii probe**, `scratchpad/n5-indirect-probe`, LLVM 18.1.6 / `x86_64-pc-windows-msvc`, 2026-09-03. Sweeping caller arity C × callee arity K over 1..8 for a `musttail` call under `tailcc`, the **indirect-callee matrix is cell-for-cell identical to the direct-callee matrix**: K ≤ 5 compiles for every C; K ∈ {6,7} compiles only when C ≥ 6; K = 8 only when C ≥ 8. Every passing cell emits a real tail jump (`jmpq *%rax` for the indirect form) — no silent degradation to a call. Every failing cell is `LLVM ERROR: Can't handle guaranteed tail call under win64 yet`, a backend abort with no recoverable diagnostic. Nothing is claimed outside 1..8. Since a lambda of P source parameters converts to a lifted function of K = P + 1, the only P safe for **every** caller is **P ≤ 4**.

  Honest caveat to carry with it: `@elya_main` has arity 0, outside the swept 1..8 range. That cell rests on the shipped 5b-3/5b-4 corpus, which already `musttail`s from arity-0 `@elya_main` into arity-1..3 callees and runs.
- **Git.** Approving this plan authorizes its per-task commits and `git add <explicit paths>` (never `git add -A` / `git add .`) and fast-forward pushes to the existing upstream. It authorizes nothing else — no amend, rebase, reset, checkout, stash, or force-push. Checkpoint pauses are review points and are **not** waived by that authorization.
- **Commit trailer:** `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`.
- **Scratch work goes in the scratchpad**, never under a tracked path.
- After the last code change, run `graphify update .` (AST-only, no API cost).

---

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `src/types.rs` (modify, `:982-1013`) | Record a type at each **lambda parameter span** — the structural discharge of obligation T3 | 1 |
| `src/core.rs` (modify, `:40`, `:296-300`, `:424-434`) | `CoreKind::Lambda` carries `Rc<[CoreParam]>`; lowering reads the recorded types; `pretty_expr` renders `param.name` | 1 |
| `tests/core_lowering.rs` (modify) | Front-end proof that lambda parameter types reach Core | 1 |
| `crates/codegen/src/closure.rs` (**create**) | Free-variable analysis + lambda-site table. LLVM-free, unit-testable without a `Context` | 2 |
| `crates/codegen/src/lib.rs` (modify) | `mod closure;`, `repr_ty` widened, closure allocation, lifting, indirect call path, the 4-parameter cap, the synthetic descriptor rows, `is_heap_ty` | 3, 4 |
| `crates/codegen/tests/native_codegen.rs` (modify) | CR-3's two execution tests; the `live=` settling test | 4, 5 |
| `crates/codegen/src/runtime.c` (modify, `:154-178`, `:230-239`) | `gc_live` accumulator on the sweep's marked branch; `live=` in the report line | 5 |
| `README.md` (modify, `:36-52`) | The backend-coverage paragraph | 6 |

`gc_mark` (`runtime.c:134-152`) is deliberately **not** in this table. It must come out of this slice byte-identical; that is the whole point of the synthetic-descriptor-row design.

---

## Task 1: Record lambda parameter types (obligation T3, discharged structurally)

The back end must know each lambda parameter's type to build the lifted function's signature. Obligation T3 has stood since 5b-3 as "the back end reconstructs parameter types from context." Route (a) of CR-1 closes it at the source instead: the checker already creates a fresh type variable per lambda parameter — it simply never records it. Record it, and reshape `CoreKind::Lambda` to carry `CoreParam` exactly as `CoreFn` already does.

**Files:**
- Modify: `src/types.rs:982-1013` (the `Expr::Lambda` arm of `infer_expr_inner`)
- Modify: `src/core.rs:40` (the `Lambda` variant), `src/core.rs:296-300` (the lowering arm), `src/core.rs:424-434` (`pretty_expr`)
- Modify: `crates/codegen/src/lib.rs:1363-1374` (`rejects_lambda_specifically` builds a `CoreKind::Lambda` by hand)
- Test: `tests/core_lowering.rs`

`tests/core_lowering.rs:48` and `:213` also match on `CoreKind::Lambda`, but both bind the parameter list as `_` — they compile unchanged.

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `CoreKind::Lambda(Rc<[CoreParam]>, Rc<CoreExpr>)` — was `Rc<[String]>`. `CoreParam { pub name: String, pub ty: Ty }` already exists at `src/core.rs:78-81`.
  - The invariant every later task relies on: for every lambda in a well-typed module, `table[param.span]` exists, so `lower_module` never returns `LowerError::Untyped` for a lambda parameter.

**Ordering note for the implementer — the two edits are one task on purpose.** `tests/core_lowering.rs:113` and `:210` already lower a program containing a lambda (`"fn demo() { let f = fn(x) { x } ..."`). Reshaping `CoreKind::Lambda` *without* the `types.rs` insert makes `lower_module` return `Err(LowerError::Untyped(p.span))` on that existing program and the existing corpus tests panic. Do not split this task.

- [ ] **Step 1: Write the failing test**

Append to `tests/core_lowering.rs`:

```rust
/// Slice 5b-6 Task 1 (obligation T3, CR-1 route (a)). A lambda parameter's type
/// is RECORDED at its span by the checker and carried into Core, rather than
/// reconstructed in the back end from call-site context. The lambda here is
/// applied to an `Int` at a monomorphic use, so the recorded type zonks to a
/// concrete `Int` — which is exactly what the back end needs to build the lifted
/// function's signature.
#[test]
fn lambda_parameters_carry_recorded_types() {
    let (core, _table) = lower_src("pub fn main() {\n  let f = fn(x) { x + 1 }\n  f(41)\n}\n");
    let lam = nodes(&core)
        .into_iter()
        .find(|n| matches!(&n.kind, CoreKind::Lambda(..)))
        .expect("the program contains a lambda");
    let CoreKind::Lambda(params, _) = &lam.kind else {
        unreachable!("just matched")
    };
    assert_eq!(params.len(), 1, "one parameter");
    assert_eq!(params[0].name, "x", "the name survives lowering");
    assert_eq!(
        params[0].ty,
        Ty::Base(TyCon::Int),
        "the TYPE survives lowering — this is what T3 was about"
    );
}
```

`lower_src` here returns a tuple `(CoreModule, BTreeMap<Span, Ty>)` (`tests/core_lowering.rs:22-30`) — destructure it. `nodes(&core)` is the pre-order walker already in that file. `Ty` derives `PartialEq` (`src/types.rs:20`), so `assert_eq!` on a `Ty` compiles.

- [ ] **Step 2: Run it and watch it fail to compile**

```sh
CARGO_INCREMENTAL=0 cargo test -p elya --test core_lowering lambda_parameters_carry_recorded_types
```

Expected: **compile error**, `error[E0609]: no field 'name' on type '&String'` (and the same for `ty`) — because `CoreKind::Lambda` still carries `Rc<[String]>`. A compile failure is the correct falsification here: the test asserts on structure that does not yet exist.

- [ ] **Step 3: Record the type at the parameter span**

In `src/types.rs`, in the `Expr::Lambda` arm of `infer_expr_inner`, insert one line between `env.insert(...)` and `param_tys.push(pv);`:

```rust
            Expr::Lambda { params, body } => {
                // Each parameter gets a fresh monomorphic type variable.
                env.push();
                let mut param_tys = Vec::with_capacity(params.len());
                for p in params {
                    let pv = self.fresh();
                    env.insert(
                        &p.node.name,
                        Scheme {
                            vars: Vec::new(),
                            row_vars: Vec::new(),
                            ty: pv.clone(),
                        },
                    );
                    // 5b-6 §4 (obligation T3): record the parameter's type at its
                    // own span, exactly as a top-level fn parameter is recorded.
                    // The back end then READS a lambda parameter's type instead of
                    // reconstructing it from call-site context.
                    self.node_types.insert(p.span, pv.clone());
                    param_tys.push(pv);
                }
```

The receiver is `self`, not `inf` — this arm is inside a `&mut self` method. Leave the rest of the arm (the fresh ambient row, `close_unrelayed_residual`, the returned `Ty::Fn`) untouched.

- [ ] **Step 4: Reshape the Core variant**

In `src/core.rs`, at the `Lambda` variant:

```rust
    /// uncurried params, each carrying the type recorded at its span (5b-6 §4,
    /// obligation T3); the block body is flattened into a single expression.
    Lambda(Rc<[CoreParam]>, Rc<CoreExpr>),
```

- [ ] **Step 5: Read the recorded types in the lowering arm**

In `src/core.rs`, replace the `Expr::Lambda` arm of the expression lowerer:

```rust
        Expr::Lambda { params, body } => {
            // Same shape as the top-level fn parameter loop above: the type comes
            // out of the frozen table keyed by the parameter's own span. A lambda
            // parameter missing from the table is `Untyped`, not a guess.
            let mut ps = Vec::with_capacity(params.len());
            for p in params {
                let ty = table
                    .get(&p.span)
                    .cloned()
                    .ok_or(LowerError::Untyped(p.span))?;
                ps.push(CoreParam {
                    name: p.node.name.clone(),
                    ty,
                });
            }
            let b = lower_block(&body.node, table, ctors)?;
            CoreKind::Lambda(ps.into(), Rc::new(b))
        }
```

- [ ] **Step 6: Keep `pretty_expr`'s rendering byte-identical**

In `src/core.rs`'s `pretty_expr`, the `Lambda` arm changes exactly one line — `s.push_str(param)` becomes `s.push_str(&param.name)` — so the existing `insta` corpus snapshots of pretty-printed Core render unchanged and need no acceptance:

```rust
        CoreKind::Lambda(params, body) => {
            s.push_str("(fn (");
            for (i, param) in params.iter().enumerate() {
                if i > 0 {
                    s.push(' ');
                }
                s.push_str(&param.name);
            }
            s.push_str(") ");
            pretty_expr(body, p, s);
        }
```

- [ ] **Step 7: Fix the one hand-built `CoreKind::Lambda` in the back end**

In `crates/codegen/src/lib.rs`, `rejects_lambda_specifically` constructs a `Lambda` by hand. Keep the test's meaning (Task 3 replaces it wholesale); make it compile:

```rust
            kind: CoreKind::Lambda(
                Rc::from([CoreParam {
                    name: "x".to_string(),
                    ty: Ty::Base(TyCon::Int),
                }]),
                Rc::new(int_lit(1)),
            ),
```

Add `CoreParam` to the `elya::core::{...}` import list in that file if it is not already there.

- [ ] **Step 8: Run the new test and the existing corpus**

```sh
CARGO_INCREMENTAL=0 cargo test -p elya --test core_lowering
```

Expected: PASS, including `lambda_parameters_carry_recorded_types` **and** the pre-existing tests at `:113` / `:210` that lower a lambda program (they are what would break if Step 3 were skipped).

- [ ] **Step 9: Run the full gate**

```sh
cargo fmt --all
powershell -NoProfile -File scripts/check.ps1
```

Expected: all five stages green.

- [ ] **Step 10: Commit**

```bash
git add src/types.rs src/core.rs tests/core_lowering.rs crates/codegen/src/lib.rs
git commit -m "feat(types,core): record lambda parameter types (5b-6 Task 1, obligation T3)"
```

---

## Task 2: Free variables and the lambda-site table

Closure conversion's analysis half, in its own module, with no `inkwell` import. Everything here is a pure function over Core, so every claim in it is provable by a unit test that never touches LLVM.

**Files:**
- Create: `crates/codegen/src/closure.rs`
- Modify: `crates/codegen/src/lib.rs` (add `mod closure;` next to the other module declarations)
- Test: `crates/codegen/src/closure.rs` (a `#[cfg(test)] mod tests` at the bottom, same house style as `lib.rs`)

**Interfaces:**
- Consumes: `CoreKind::Lambda(Rc<[CoreParam]>, Rc<CoreExpr>)` from Task 1.
- Produces:
  - `pub fn free_vars_under(e: &CoreExpr, bound: &[String]) -> BTreeMap<String, Ty>`
  - `pub fn free_vars(e: &CoreExpr) -> BTreeMap<String, Ty>`
  - `pub struct LambdaSite { pub key: usize, pub symbol: String, pub tag: usize, pub captures: Vec<(String, Ty)>, pub params: Rc<[CoreParam]>, pub body: Rc<CoreExpr>, pub ret: Ty }`
  - `pub fn collect_lambdas(core: &CoreModule, first_tag: usize) -> Vec<LambdaSite>`
  - Guaranteed by construction and asserted by test: `sites[i].tag == first_tag + i`, and `sites` is in one fixed pre-order.

**Two design points to carry, both refinements of the spec worth writing down:**

1. **`free_vars` returns `BTreeMap<String, Ty>`, not the spec's `BTreeSet<String>`.** A capture needs its type (to widen a `Bool` into a word, to pick the mask bit, to load it back). `CorePat::Ctor(String, Rc<[CorePat]>)` binders carry no types, so a type environment cannot be threaded through a pure Core walk. The type instead comes from the free `CoreKind::Var(x)` occurrence's own `CoreExpr.ty` — first occurrence wins, and all occurrences of a monomorphic local agree. `BTreeMap` also fixes capture order = name order, deterministically.
2. **A global is "free in the enclosing function too", not "found in a symbol table".** The capture set is `free_vars(lambda) − free_vars_under(enclosing_fn_body, enclosing_fn_params)`. A name free in the whole function body is module-level; a name bound by an enclosing `let`, parameter, or pattern is a capture. This is purely structural, needs no module symbol table, and gets `let churn = fn(x){...}` shadowing a top-level `churn` right, where a by-name filter would silently drop the capture and then silently call the global.

- [ ] **Step 1: Write the failing tests**

Create `crates/codegen/src/closure.rs` with **only** the test module for now (the implementation lands in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use elya::core::{CoreArm, CoreFn, CoreLit, CoreType};
    use elya::span::Span;
    use elya::types::TyCon;

    fn e(ty: Ty, kind: CoreKind) -> CoreExpr {
        CoreExpr {
            span: Span::EMPTY,
            ty,
            kind,
        }
    }
    fn int() -> Ty {
        Ty::Base(TyCon::Int)
    }
    fn var(name: &str) -> CoreExpr {
        e(int(), CoreKind::Var(name.to_string()))
    }
    fn lit(n: i64) -> CoreExpr {
        e(int(), CoreKind::Lit(CoreLit::Int(n)))
    }
    fn param(name: &str) -> CoreParam {
        CoreParam {
            name: name.to_string(),
            ty: int(),
        }
    }

    /// `let x = a  x` — `a` is free (it is in the VALUE, outside the binding),
    /// `x` is not (it is in the BODY, inside it). Getting this asymmetry backwards
    /// is the classic capture bug: it silently under-captures `a`.
    #[test]
    fn let_binds_its_body_but_not_its_value() {
        let expr = e(
            int(),
            CoreKind::Let("x".to_string(), Rc::new(var("a")), Rc::new(var("x"))),
        );
        let fv = free_vars(&expr);
        assert!(fv.contains_key("a"), "the let VALUE is outside the binding");
        assert!(!fv.contains_key("x"), "the let BODY is inside the binding");
    }

    /// An inner lambda's parameter shadows an outer free name of the same spelling.
    #[test]
    fn inner_lambda_parameter_shadows() {
        let inner = e(
            Ty::Fn(vec![int()], EffectRow::pure(), Box::new(int())),
            CoreKind::Lambda(Rc::from([param("k")]), Rc::new(var("k"))),
        );
        assert!(
            free_vars(&inner).is_empty(),
            "`k` is bound by the lambda's own parameter"
        );
    }

    /// Match-arm binders are binders. `Cons(h, t) -> h` must not report `h` free.
    #[test]
    fn match_arm_binders_bind() {
        let arm = CoreArm {
            pat: CorePat::Ctor(
                "Cons".to_string(),
                Rc::from([CorePat::Var("h".to_string()), CorePat::Wild]),
            ),
            body: var("h"),
        };
        let expr = e(
            int(),
            CoreKind::Match(Rc::new(var("xs")), Rc::from([arm])),
        );
        let fv = free_vars(&expr);
        assert!(fv.contains_key("xs"), "the scrutinee is outside the arm");
        assert!(!fv.contains_key("h"), "a pattern binder binds in its arm");
    }

    /// A free `Var` carries the type of its own occurrence — that is where a
    /// capture's type comes from, since pattern binders carry none.
    #[test]
    fn a_free_var_carries_its_own_type() {
        let fv = free_vars(&var("a"));
        assert_eq!(fv.get("a"), Some(&int()));
    }

    /// `fn wrap(k) { fn(x) { x + k } }` — `k` is captured; a module-level name
    /// used inside the lambda is NOT, because it is free in the enclosing function
    /// too. Tags are assigned from `first_tag` in the collection order.
    #[test]
    fn collect_captures_locals_and_excludes_globals() {
        let lam = e(
            Ty::Fn(vec![int()], EffectRow::pure(), Box::new(int())),
            CoreKind::Lambda(
                Rc::from([param("x")]),
                Rc::new(e(
                    int(),
                    CoreKind::App(
                        Rc::new(var("helper")),
                        Rc::from([var("x"), var("k")]),
                    ),
                )),
            ),
        );
        let core = CoreModule {
            fns: vec![CoreFn {
                name: "wrap".to_string(),
                params: Rc::from([param("k")]),
                body: lam,
            }],
            types: Vec::<CoreType>::new(),
        };
        let sites = collect_lambdas(&core, 7);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].symbol, "wrap.lambda.0");
        assert_eq!(sites[0].tag, 7, "tags start at `first_tag`");
        let names: Vec<&str> = sites[0].captures.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec!["k"],
            "`k` is a parameter of the enclosing fn (a capture); \
             `helper` is free in the enclosing fn too (a global)"
        );
    }

    /// Two sibling lambdas: order is fixed pre-order, symbols number within the
    /// enclosing function, tags number consecutively from `first_tag`. Task 3
    /// hard-checks `tag == first_tag + i` when it appends descriptor rows, so
    /// this is load-bearing, not cosmetic.
    #[test]
    fn sites_are_ordered_and_tags_are_consecutive() {
        let mk = |p: &str| {
            e(
                Ty::Fn(vec![int()], EffectRow::pure(), Box::new(int())),
                CoreKind::Lambda(Rc::from([param(p)]), Rc::new(lit(0))),
            )
        };
        let body = e(
            int(),
            CoreKind::Let(
                "a".to_string(),
                Rc::new(mk("u")),
                Rc::new(e(
                    int(),
                    CoreKind::Let("b".to_string(), Rc::new(mk("v")), Rc::new(lit(1))),
                )),
            ),
        );
        let core = CoreModule {
            fns: vec![CoreFn {
                name: "two".to_string(),
                params: Rc::from([]),
                body,
            }],
            types: Vec::<CoreType>::new(),
        };
        let sites = collect_lambdas(&core, 0);
        let syms: Vec<&str> = sites.iter().map(|s| s.symbol.as_str()).collect();
        assert_eq!(syms, vec!["two.lambda.0", "two.lambda.1"]);
        for (i, s) in sites.iter().enumerate() {
            assert_eq!(s.tag, i, "tag {i} must be `first_tag + i`");
        }
    }
}
```

- [ ] **Step 2: Run them and watch them fail to compile**

Add `mod closure;` to `crates/codegen/src/lib.rs`, next to the existing module declarations, then:

```sh
CARGO_INCREMENTAL=0 cargo test -p elya-codegen --lib closure::
```

Expected: **compile error** — `cannot find function 'free_vars' in this scope`, `cannot find function 'collect_lambdas' in this scope`, `cannot find struct 'LambdaSite'`. Nothing is implemented yet.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/codegen/src/closure.rs`, above the test module:

```rust
//! Slice 5b-6 §6 — closure conversion's ANALYSIS half: free variables and the
//! lambda-site table.
//!
//! Deliberately LLVM-free. Every function here is a pure fold over Core, so the
//! capture rules — which the collector's correctness rests on — are provable by
//! unit tests that never construct an LLVM `Context`. The emission half lives in
//! `lib.rs`, which reads the table this module builds.

use std::collections::BTreeMap;
use std::rc::Rc;

use elya::core::{CoreExpr, CoreKind, CoreModule, CoreParam, CorePat};
use elya::types::{EffectRow, Ty};

/// One lambda in the module, in a fixed pre-order.
pub struct LambdaSite {
    /// Identity: the ADDRESS of the `CoreExpr` node this site was built from.
    ///
    /// Span is forbidden as a key (`core.rs:20`: provenance, never a type key),
    /// and a pre-order index would rest on an unchecked walk-order invariant
    /// shared between two separate walks. The address is checkable and exact.
    /// It is stable because `collect_lambdas` and the emitter both run under the
    /// SAME immutable `&CoreModule` borrow, so nothing can move or reallocate a
    /// node in between; and every child node is behind an `Rc`, so cloning the
    /// handles below preserves addresses exactly.
    pub key: usize,
    /// The lifted function's Elya-level name, `<enclosing_fn>.lambda.<n>`, later
    /// mangled to `elya_<enclosing_fn>.lambda.<n>`. `.` is legal in LLVM
    /// identifiers and in COFF symbols, and it is Elya's access operator, so no
    /// source identifier can collide with one of these.
    pub symbol: String,
    /// The SYNTHETIC constructor tag for this closure's descriptor row. Assigned
    /// in the same pre-order walk as `symbol`, so the symbol and its descriptor
    /// row cannot drift apart.
    pub tag: usize,
    /// Captures in NAME order (`BTreeMap` iteration order), which is therefore
    /// the slot order in the heap block and the bit order in the pointer mask.
    pub captures: Vec<(String, Ty)>,
    pub params: Rc<[CoreParam]>,
    pub body: Rc<CoreExpr>,
    /// The lifted function's return type — the body's own type, never a second
    /// source of truth (the same rule `CoreFn` states at `core.rs:87`).
    pub ret: Ty,
}

/// Free variables of `e`, each mapped to the type carried by its first free
/// occurrence.
///
/// Types come from occurrences because pattern binders carry none: `CorePat::Ctor`
/// holds only sub-patterns, so no type environment can be threaded through a pure
/// Core walk. All occurrences of a monomorphic local agree, so "first occurrence
/// wins" is not a choice between different answers.
pub fn free_vars(e: &CoreExpr) -> BTreeMap<String, Ty> {
    free_vars_under(e, &[])
}

/// `free_vars`, with an initial set of already-bound names — used to bind the
/// enclosing function's parameters when computing which names are module-level.
pub fn free_vars_under(e: &CoreExpr, bound: &[String]) -> BTreeMap<String, Ty> {
    let mut scope: Vec<String> = bound.to_vec();
    let mut out = BTreeMap::new();
    fv_walk(e, &mut scope, &mut out);
    out
}

/// The match is EXHAUSTIVE with no catch-all, on purpose. Under-capture is a
/// wrong answer that the collector turns into a use-after-free; over-capture is
/// benign retention. So a new `CoreKind` variant must fail the build here rather
/// than fall into a `_ => {}` and go silently uncaptured.
fn fv_walk(e: &CoreExpr, scope: &mut Vec<String>, out: &mut BTreeMap<String, Ty>) {
    match &e.kind {
        CoreKind::Lit(_) => {}
        CoreKind::Var(x) => {
            if !scope.iter().any(|b| b == x) {
                out.entry(x.clone()).or_insert_with(|| e.ty.clone());
            }
        }
        CoreKind::App(f, args) => {
            fv_walk(f, scope, out);
            for a in args.iter() {
                fv_walk(a, scope, out);
            }
        }
        CoreKind::Ctor(_, fields) => {
            for f in fields.iter() {
                fv_walk(f, scope, out);
            }
        }
        CoreKind::Prim(_, args) => {
            for a in args.iter() {
                fv_walk(a, scope, out);
            }
        }
        CoreKind::Lambda(params, body) => {
            let depth = scope.len();
            for p in params.iter() {
                scope.push(p.name.clone());
            }
            fv_walk(body, scope, out);
            scope.truncate(depth);
        }
        CoreKind::Let(name, value, body) => {
            // The VALUE is outside the binding; the BODY is inside it.
            fv_walk(value, scope, out);
            let depth = scope.len();
            scope.push(name.clone());
            fv_walk(body, scope, out);
            scope.truncate(depth);
        }
        CoreKind::If(c, t, f) => {
            fv_walk(c, scope, out);
            fv_walk(t, scope, out);
            fv_walk(f, scope, out);
        }
        CoreKind::Match(scrutinee, arms) => {
            fv_walk(scrutinee, scope, out);
            for arm in arms.iter() {
                let depth = scope.len();
                pat_binders(&arm.pat, scope);
                fv_walk(&arm.body, scope, out);
                scope.truncate(depth);
            }
        }
    }
}

fn pat_binders(p: &CorePat, scope: &mut Vec<String>) {
    match p {
        CorePat::Wild | CorePat::Lit(_) => {}
        CorePat::Var(x) => scope.push(x.clone()),
        CorePat::Ctor(_, subs) => {
            for s in subs.iter() {
                pat_binders(s, scope);
            }
        }
    }
}

/// Every lambda in the module, in one fixed pre-order, with tags running
/// consecutively from `first_tag` (which the caller sets to the number of real
/// constructor descriptor rows, so closure tags continue the same numbering).
pub fn collect_lambdas(core: &CoreModule, first_tag: usize) -> Vec<LambdaSite> {
    let mut out = Vec::new();
    for f in &core.fns {
        // A name free in the WHOLE enclosing function body (with that function's
        // parameters bound) is module-level, so it is not a capture. A name bound
        // by an enclosing `let`, parameter, or pattern IS. This is structural: it
        // needs no symbol table, and it gets a local that shadows a top-level name
        // right, where a by-name filter would drop the capture and then silently
        // call the global instead.
        let param_names: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
        let module_level = free_vars_under(&f.body, &param_names);
        let mut n = 0usize;
        collect_in(&f.body, &f.name, &mut n, &module_level, first_tag, &mut out);
    }
    out
}

fn collect_in(
    e: &CoreExpr,
    enclosing: &str,
    n: &mut usize,
    module_level: &BTreeMap<String, Ty>,
    first_tag: usize,
    out: &mut Vec<LambdaSite>,
) {
    match &e.kind {
        CoreKind::Lambda(params, body) => {
            let captures: Vec<(String, Ty)> = free_vars(e)
                .into_iter()
                .filter(|(name, _)| !module_level.contains_key(name))
                .collect();
            let symbol = format!("{enclosing}.lambda.{n}");
            *n += 1;
            out.push(LambdaSite {
                key: e as *const CoreExpr as usize,
                symbol,
                tag: first_tag + out.len(),
                captures,
                params: Rc::clone(params),
                body: Rc::clone(body),
                ret: body.ty.clone(),
            });
            collect_in(body, enclosing, n, module_level, first_tag, out);
        }
        CoreKind::Lit(_) | CoreKind::Var(_) => {}
        CoreKind::App(f, args) => {
            collect_in(f, enclosing, n, module_level, first_tag, out);
            for a in args.iter() {
                collect_in(a, enclosing, n, module_level, first_tag, out);
            }
        }
        CoreKind::Ctor(_, fields) => {
            for f in fields.iter() {
                collect_in(f, enclosing, n, module_level, first_tag, out);
            }
        }
        CoreKind::Prim(_, args) => {
            for a in args.iter() {
                collect_in(a, enclosing, n, module_level, first_tag, out);
            }
        }
        CoreKind::Let(_, value, body) => {
            collect_in(value, enclosing, n, module_level, first_tag, out);
            collect_in(body, enclosing, n, module_level, first_tag, out);
        }
        CoreKind::If(c, t, f) => {
            collect_in(c, enclosing, n, module_level, first_tag, out);
            collect_in(t, enclosing, n, module_level, first_tag, out);
            collect_in(f, enclosing, n, module_level, first_tag, out);
        }
        CoreKind::Match(scrutinee, arms) => {
            collect_in(scrutinee, enclosing, n, module_level, first_tag, out);
            for arm in arms.iter() {
                collect_in(&arm.body, enclosing, n, module_level, first_tag, out);
            }
        }
    }
}
```

`EffectRow` is imported for the tests' `Ty::Fn(..)` construction; if clippy flags it as unused in the non-test build, move that import into the test module.

- [ ] **Step 4: Run the tests**

```sh
CARGO_INCREMENTAL=0 cargo test -p elya-codegen --lib closure::
```

Expected: PASS, all six.

- [ ] **Step 5: Confirm the task is inert**

```sh
CARGO_INCREMENTAL=0 cargo test --workspace --features elya-cli/codegen
```

Expected: PASS with **no change to any pre-existing test's outcome**. Nothing in `lib.rs` calls `collect_lambdas` yet; the `Lambda` arm still returns `Unsupported("Lambda")`. This step exists so that when Task 3 changes behaviour, the change is attributable.

- [ ] **Step 6: Run the full gate**

```sh
cargo fmt --all
powershell -NoProfile -File scripts/check.ps1
```

Expected: all five stages green.

- [ ] **Step 7: Commit**

```bash
git add crates/codegen/src/closure.rs crates/codegen/src/lib.rs
git commit -m "feat(codegen): free-variable analysis and the lambda-site table (5b-6 Task 2)"
```

---

## Task 3: Closure representation, lifting, the indirect call path, and the four-parameter cap

The emission half. After this task a compiled program can build a closure, call it (including in tail position), and get the right answer — **but the collector cannot yet see through one**. That is deliberate: it makes HEAD at the end of this task the genuine un-fixed build that Task 4's direction-(a) negative control is run against, with nothing stubbed.

**Files:**
- Modify: `crates/codegen/src/lib.rs` — imports; `MAX_PARAMS`'s docstring and a new `MAX_LAMBDA_PARAMS`; `repr_ty`; a new `fn_type_of`; `declare_all`; `LowerCtx`; `build_elya_call` split into `build_direct_call` + `build_closure_call`; the `CoreKind::Lambda` arm of `lower_expr` (`:550`); `build_module`'s prologue and body loop; `declare_lifted` and `emit_lifted`.
- Test: `crates/codegen/src/lib.rs` (`mod tests`) and `crates/codegen/tests/native_codegen.rs`.

**Interfaces:**
- Consumes: `closure::{collect_lambdas, LambdaSite}` from Task 2; `CoreParam`-carrying `CoreKind::Lambda` from Task 1.
- Produces:
  - `const MAX_LAMBDA_PARAMS: usize = 4;`
  - `fn fn_type_of<'ctx>(ret: BasicTypeEnum<'ctx>, params: &[BasicMetadataTypeEnum<'ctx>]) -> Result<FunctionType<'ctx>, CodegenError>`
  - `LowerCtx` gains `lambdas: &'ctx [LambdaSite]`, `lambda_index: &'ctx HashMap<usize, usize>`, `lifted: &'ctx HashMap<String, FunctionValue<'ctx>>`
  - Heap layout, relied on verbatim by Task 4's descriptor rows: word 0 = tag, word 1 = code pointer, word `i + 2` = capture `i`. Allocation size `2 + n_captures`.
  - A new refusal string: `"lambda takes more than four parameters"`.

- [ ] **Step 1: Write the failing tests**

First, in `crates/codegen/src/lib.rs`, **replace** `rejects_lambda_specifically` (it asserted the refusal this task removes) with the refusal that replaces it:

```rust
    /// 5b-6 §5.1. A lambda converts to a lifted function of arity `P + 1` (the
    /// closure is parameter 0), so the source cap is FOUR, not five.
    ///
    /// C-ii probe, `scratchpad/n5-indirect-probe`, LLVM 18.1.6 /
    /// `x86_64-pc-windows-msvc`, 2026-09-03. Sweeping caller arity C × callee
    /// arity K over 1..8 for a `musttail` call under `tailcc`, the indirect-callee
    /// matrix is cell-for-cell identical to the direct-callee matrix: K <= 5
    /// compiles for every C; K in {6,7} only when C >= 6; K = 8 only when C >= 8.
    /// Every passing cell emits a real tail jump; every failing cell is
    /// `LLVM ERROR: Can't handle guaranteed tail call under win64 yet`, a
    /// `report_fatal_error` that kills the process with no source span. Refusing
    /// at five source parameters is what keeps that unreachable.
    #[test]
    fn rejects_a_lambda_with_five_parameters() {
        let p = |n: &str| CoreParam {
            name: n.to_string(),
            ty: Ty::Base(TyCon::Int),
        };
        let e = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Fn(
                vec![Ty::Base(TyCon::Int); 5],
                EffectRow::pure(),
                Box::new(Ty::Base(TyCon::Int)),
            ),
            kind: CoreKind::Lambda(
                Rc::from([p("a"), p("b"), p("c"), p("d"), p("e")]),
                Rc::new(int_lit(1)),
            ),
        };
        let err = emit_ir(&main_fn(e)).unwrap_err();
        assert!(
            matches!(
                err,
                CodegenError::Unsupported("lambda takes more than four parameters")
            ),
            "{err:?}"
        );
    }
```

Then append two execution tests to `crates/codegen/tests/native_codegen.rs`:

```rust
/// 5b-6 §3. A closure captures a local, outlives the scope that created it (the
/// only scope-ender in Elya is a function return), and computes with the captured
/// value when called later. This is the slice's basic claim, checked by running.
#[test]
fn a_closure_captures_and_is_called_natively() {
    let src = "fn wrap(k) { fn(x) { x + k } }\npub fn main() { let f = wrap(10)  f(32) }\n";
    let dir = temp_dir("clos-basic");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "clos-basic");
    assert_eq!(run(&exe, "clos-basic"), "42");
    std::fs::remove_dir_all(&dir).ok();
}

/// 5b-6 §5.2. A closure called in TAIL position recurs to a depth that would
/// exhaust the stack under a plain call. The C-ii probe measured that `musttail`
/// through a loaded code pointer under `tailcc` emits a real indirect tail jump
/// (`jmpq *%rax`) rather than degrading silently to a call; this is that
/// measurement re-checked by execution, at 1,000,000 frames.
#[test]
fn a_closure_tail_call_recurs_in_bounded_stack() {
    let src = "fn mk() { fn(n) { if n == 0 { 7 } else { down(n - 1) } } }\n\
               fn down(n) { if n == 0 { 7 } else { down(n - 1) } }\n\
               pub fn main() { let f = mk()  f(1000000) }\n";
    let dir = temp_dir("clos-tail");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "clos-tail");
    assert_eq!(run(&exe, "clos-tail"), "7");
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 2: Run them and watch them fail**

```sh
CARGO_INCREMENTAL=0 cargo test -p elya-codegen --lib rejects_a_lambda_with_five_parameters
CARGO_INCREMENTAL=0 cargo test -p elya-codegen --test native_codegen a_closure_
```

Expected: the unit test fails with the *old* refusal, `CodegenError::Unsupported("Lambda")`, not the new one. Both execution tests fail the same way — `Unsupported("Lambda")` out of `compile_and_link`. Lambdas are rejected wholesale today.

- [ ] **Step 3: Widen `repr_ty` and factor `fn_type_of`**

In `crates/codegen/src/lib.rs`, add to the imports: `use inkwell::types::FunctionType;` and `use elya::core::CoreParam;` (if Task 1 did not already add the latter). Then:

```rust
fn repr_ty<'ctx>(ctx: &'ctx Context, ty: &Ty) -> Result<BasicTypeEnum<'ctx>, CodegenError> {
    match ty {
        Ty::Base(TyCon::Int) => Ok(ctx.i64_type().into()),
        Ty::Base(TyCon::Bool) => Ok(ctx.bool_type().into()),
        // N4 (spec §1): an ADT value is a pointer to its heap object.
        Ty::Con(..) => Ok(ctx.ptr_type(AddressSpace::default()).into()),
        // N5 (5b-6 §3): a function value is a pointer to its closure block.
        Ty::Fn(..) => Ok(ctx.ptr_type(AddressSpace::default()).into()),
        _ => Err(CodegenError::Unsupported("unrepresentable type")),
    }
}

/// Build a `FunctionType` from a return type and a parameter list.
///
/// Matching on the enum rather than calling `BasicType::fn_type` keeps the
/// "unrepresentable in return position" refusal explicit — and `BasicType` is
/// deliberately not imported, which is why `declare_all` has always done it this
/// way. Factored out here because three sites now need the same shape: top-level
/// declarations, lifted lambda declarations, and the indirect call site.
fn fn_type_of<'ctx>(
    ret: BasicTypeEnum<'ctx>,
    params: &[BasicMetadataTypeEnum<'ctx>],
) -> Result<FunctionType<'ctx>, CodegenError> {
    match ret {
        BasicTypeEnum::IntType(t) => Ok(t.fn_type(params, false)),
        BasicTypeEnum::PointerType(t) => Ok(t.fn_type(params, false)),
        _ => Err(CodegenError::Unsupported("unrepresentable type")),
    }
}
```

Rewrite `declare_all`'s type construction to call `fn_type_of(repr_ty(ctx, &f.body.ty)?, &params)?` in place of its inline `match`, leaving its `func.set_call_conventions(TAILCC)` line untouched.

- [ ] **Step 4: State the lambda cap next to the function cap**

Immediately below `const MAX_PARAMS: usize = 5;` in `crates/codegen/src/lib.rs`:

```rust
/// The lambda cap. A lambda of P source parameters is lifted to a function of
/// arity P + 1 — the closure pointer is parameter 0 (§3, C-iii) — so the flat
/// arity cap of five converts to a source cap of FOUR.
///
/// C-ii probe, `scratchpad/n5-indirect-probe`, LLVM 18.1.6 /
/// `x86_64-pc-windows-msvc`, 2026-09-03. Sweeping caller arity C x callee arity K
/// over 1..8 for a `musttail` call under `tailcc`, the INDIRECT-callee matrix is
/// cell-for-cell identical to the direct-callee matrix: K <= 5 compiles for every
/// C; K in {6,7} compiles only when C >= 6; K = 8 only when C >= 8. Every passing
/// cell emits a real tail jump (`jmpq *%rax` for the indirect form) — no silent
/// degradation to a call. Every failing cell is the same `report_fatal_error`
/// `MAX_PARAMS` guards against. Nothing is claimed outside 1..8: `@elya_main` is
/// arity 0, and that cell rests on the shipped 5b-3/5b-4 corpus, which already
/// `musttail`s out of arity-0 `@elya_main` and runs.
///
/// This number is MEASURED. Do not raise it to make a program compile; re-run the
/// probe, or lower it.
const MAX_LAMBDA_PARAMS: usize = 4;
```

Append to `MAX_PARAMS`'s existing docstring (do not replace its text):

```rust
/// Lambdas are capped separately and lower, at `MAX_LAMBDA_PARAMS`, because
/// closure conversion adds the environment parameter. The C-ii probe confirmed
/// the matrix is identical for an indirect callee, so one measured cap governs
/// both — applied to the CONVERTED arity.
```

- [ ] **Step 5: Extend `LowerCtx` and split the call path**

```rust
struct LowerCtx<'ctx> {
    decls: &'ctx HashMap<String, FunctionValue<'ctx>>,
    ctors: &'ctx HashMap<String, (usize, Vec<Ty>)>,
    /// Every lambda site in the module, in the pre-order `collect_lambdas` fixed.
    lambdas: &'ctx [LambdaSite],
    /// Core node address -> index into `lambdas`.
    lambda_index: &'ctx HashMap<usize, usize>,
    /// `LambdaSite::symbol` -> the declared lifted function.
    lifted: &'ctx HashMap<String, FunctionValue<'ctx>>,
    alloc: FunctionValue<'ctx>,
    fail: FunctionValue<'ctx>,
    gc_push: FunctionValue<'ctx>,
    gc_pop: FunctionValue<'ctx>,
}
```

Replace `build_elya_call`'s body with a dispatcher and move its old body into `build_direct_call`. Two mechanical rules, both easy to get wrong by copy-pasting the block below:

1. **Move the old body *with its comments*.** `build_elya_call`'s current body (`crates/codegen/src/lib.rs:325-368`) carries three long comments — the two-layer root layering, the left-to-right per-argument rooting, and why the arguments come off LIFO before the call. Those are 5b-5's reasoning, and they belong to the code that still does it. The `build_direct_call` block below shows the signature and the shape with those comments **elided for brevity**; do not let the elision reach the source.
2. **Replace the old doc comment.** The doc comment above `build_elya_call` (`lib.rs:307-315`) says the "computed callee" refusal fires "before the existing `Lambda` arm is ever reached" — false the moment Step 6 makes that arm succeed. Swap it for the dispatcher doc comment shown here. The refusal itself moves into `build_closure_call`, where it still fires before any argument is lowered, now keyed on the callee's *type* rather than its syntactic form.

```rust
/// Dispatch one Elya call. A callee that names a LOCAL holds a closure pointer,
/// so the call is indirect; a callee that names a top-level function is direct.
/// `env` is consulted before `decls`, the same order `lower_expr`'s head guard
/// uses, so a local shadowing a top-level name resolves to the local.
#[allow(clippy::too_many_arguments)]
fn build_elya_call<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'ctx>,
    callee: &CoreExpr,
    args: &[CoreExpr],
    env: &mut HashMap<String, BasicValueEnum<'ctx>>,
    tail: bool,
) -> Result<CallSiteValue<'ctx>, CodegenError> {
    if let CoreKind::Var(name) = &callee.kind {
        if !env.contains_key(name) {
            let target = *lc.decls.get(name).ok_or(CodegenError::Unsupported(
                "callee is not a top-level function",
            ))?;
            return build_direct_call(ctx, func, b, lc, target, args, env, tail);
        }
    }
    build_closure_call(ctx, func, b, lc, callee, args, env, tail)
}

#[allow(clippy::too_many_arguments)]
fn build_direct_call<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'ctx>,
    target: FunctionValue<'ctx>,
    args: &[CoreExpr],
    env: &mut HashMap<String, BasicValueEnum<'ctx>>,
    tail: bool,
) -> Result<CallSiteValue<'ctx>, CodegenError> {
    let env_roots = if tail { 0 } else { gc_root_env(b, lc, env)? };
    let mut vals: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(args.len());
    let mut arg_roots = 0usize;
    for a in args.iter() {
        let v = lower_expr(ctx, func, b, lc, a, env)?;
        if gc_root(b, lc, v)? {
            arg_roots += 1;
        }
        vals.push(v.into());
    }
    gc_unroot(b, lc, arg_roots)?;
    let site = b.build_call(target, &vals, "c").map_err(internal)?;
    // Singular here (CallSiteValue), plural on the declaration (FunctionValue).
    site.set_call_convention(TAILCC);
    gc_unroot(b, lc, env_roots)?;
    Ok(site)
}

/// Call through a closure. The signature is read WHOLE off the callee's own
/// recorded `Ty::Fn` — not reconstructed from surrounding context, which is the
/// reconstruction §4 rejected. Parameter 0 is the closure pointer itself.
#[allow(clippy::too_many_arguments)]
fn build_closure_call<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'ctx>,
    callee: &CoreExpr,
    args: &[CoreExpr],
    env: &mut HashMap<String, BasicValueEnum<'ctx>>,
    tail: bool,
) -> Result<CallSiteValue<'ctx>, CodegenError> {
    let i64t = ctx.i64_type();
    let ptrt = ctx.ptr_type(AddressSpace::default());
    let Ty::Fn(param_tys, _, ret_ty) = &callee.ty else {
        return Err(CodegenError::Unsupported("computed callee"));
    };
    if param_tys.len() != args.len() {
        return Err(CodegenError::Unsupported("closure call arity mismatch"));
    }
    let mut sig: Vec<BasicMetadataTypeEnum<'ctx>> = Vec::with_capacity(args.len() + 1);
    sig.push(ptrt.into());
    for t in param_tys.iter() {
        sig.push(repr_ty(ctx, t)?.into());
    }
    let fn_ty = fn_type_of(repr_ty(ctx, ret_ty)?, &sig)?;

    let env_roots = if tail { 0 } else { gc_root_env(b, lc, env)? };
    let clos = lower_expr(ctx, func, b, lc, callee, env)?.into_pointer_value();
    // The closure is rooted UNCONDITIONALLY — including at a tail call, where
    // `env_roots` is 0 by design. It is about to become argument 0, and lowering
    // an argument can allocate, so this is the one root a tail call still needs.
    b.build_call(lc.gc_push, &[clos.into()], "")
        .map_err(internal)?;
    let mut vals: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(args.len() + 1);
    vals.push(clos.into());
    let mut arg_roots = 0usize;
    for a in args.iter() {
        let v = lower_expr(ctx, func, b, lc, a, env)?;
        if gc_root(b, lc, v)? {
            arg_roots += 1;
        }
        vals.push(v.into());
    }
    // Word 1 is the code pointer. Loading is not an allocation, so it is safe
    // after the arguments and before the unroot.
    let slot = unsafe { b.build_gep(i64t, clos, &[i64t.const_int(1, false)], "cp") }
        .map_err(internal)?;
    let code = b
        .build_load(i64t, slot, "cw")
        .map_err(internal)?
        .into_int_value();
    let fp = b.build_int_to_ptr(code, ptrt, "i2f").map_err(internal)?;
    gc_unroot(b, lc, arg_roots + 1)?;
    let site = b
        .build_indirect_call(fn_ty, fp, &vals, "ci")
        .map_err(internal)?;
    site.set_call_convention(TAILCC);
    gc_unroot(b, lc, env_roots)?;
    Ok(site)
}
```

`lower_tail`'s `App` arm needs **no change**: both paths return a `CallSiteValue`, and it sets `LLVMTailCallKindMustTail` on whichever it gets.

- [ ] **Step 6: Allocate the closure**

Replace `CoreKind::Lambda(..) => Err(CodegenError::Unsupported("Lambda"))` in `lower_expr` with:

```rust
        CoreKind::Lambda(..) => {
            let i64t = ctx.i64_type();
            // Identity by node address: the pre-pass and this emitter hold the
            // same immutable `&CoreModule`, so a miss is a real bug, not a
            // tolerable absence.
            let key = e as *const CoreExpr as usize;
            let idx = *lc.lambda_index.get(&key).ok_or(CodegenError::Unsupported(
                "lambda site missing from the pre-pass",
            ))?;
            let site = &lc.lambdas[idx];
            let code_fn = *lc.lifted.get(&site.symbol).ok_or(CodegenError::Unsupported(
                "lambda body was never declared",
            ))?;
            // Same bracket shape as `CoreKind::Ctor`: `elya_alloc` is the one
            // call here that can collect, so the environment is rooted across it.
            let env_roots = gc_root_env(b, lc, env)?;
            let p = b
                .build_call(
                    lc.alloc,
                    &[i64t
                        .const_int((2 + site.captures.len()) as u64, false)
                        .into()],
                    "cl",
                )
                .map_err(internal)?
                .try_as_basic_value()
                .left()
                .ok_or(CodegenError::Unsupported("elya_alloc returned no value"))?
                .into_pointer_value();
            // Word 0: the synthetic tag, so `gc_mark` finds a descriptor row for
            // this block exactly as it does for a constructor (Task 4).
            b.build_store(p, i64t.const_int(site.tag as u64, false))
                .map_err(internal)?;
            // Word 1: the code pointer, stored as a word and NOT traced.
            let cp = unsafe { b.build_gep(i64t, p, &[i64t.const_int(1, false)], "cp") }
                .map_err(internal)?;
            let code = b
                .build_ptr_to_int(code_fn.as_global_value().as_pointer_value(), i64t, "f2i")
                .map_err(internal)?;
            b.build_store(cp, code).map_err(internal)?;
            // Words 2..: the captures, in name order. Values come straight out of
            // `env` — nothing here allocates, so `env_roots` already covers them.
            for (i, (name, ty)) in site.captures.iter().enumerate() {
                let v = *env
                    .get(name)
                    .ok_or(CodegenError::Unsupported("captured name is not in scope"))?;
                let word = match ty {
                    Ty::Base(TyCon::Int) => v.into_int_value(),
                    Ty::Base(TyCon::Bool) => b
                        .build_int_z_extend(v.into_int_value(), i64t, "zw")
                        .map_err(internal)?,
                    _ => b
                        .build_ptr_to_int(v.into_pointer_value(), i64t, "p2i")
                        .map_err(internal)?,
                };
                let cs = unsafe {
                    b.build_gep(i64t, p, &[i64t.const_int((i + 2) as u64, false)], "cs")
                }
                .map_err(internal)?;
                b.build_store(cs, word).map_err(internal)?;
            }
            gc_unroot(b, lc, env_roots)?;
            Ok(p.into())
        }
```

- [ ] **Step 7: Declare and emit the lifted bodies**

Add two functions to `crates/codegen/src/lib.rs`:

```rust
/// One LLVM function per lambda site. Parameter 0 is the closure pointer — the
/// environment IS the closure (§3, C-iii) — then the source parameters in order.
/// `tailcc` on every one of them, for the same reason every top-level Elya
/// function gets it: `musttail`'s convention-match requirement is then true by
/// construction.
fn declare_lifted<'ctx>(
    ctx: &'ctx Context,
    module: &Module<'ctx>,
    sites: &[LambdaSite],
) -> Result<HashMap<String, FunctionValue<'ctx>>, CodegenError> {
    let ptrt = ctx.ptr_type(AddressSpace::default());
    let mut out = HashMap::new();
    for site in sites {
        let mut params: Vec<BasicMetadataTypeEnum<'ctx>> =
            Vec::with_capacity(site.params.len() + 1);
        params.push(ptrt.into());
        for p in site.params.iter() {
            params.push(repr_ty(ctx, &p.ty)?.into());
        }
        let fn_ty = fn_type_of(repr_ty(ctx, &site.ret)?, &params)?;
        let f = module.add_function(&mangle(&site.symbol), fn_ty, None);
        f.set_call_conventions(TAILCC);
        out.insert(site.symbol.clone(), f);
    }
    Ok(out)
}

/// Emit one lifted body. Captures are loaded ONCE, at entry, into the same `env`
/// the lowering fold already threads — so from that point a captured name is an
/// ordinary SSA binding and `gc_root_env` roots it exactly as it roots a
/// parameter. That is why the closure pointer itself needs no root inside the
/// body: nothing re-reads it, and a non-recursive lambda never self-calls.
fn emit_lifted<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'ctx>,
    site: &LambdaSite,
) -> Result<(), CodegenError> {
    let func = *lc
        .lifted
        .get(&site.symbol)
        .ok_or(CodegenError::Unsupported("lambda body was never declared"))?;
    let entry = ctx.append_basic_block(func, "entry");
    b.position_at_end(entry);
    let i64t = ctx.i64_type();
    let ptrt = ctx.ptr_type(AddressSpace::default());
    let clos = func
        .get_nth_param(0)
        .ok_or_else(|| internal("lifted body has no environment parameter"))?
        .into_pointer_value();
    let mut env: HashMap<String, BasicValueEnum<'ctx>> = HashMap::new();
    for (i, (name, ty)) in site.captures.iter().enumerate() {
        let cs = unsafe { b.build_gep(i64t, clos, &[i64t.const_int((i + 2) as u64, false)], "cs") }
            .map_err(internal)?;
        let loaded = b
            .build_load(i64t, cs, "cv")
            .map_err(internal)?
            .into_int_value();
        let v: BasicValueEnum<'ctx> = match ty {
            Ty::Base(TyCon::Int) => loaded.into(),
            Ty::Base(TyCon::Bool) => b
                .build_int_truncate(loaded, ctx.bool_type(), "bt")
                .map_err(internal)?
                .into(),
            _ => b
                .build_int_to_ptr(loaded, ptrt, "i2p")
                .map_err(internal)?
                .into(),
        };
        env.insert(name.clone(), v);
    }
    for (i, p) in site.params.iter().enumerate() {
        let v = func
            .get_nth_param((i + 1) as u32)
            .ok_or_else(|| internal("declared lambda arity disagrees with Core"))?;
        env.insert(p.name.clone(), v);
    }
    lower_tail(ctx, func, b, lc, &site.body, &mut env)
}
```

- [ ] **Step 8: Wire it into `build_module`**

In `build_module`, immediately after the existing `MAX_PARAMS` scan (keep that scan first — the comment says so on purpose):

```rust
    // The closure tags continue the constructor numbering, so `first_tag` is the
    // number of REAL descriptor rows — computed the same way the descriptor table
    // below counts them, from `core.types`.
    let n_real_ctors: usize = core.types.iter().map(|t| t.ctors.len()).sum();
    let lambdas = closure::collect_lambdas(core, n_real_ctors);
    // §5.1, and for the same reason the scan above is first: over-cap is a
    // `report_fatal_error` inside LLVM, so it must be refused BEFORE anything is
    // emitted. One site governs the whole module: every `Ty::Fn` value in a
    // whole-module compile originates at a lambda site this loop has seen.
    for site in &lambdas {
        if site.params.len() > MAX_LAMBDA_PARAMS {
            return Err(CodegenError::Unsupported(
                "lambda takes more than four parameters",
            ));
        }
    }
    let lambda_index: HashMap<usize, usize> =
        lambdas.iter().enumerate().map(|(i, s)| (s.key, i)).collect();
```

Then, after `declare_all` and the runtime declarations, build `lifted` and pass all three into `LowerCtx`:

```rust
    let lifted = declare_lifted(ctx, &module, &lambdas)?;
```

```rust
    let lc = LowerCtx {
        decls: &decls,
        ctors: &ctors,
        lambdas: &lambdas,
        lambda_index: &lambda_index,
        lifted: &lifted,
        alloc,
        fail,
        gc_push,
        gc_pop,
    };
```

and emit the lifted bodies alongside the top-level ones:

```rust
    for f in &core.fns {
        emit_body(ctx, &b, &lc, f)?;
    }
    for site in &lambdas {
        emit_lifted(ctx, &b, &lc, site)?;
    }
```

If the borrow checker objects to `&lambdas` living in `lc` while `lambdas` is also iterated, take the iteration over `lc.lambdas` instead — the data is the same slice.

- [ ] **Step 9: Run the three tests**

```sh
CARGO_INCREMENTAL=0 cargo test -p elya-codegen --lib rejects_a_lambda_with_five_parameters
CARGO_INCREMENTAL=0 cargo test -p elya-codegen --lib rejects_a_function_name_in_value_position
CARGO_INCREMENTAL=0 cargo test -p elya-codegen --test native_codegen a_closure_
```

`rejects_a_function_name_in_value_position` (`crates/codegen/src/lib.rs:1531-1549`) is the thin cut of §6, and it must pass **unmodified** — if making closures work required editing it, the cut moved without a decision. It survives structurally rather than by luck: `lower_expr`'s head guard (`lib.rs:465-469`) tests `!env.contains_key(x) && lc.decls.contains_key(x)` and returns *before* `repr_ty` is consulted, so widening `repr_ty` with `Ty::Fn(..) => ptr` cannot turn `let f = add3` into a representable value. A local shadowing a top-level name is in `env`, so it takes the closure path — the same `env`-before-`decls` order the new dispatcher uses.

Expected: all four PASS. `a_closure_tail_call_recurs_in_bounded_stack` at 1,000,000 frames is the one that would die with `STATUS_STACK_OVERFLOW` (`0xC00000FD`) if `musttail` had degraded to a plain call. If the harness reports a bare `exit=127` from a Bash-launched run, re-run under PowerShell to see the real code:

```powershell
$p = Start-Process -NoNewWindow -Wait -PassThru <exe>; "0x{0:X8}" -f $p.ExitCode
```

- [ ] **Step 10: Run the full gate**

```sh
cargo fmt --all
powershell -NoProfile -File scripts/check.ps1
```

Expected: all five stages green. Note for the reviewer: at this point the collector **cannot** trace a closure. That is the state Task 4 measures against.

- [ ] **Step 11: Commit**

```bash
git add crates/codegen/src/lib.rs crates/codegen/tests/native_codegen.rs
git commit -m "feat(codegen): closure blocks, lifted bodies, the indirect tail call, the four-parameter cap (5b-6 Task 3)"
```

### CHECKPOINT — pause for review

Report: closures build, call, and tail-recur; the cap refuses at five source parameters with the C-ii measurement quoted at the refusal; the collector is still blind to closures **by design**, so HEAD is the un-fixed build Task 4's first negative control runs against.

---

## Task 4: CR-3 — teaching the collector to see a closure, both directions, staged

This is the slice's only **silent** failure mode, so it gets the strongest available proof: each direction's test is named for the predicate it trips, and each is run against a build where only that predicate is un-fixed.

**The two predicates, verbatim from `runtime.c` and `lib.rs`:**

- **Direction (a)** — `runtime.c:141`, inside `gc_mark`: `if (tag < 0 || tag >= gc_n_ctors) { continue; }`. A closure whose tag has no descriptor row falls into this `continue` and its captures are **never traced**. Silent: the program keeps running and prints a wrong number.
- **Direction (b)** — the descriptor table's pointer-mask predicate in `lib.rs`, today `matches!(f, Ty::Con(..))`. A capture whose type is `Ty::Fn` is a heap pointer but gets **mask bit 0**, so it is not traced. Also silent at mark time; observed later as a use-after-free.

**Retargeting of direction (b), flagged rather than silently substituted.** The user's wording names "the `Ty::Fn`-in-an-ADT-field mask miss." That exact program is **unbuildable through the surface language**: ✓ VERIFIED at `src/ast.rs:64-67` (`TypeAnn { name, args }` has no function-type form) and `src/core.rs:187-199` (`ann_to_ty`), so no ADT field can be declared with a function type. The surface-reachable program that trips **the same predicate** is a closure capturing a closure. Three consequences, all discharged in this task:

1. Both the ADT-field rows and the closure rows call **one shared `is_heap_ty`**, so a single execution test falsifies both call sites.
2. A cheap invariant unit test pins `repr_ty` says pointer ⟺ `is_heap_ty` says traced, over every representable type — including `Ty::Con` and `Ty::Fn`.
3. The ADT-field *execution* test is filed as an obligation for N6/N7, whenever the surface grows a function-type annotation.

**Files:**
- Modify: `crates/codegen/src/lib.rs` (the descriptor table and its mask predicate; a new `is_heap_ty`)
- Test: `crates/codegen/tests/native_codegen.rs` (two execution tests), `crates/codegen/src/lib.rs` (one invariant unit test)

**Interfaces:**
- Consumes: `LambdaSite { tag, captures }` (Task 2), the `[tag][code_ptr][cap_0..]` layout (Task 3).
- Produces: `fn is_heap_ty(ty: &Ty) -> bool`, used by **both** the constructor rows and the closure rows.
- Descriptor row for lambda site *i*: `arity = 1 + captures.len()`; `ptr_mask` has bit 0 **clear** (the code pointer is not a heap object) and bit `j + 1` set iff `is_heap_ty(&captures[j].1)`.

- [ ] **Step 1: Write both failing tests, named for their predicates**

Append to `crates/codegen/tests/native_codegen.rs`:

```rust
/// CR-3, direction (a) — the predicate this trips is `gc_mark`'s
/// `tag >= gc_n_ctors` skip (`runtime.c:141`).
///
/// Without a descriptor row for the closure's synthetic tag, the mark phase hits
/// that `continue` and never traces the closure's captures. `wrap` exists because
/// a function return is the only scope-ender in Elya: after it, the list is
/// reachable ONLY through the closure. `churn` then forces a real collection, and
/// the captured `Cons(7, Nil)` is swept while still live.
///
/// This fails SILENTLY on the un-fixed build — it prints a wrong `Int`, it does
/// not crash — which is why the assertion is on the VALUE, not on survival.
#[test]
fn a_closure_capture_survives_collection_descriptor_row_present() {
    let src = "type L { Nil, Cons(Int, L) }\n\
               fn head_or(d, xs) { match xs { Nil -> d  Cons(h, t) -> h } }\n\
               fn wrap(xs) { fn(d) { head_or(d, xs) } }\n\
               fn churn(n) { if n == 0 { 0 } else { let _ = Cons(1, Nil)  churn(n - 1) } }\n\
               pub fn main() { let f = wrap(Cons(7, Nil))  let _ = churn(100000)  f(0) }\n";
    let dir = temp_dir("clos-gc-tag");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "clos-gc-tag");
    let (stdout, stats) = run_with_gc_stats(&exe, "clos-gc-tag");
    assert!(
        stats.collections > 0,
        "no collection happened across the call, so nothing was proved: {stats:?}"
    );
    assert_eq!(
        stdout, "7",
        "the captured list did not survive collection — the closure's tag has no \
         descriptor row, so gc_mark skipped it"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// CR-3, direction (b) — the predicate this trips is the descriptor table's
/// pointer-mask test, `matches!(f, Ty::Con(..))`, un-widened for `Ty::Fn`.
///
/// `outer` captures `inner`, whose type is `Ty::Fn`. Under the un-widened
/// predicate `inner`'s mask bit is CLEAR, so `outer`'s row says "not a pointer"
/// and `inner` is never traced — even though `outer` itself has a descriptor row
/// and is traced fine. An ADT capture cannot show this: `Ty::Con` already sets the
/// bit, so an ADT test passes on the un-fixed build and controls nothing.
///
/// On the un-fixed build this dereferences a swept block. The exit code is
/// deliberately NOT pinned (an access violation is not a defined outcome); what
/// is pinned is that it does not print 42.
#[test]
fn a_captured_closure_is_traced_mask_covers_ty_fn() {
    let src = "fn churn(n) { if n == 0 { 0 } else { let _ = mk(n)  churn(n - 1) } }\n\
               fn mk(k) { fn(z) { z + k } }\n\
               fn wrap(k) { let inner = fn(x) { x + k }\n\
                            let outer = fn(y) { inner(y) }\n\
                            outer }\n\
               pub fn main() { let f = wrap(10)  let _ = churn(100000)  f(32) }\n";
    let dir = temp_dir("clos-gc-mask");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "clos-gc-mask");
    let (stdout, stats) = run_with_gc_stats(&exe, "clos-gc-mask");
    assert!(
        stats.collections > 0,
        "no collection happened across the call, so nothing was proved: {stats:?}"
    );
    assert_eq!(
        stdout, "42",
        "the captured CLOSURE did not survive collection — its mask bit was clear \
         because the predicate only recognised Ty::Con"
    );
    std::fs::remove_dir_all(&dir).ok();
}
```

**Churn arithmetic, so `collections > 0` is not a hope.** `GC_THRESHOLD_WORDS = 1 << 16 = 65536` words (`runtime.c:52`). Direction (a)'s `Cons(1, Nil)` is `elya_alloc(1 + 2)` = 3 visible words; 100000 iterations = 300000 words. Direction (b)'s `mk(n)` allocates a closure with one capture, `elya_alloc(2 + 1)` = 3 words; likewise 300000 words. Both are comfortably over the threshold — and the `collections > 0` assertion checks it by measurement rather than trusting this paragraph.

- [ ] **Step 2: Run direction (a) against un-fixed HEAD**

```sh
CARGO_INCREMENTAL=0 cargo test -p elya-codegen --test native_codegen a_closure_capture_survives_collection
```

Expected: **FAIL**, with the assertion on `stdout` — a number that is not `7` (whatever the swept block's word 1 happens to read as), and `stats.collections > 0` already satisfied. Nothing was stubbed to produce this: Task 3 shipped closures without descriptor rows, so HEAD *is* the un-fixed build for this predicate.

Record the observed wrong value in the commit message. If instead the process dies, note that too — but do **not** relax the assertion; the point is that the un-fixed build produces a defined-looking wrong answer.

- [ ] **Step 3: Add the synthetic descriptor rows — and only those**

In `crates/codegen/src/lib.rs`'s descriptor-table construction, after the loop that appends one row per real constructor, append one row per lambda site. Leave the constructor rows' mask predicate **exactly as it is** for now:

```rust
    // One SYNTHETIC constructor row per lambda site, continuing the tag numbering.
    // This is what keeps `gc_mark` byte-identical: a closure is just an object
    // whose descriptor happens to have been synthesized rather than declared, so
    // the one function whose failure mode is silent gains no second dispatch path.
    for (i, site) in lambdas.iter().enumerate() {
        // Assigned in `collect_lambdas`'s pre-order; checked here rather than
        // assumed, because a drift between the tag stored in word 0 and the row
        // index would mis-trace silently. A hard error, not a `debug_assert` —
        // release builds must not skip it.
        if site.tag != n_real_ctors + i {
            return Err(CodegenError::Unsupported(
                "lambda tag disagrees with its descriptor row index",
            ));
        }
        // arity = 1 (the code pointer) + the captures.
        rows.push(i64t.const_int((1 + site.captures.len()) as u64, false));
        // Bit 0 is CLEAR: word 1 is a code pointer into the text segment, not a
        // heap object. Bit j+1 is set iff capture j is a heap value.
        let mut mask: u64 = 0;
        for (j, (_, ty)) in site.captures.iter().enumerate() {
            if matches!(ty, Ty::Con(..)) {
                mask |= 1 << (j + 1);
            }
        }
        rows.push(i64t.const_int(mask, false));
    }
```

Adapt the two `rows.push(...)` lines to however the existing table accumulates its `[arity, mask]` pairs (`crates/codegen/src/lib.rs:935-969`) — same vector, same `i64` constant shape, appended after the real rows. `n_real_ctors` and `lambdas` are already in scope from Task 3's Step 8; if the descriptor table is built in a helper, pass both in.

- [ ] **Step 4: Re-run direction (a), then run direction (b)**

```sh
CARGO_INCREMENTAL=0 cargo test -p elya-codegen --test native_codegen a_closure_capture_survives_collection
CARGO_INCREMENTAL=0 cargo test -p elya-codegen --test native_codegen a_captured_closure_is_traced
```

Expected: direction (a) now **PASSES** — the descriptor row alone fixed it. Direction (b) **FAILS**, and it fails for exactly its own predicate: `outer` now has a row and is traced, but `inner`'s mask bit is clear because `matches!(ty, Ty::Con(..))` does not recognise `Ty::Fn`. This is the staging that makes each control control.

- [ ] **Step 5: Widen the predicate — once, shared**

Add to `crates/codegen/src/lib.rs`:

```rust
/// Does a value of this type live on the heap, so the collector must trace it?
///
/// ONE predicate, used by both the constructor descriptor rows and the closure
/// descriptor rows, so a single negative control falsifies both call sites. It
/// must agree with `repr_ty`: exactly the types `repr_ty` represents as a pointer
/// are the types the mask marks traced. `a_captured_closure_is_traced_mask_covers_ty_fn`
/// is the execution proof of the `Ty::Fn` half; `mask_and_repr_agree_on_pointers`
/// pins the correspondence itself.
fn is_heap_ty(ty: &Ty) -> bool {
    matches!(ty, Ty::Con(..) | Ty::Fn(..))
}
```

Replace **both** mask predicates with it — the constructor rows' `matches!(f, Ty::Con(..))` and the closure rows' `matches!(ty, Ty::Con(..))` from Step 3 — so the two sites become `if is_heap_ty(f)` and `if is_heap_ty(ty)`.

Add the invariant unit test to `crates/codegen/src/lib.rs`'s `mod tests`:

```rust
/// `repr_ty` says pointer <=> `is_heap_ty` says traced. If these ever disagree,
/// some value is passed around as a pointer and never traced (a use-after-free)
/// or traced without being one (a wild dereference in `gc_mark`). Cheap to check,
/// and it catches the next `repr_ty` widening that forgets the mask.
#[test]
fn mask_and_repr_agree_on_pointers() {
    let ctx = Context::create();
    let cases = [
        Ty::Base(TyCon::Int),
        Ty::Base(TyCon::Bool),
        Ty::Con("List".to_string(), vec![]),
        Ty::Fn(
            vec![Ty::Base(TyCon::Int)],
            EffectRow::pure(),
            Box::new(Ty::Base(TyCon::Int)),
        ),
    ];
    for ty in cases {
        let repr = repr_ty(&ctx, &ty).expect("every case is representable");
        assert_eq!(
            repr.is_pointer_type(),
            is_heap_ty(&ty),
            "repr_ty and is_heap_ty disagree on {ty:?}"
        );
    }
}
```

- [ ] **Step 6: Run both directions and the whole corpus**

```sh
CARGO_INCREMENTAL=0 cargo test -p elya-codegen --lib mask_and_repr_agree_on_pointers
CARGO_INCREMENTAL=0 cargo test -p elya-codegen --test native_codegen
```

Expected: PASS throughout — both CR-3 directions, the invariant test, and every pre-existing ADT/GC test (the constructor rows are unchanged in behaviour: `is_heap_ty` agrees with `matches!(f, Ty::Con(..))` on every type an ADT field can currently have).

- [ ] **Step 7: Run the full gate**

```sh
cargo fmt --all
powershell -NoProfile -File scripts/check.ps1
```

Expected: all five stages green.

- [ ] **Step 8: Commit**

```bash
git add crates/codegen/src/lib.rs crates/codegen/tests/native_codegen.rs
git commit -m "feat(codegen): synthetic closure descriptor rows and a Ty::Fn-aware pointer mask (5b-6 Task 4, CR-3)"
```

Put the two observed un-fixed failures in the message body — direction (a)'s wrong `stdout`, direction (b)'s crash — so the record shows both controls controlled.

### CHECKPOINT — pause for review

Report both staged failures with their observed output, and flag the direction-(b) retargeting and the N6/N7 obligation it leaves behind.

---

## Task 5: The `live=` space instrument

Obligation T7 was re-filed this slice on a corrected premise: Elya's value graph is acyclic **by construction of the binding forms** (`src/resolve.rs:102-104` resolves a `let`'s right-hand side before binding the name), so a refcounting evaluator and a tracing collector cannot currently diverge in what they reclaim. That makes today the wrong day to *test* for divergence and the right day to build the instrument that will **measure** it, at N8, when `Value::Resume` holding captured frames plausibly makes a cycle constructible (marked `? INFERRED`).

The collector currently reports `collections`, `freed`, and `words_since_gc` — all flow, no level. `live` is the level: total visible words surviving a collection. A refcounting/tracing divergence shows up there and nowhere else.

**Files:**
- Modify: `crates/codegen/src/runtime.c:154-178` (`gc_sweep`), `:230-239` (`elya_gc_report`)
- Test: `crates/codegen/tests/native_codegen.rs` (extend `GcStats` / `run_with_gc_stats`, add one test)

**Interfaces:**
- Consumes: `GcStats { collections, freed }` and `run_with_gc_stats` as they exist at `crates/codegen/tests/native_codegen.rs:722-761`.
- Produces: `GcStats { collections: i64, freed: i64, live: i64 }`; report line `elya-gc: collections=%lld freed=%lld live=%lld words_since_gc=%lld`. Field order is immaterial to the parser — it is `split_whitespace().find_map(strip_prefix)` — but it is pinned anyway so the line stays readable.

- [ ] **Step 1: Write the failing test**

Extend the existing helpers **in place** (do not add a parallel helper):

```rust
#[derive(Debug)]
struct GcStats {
    collections: i64,
    freed: i64,
    /// Visible words still live at the end of the LAST collection — the level,
    /// where `freed` and `words_since_gc` are flows (5b-6 §11, obligation T7).
    live: i64,
}
```

and, in `run_with_gc_stats`, add the third field to the struct literal:

```rust
    let stats = GcStats {
        collections: field("collections="),
        freed: field("freed="),
        live: field("live="),
    };
```

Then append the test:

```rust
/// 5b-6 §11. The steady-state live set of a program whose live data does NOT grow
/// with its iteration count is INDEPENDENT of that count. Running the same program
/// at two iteration counts and asserting the two `live` figures are equal makes
/// the claim without pinning a magic number — so there is no constant here that a
/// future change could be tempted to nudge, and no expected value to edit.
///
/// This is the instrument obligation T7 will be measured with. It is built now,
/// while acyclicity makes divergence unconstructible, so the day a cycle becomes
/// constructible (N8, `Value::Resume` holding captured frames) the measurement
/// already exists rather than being invented under pressure.
#[test]
fn the_live_set_settles_independent_of_iteration_count() {
    let prog = |n: i64| {
        format!(
            "type L {{ Nil, Cons(Int, L) }}\n\
             fn churn(n) {{ if n == 0 {{ 0 }} else {{ let _ = Cons(1, Nil)  churn(n - 1) }} }}\n\
             pub fn main() {{ let keep = Cons(5, Cons(6, Nil))  let _ = churn({n})  \
             match keep {{ Nil -> 0  Cons(h, t) -> h }} }}\n"
        )
    };
    let mut seen: Vec<i64> = Vec::new();
    for (i, n) in [100000i64, 200000].into_iter().enumerate() {
        let tag = format!("gc-live-{i}");
        let dir = temp_dir(&tag);
        let core = lower_src(&prog(n));
        let exe = compile_and_link(&core, &dir, &tag);
        let (stdout, stats) = run_with_gc_stats(&exe, &tag);
        assert_eq!(stdout, "5", "{tag}: the retained list did not survive");
        assert!(
            stats.collections > 0,
            "{tag}: no collection happened, so `live` was never computed: {stats:?}"
        );
        assert!(
            stats.live > 0,
            "{tag}: a live retained list must contribute live words: {stats:?}"
        );
        seen.push(stats.live);
        std::fs::remove_dir_all(&dir).ok();
    }
    assert_eq!(
        seen[0], seen[1],
        "the live set must not grow with the iteration count — it settles"
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

```sh
CARGO_INCREMENTAL=0 cargo test -p elya-codegen --test native_codegen the_live_set_settles
```

Expected: **FAIL** with the panic `run_with_gc_stats`'s `field` closure raises — `no 'live=' field in: elya-gc: collections=... freed=... words_since_gc=...`. The runtime does not report it yet.

- [ ] **Step 3: Accumulate the live level in the sweep**

In `crates/codegen/src/runtime.c`, add the counter next to the existing ones (`:38`):

```c
static int64_t gc_allocated = 0, gc_collections = 0, gc_freed = 0, gc_live = 0;
```

and accumulate it on the **marked** branch of `gc_sweep`:

```c
static void gc_sweep(void) {
    Block **prev = &gc_all_blocks;
    Block *b = gc_all_blocks;
    /* Reset per cycle: `gc_live` is a LEVEL (what survived this collection), not
       a running total like `gc_freed`. */
    gc_live = 0;
    while (b) {
        Block *next = b->next;
        if (b->meta & GC_MARK_BIT) {
            /* `>> 1` discards the mark bit, so this reads the same whether it
               runs before or after the clear below. */
            gc_live += b->meta >> 1;
            b->meta &= ~(intptr_t)GC_MARK_BIT; /* clear for the next cycle */
            prev = &b->next;
        } else {
```

Leave the unmarked branch, the free-list recycling, and `gc_freed++` untouched.

- [ ] **Step 4: Report it**

```c
void elya_gc_report(void) {
    if (getenv("ELY_GC_STATS")) {
        fprintf(stderr,
                "elya-gc: collections=%lld freed=%lld live=%lld words_since_gc=%lld\n",
                (long long)gc_collections, (long long)gc_freed, (long long)gc_live,
                (long long)gc_allocated);
    }
}
```

- [ ] **Step 5: Run the test**

```sh
CARGO_INCREMENTAL=0 cargo test -p elya-codegen --test native_codegen the_live_set_settles
```

Expected: PASS. Both runs report the same positive `live`.

**Caveat to record, measured not nudged:** `GC_MAX_WORDS = 16` (`runtime.c:48`) caps the size-segregated free lists, so blocks of 16+ visible words are `free()`d rather than recycled. Closures with 14+ captures fall outside the recycling path. This is a note about allocator behaviour, not a correctness bound, and `GC_MAX_WORDS` is **not** to be raised to make anything pass.

- [ ] **Step 6: Confirm the whole GC corpus still holds**

```sh
CARGO_INCREMENTAL=0 cargo test -p elya-codegen --test native_codegen
```

Expected: PASS, including the 5b-5 tests that parse the report line — the parser is order-insensitive, so adding a field between `freed=` and `words_since_gc=` cannot break them.

- [ ] **Step 7: Run the full gate**

```sh
cargo fmt --all
powershell -NoProfile -File scripts/check.ps1
```

Expected: all five stages green.

- [ ] **Step 8: Commit**

```bash
git add crates/codegen/src/runtime.c crates/codegen/tests/native_codegen.rs
git commit -m "feat(codegen): report the live word level, and prove it settles (5b-6 Task 5, obligation T7)"
```

---

## Task 6: Close-out — README, the graph, the ledger

**Files:**
- Modify: `README.md:36-52`
- Modify: `docs/superpowers/specs/2026-09-03-elya-slice-5b6-native-closures-design.md` (tick its checklist)
- Modify: this plan (tick every `- [ ]`)
- Modify: `C:\Users\elakk\.claude\projects\e--Programming-language-with-a-toolchain\memory\next-slice-decision.md` and `MEMORY.md` (the slice ledger)
- Run: `graphify update .`

**Interfaces:**
- Consumes: everything Tasks 1–5 shipped.
- Produces: no code interface. This task's deliverable is that the repository's claims about itself match what it does.

- [ ] **Step 1: Amend the backend-coverage paragraph**

Replace `README.md` lines 37–52 (the paragraph beginning "As of Slice 5b-5 the backend covers") with:

```
overrides it. As of Slice 5b-6 the backend covers arithmetic, control flow,
top-level functions, algebraic data types with `match`, and closures (`Int` and
`Bool`, `let`, `+ - *`, the six comparisons, strict `&&`/`||`, `if`/`else`, calls
to named functions of up to five parameters, monomorphic ADTs constructed then
matched to extract an `Int`, and non-recursive lambdas of up to four parameters
that capture their enclosing locals), so anything outside it is rejected by name
rather than mis-compiled — the tree-walking `elya run` remains the full language.
Tail calls are eliminated under a guarantee the LLVM verifier enforces, so
mutually recursive functions recur to any depth in a compiled binary just as they
do under `elya run` — and that holds through a closure call, which jumps through
a loaded code pointer rather than degrading to an ordinary call. ADT values and
closures are heap-allocated (tag + fields; tag + code pointer + captures) and
reclaimed by a threshold-triggered mark-sweep collector backed by a shadow stack,
so a compiled binary that allocates unboundedly in a loop runs in bounded memory
rather than exhausting it; a closure is described to the collector by a synthetic
constructor descriptor row, so it is traced by exactly the code that traces an
ADT. The evaluator refcounts instead, and reclaim timing is unobservable in this
acyclic subset — the collector reports its live word level so the day that stops
being true, the divergence is measured rather than argued. A failed match traps
with a defined error rather than undefined behaviour. Every compiled program in
the test corpus is additionally checked against what the evaluator computes, so
the two never drift apart silently.
```

- [ ] **Step 2: Refresh the knowledge graph**

```sh
graphify update .
```

- [ ] **Step 3: Tick the checklists**

Mark every `- [ ]` in this plan `- [x]`, and tick the acceptance checklist in the design spec.

- [ ] **Step 4: Update the slice ledger in memory**

Rewrite the body of `memory/next-slice-decision.md` so the 5b arc's state is current: 5b-6 (N5, closures) **CLOSED**; the frontier is N6 (strings/io) or N7 (runtime polymorphism), the user's sequencing call. Record the two obligations this slice leaves open, both as tracked items rather than prose:

- **Uniform function representation** (the thin cut). A top-level function used as a value is still refused with `"function used as a value"`. Closing it means giving every function the closure representation (a zero-capture block), which is wanted at **N7** where runtime polymorphism actually needs it. Intended end state recorded so it is not relitigated.
- **The ADT-field `Ty::Fn` execution test.** The mask predicate is widened and unit-pinned by `mask_and_repr_agree_on_pointers`, and its closure half is execution-proved by `a_captured_closure_is_traced_mask_covers_ty_fn`. The *ADT-field* half has no execution test because the surface cannot declare a field of function type (`src/ast.rs:64-67`, `src/core.rs:187-199`). File it against whichever of N6/N7 gives the surface a function-type annotation.
- **T1** stays open and now covers lambda sites too: the flat cap is a refusal, not a lowering.
- **T7** stays open, re-filed on the corrected premise (acyclic by construction of the binding forms) and redirected at N8, with `the_live_set_settles_independent_of_iteration_count` as the instrument that will measure it.

Add the corresponding one-line pointer to `MEMORY.md` if the ledger entry's hook changes.

- [ ] **Step 5: Run the full gate one last time**

```sh
cargo fmt --all
powershell -NoProfile -File scripts/check.ps1
```

Expected: all five stages green.

- [ ] **Step 6: Commit**

```bash
git add README.md docs/superpowers/plans/2026-09-04-elya-slice-5b6-native-closures.md docs/superpowers/specs/2026-09-03-elya-slice-5b6-native-closures-design.md graphify-out
git commit -m "docs(codegen): close out Slice 5b-6 — native closures"
```

---

## Acceptance

The slice is done when all five gate stages are green and these hold, each by execution:

| Claim | Proof |
|---|---|
| A closure captures a local and computes with it after its scope ends | `a_closure_captures_and_is_called_natively` |
| A closure call is a real tail call | `a_closure_tail_call_recurs_in_bounded_stack` (1,000,000 frames) |
| Over-cap lambdas are refused, not mis-compiled | `rejects_a_lambda_with_five_parameters` |
| The collector traces a closure's captures at all | `a_closure_capture_survives_collection_descriptor_row_present`, shown failing on Task 3's HEAD |
| The pointer mask covers `Ty::Fn` | `a_captured_closure_is_traced_mask_covers_ty_fn`, shown failing on Task 4 Step 3's HEAD |
| `repr_ty` and the mask cannot drift | `mask_and_repr_agree_on_pointers` |
| The live set settles | `the_live_set_settles_independent_of_iteration_count` |
| `gc_mark` gained no dispatch path | `git diff` over `runtime.c:134-152` is empty |
| Free-variable analysis is right | the six unit tests in `crates/codegen/src/closure.rs` |
| Lambda parameter types are recorded, not reconstructed | `lambda_parameters_carry_recorded_types` |
| The §6 thin cut held — a top-level function is still not a value | `rejects_a_function_name_in_value_position` passes **unmodified** |
