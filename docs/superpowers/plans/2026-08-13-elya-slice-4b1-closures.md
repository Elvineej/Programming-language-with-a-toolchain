# Elya Slice 4b-1 — Basic Closures & the Value Restriction — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add first-class anonymous functions (`fn(x){ … }`), closures that capture their environment, sound let-polymorphism via the value restriction, and constructors-as-function-values — running identically on the tree-walker and the CEK machine.

**Architecture:** A lambda is an expression literal (`Expr::Lambda`) that evaluates to a `Value::Closure` capturing the persistent `Env` by O(1) `Rc` clone. The latent effect row lives only in the type (`Ty::Fn`); the runtime value carries code + env, and effects thread dynamically at the call site (already how the CEK works). Calling a closure reuses the current continuation (TCE-preserving), exactly like a named function. Let-generalization is gated by `is_syntactic_value`, closing a latent soundness hole that lambdas would otherwise expose. A bare n-ary constructor becomes a callable value by making `Value::Ctor(name, …)` callable (calling it appends args), retiring E0433's bare case.

**Tech Stack:** Rust 2021, `logos` lexer, `ariadne` diagnostics, `insta` snapshots. No new dependencies.

## Global Constraints

- **Rust 2021, MSRV 1.75. No new dependencies.**
- **Every shell session:** `export PATH="$HOME/.cargo/bin:$PATH"` and `export CARGO_INCREMENTAL=0` (incremental cache hangs on this Windows machine; disabling gives ~18 s builds).
- **Before every `sh scripts/check.sh`:** run `cargo fmt --all` (the gate fmt-checks and fails hard).
- **Commits:** end every commit message with `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`. Use **explicit `git add` paths** (never `git add -A` — it has twice swept in stray tooling dirs). Push to `origin/main` after each task's commit.
- **Each task ends with a green build:** `Expr::Lambda` is introduced whole in Task 2 (every exhaustive `match` over `Expr` gets its arm at once) so no task leaves the crate non-compiling.
- **`cek == tree` is a hard gate** from Task 2 (the first task that evaluates a lambda) onward: every `run_both` golden asserts the two evaluators agree, held green through Tasks 3–6.
- **Value-restriction verification is a hard gate (Task 1):** the full existing suite (133 tests) must stay green *after* the `is_syntactic_value` gate lands and *before* any lambda syntax exists — behavior-preservation is **proven, not asserted**.
- **TCE discipline:** measure depth, pin the constant, never raise a `K_MAX_*` to hide a regression — fix the machine.
- Scratch/debug files go in the session scratchpad **outside** the repo, never under a tracked path.

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `src/ast.rs` | AST + pretty-printer | Add `Expr::Lambda`; pretty arm. |
| `src/lex.rs` | Lexer | None (reuses `KwFn`, `LBrace`). |
| `src/parse.rs` | Parser | `atom()` dispatch on `KwFn` → `lambda_expr()`. |
| `src/resolve.rs` | Name resolution | `check_expr` Lambda arm — scope params, check body. |
| `src/exhaust.rs` | Exhaustiveness walk | `walk_expr` Lambda arm — walk the body (so a `match` inside a lambda is still checked). |
| `src/types.rs` | HM inference | `is_syntactic_value` + gate at the `let` site; `Expr::Lambda` typing; bare n-ary ctor → scheme instantiation; E0433 message narrowed. |
| `src/eval.rs` | Both evaluators | `Value::Closure`; lambda construction + closure call on tree and CEK; make `Value::Ctor(name, …)` callable. |
| `tests/adt.rs` | Closure end-to-end (`run_both`) | Closure/capture/HOF/ctor-value goldens + type-level polymorphism/negative-control. |
| `tests/tce_closure.rs` (new) | TCE two-sided | Bounded 1 M closure tail-call + non-tail grow control. |
| `tests/ui/*.elya` + `tests/ui.rs` | Diagnostic fixtures | E0433 partial-application fixture (updated message). |
| `tests/crosscheck.rs` | Differential oracle | One effect-free higher-order program. |

---

## Task 1: The value-restriction gate (behavior-preserving, no lambdas yet)

Land `is_syntactic_value` and gate let-generalization **before** any lambda code, so behavior-preservation is proven on the current corpus with zero lambda syntax present.

**Files:**
- Modify: `src/types.rs` (add `is_syntactic_value`; gate `infer_block`'s `Stmt::Let` at ~line 685-688)
- Test: `src/types.rs` inline test + full suite regression

**Interfaces:**
- Produces: `fn is_syntactic_value(e: &Expr) -> bool` (module-private in `types`).

- [ ] **Step 1: Add the predicate.** In `src/types.rs`, add near the other free functions:

```rust
/// The value restriction: only *syntactic values* may have their `let`-bound
/// type generalized. Generalizing a non-value (an application, `match`, `if`, …)
/// is unsound once first-class functions or continuations exist — a lambda-bound
/// or continuation-captured cell could escape its monomorphic use. Names, literals,
/// and lambdas are values; everything that *computes* is not.
fn is_syntactic_value(e: &Expr) -> bool {
    matches!(
        e,
        Expr::Var(_)
            | Expr::Qualified { .. }
            | Expr::Int(_)
            | Expr::Float(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Unit
    )
}
```

(The `Expr::Lambda` arm is added in Task 2, when the variant exists.)

- [ ] **Step 2: Gate the generalization site.** In `infer_block`, replace the `Stmt::Let` arm body (currently `let scheme = self.generalize(&t, env);`):

```rust
Stmt::Let { name, value } => {
    let t = self.infer_expr(value, env, amb);
    // Value restriction: generalize only syntactic values (spec §2.4). A
    // non-value binding keeps its monotype — sound in the presence of
    // first-class functions/continuations.
    let scheme = if is_syntactic_value(&value.node) {
        self.generalize(&t, env)
    } else {
        Scheme {
            vars: Vec::new(),
            row_vars: Vec::new(),
            ty: self.resolve(&t),
        }
    };
    env.insert(name, scheme);
}
```

- [ ] **Step 3: Add a value-stays-polymorphic unit test** (proves the gate does not *over*-restrict). In `src/types.rs` `#[cfg(test)] mod tests`, add:

```rust
#[test]
fn value_restriction_keeps_values_polymorphic() {
    // `Nil` is a syntactic value (a Var / nullary ctor), so a let-bound `Nil`
    // stays polymorphic and unifies at two distinct element types.
    let src = "type List(a) { Nil, Cons(a, List(a)) }\n\
               pub fn main() {\n\
                 let e = Nil\n\
                 let _ = Cons(1, e)\n\
                 let _ = Cons(\"a\", e)\n\
                 io.println(\"ok\")\n\
               }\n";
    assert!(
        crate::check_source("t.elya", src).is_ok(),
        "{:?}",
        crate::check_source("t.elya", src)
    );
}
```

- [ ] **Step 4: Run the focused test + the whole suite (the hard gate).**

Run: `cargo test 2>&1 | grep -E "test result|FAILED"`
Expected: **all green (133 prior + 1 new)**. Any red is a real finding — a test relying on generalizing a *non-value* binding was depending on the now-closed unsound path; investigate that binding (bind the value to a name, or accept the monotype) before proceeding.

- [ ] **Step 5: Commit + push.**

```bash
cargo fmt --all && sh scripts/check.sh
git add src/types.rs
git commit -m "$(printf 'feat(types): value restriction on let-generalization\n\nGate generalization on is_syntactic_value (spec 4b-1 §2.4): only syntactic\nvalues (names, literals) generalize; applications/match/if keep their monotype.\nCloses a latent soundness hole that lambdas + first-class continuations would\nexpose. Behavior-preserving on the current corpus (non-value bindings are all\nmonomorphic) — proven by the full suite staying green before any lambda syntax\nexists.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

---

## Task 2: `Expr::Lambda` — parse, resolve, type, and construct on both evaluators

Introduce the lambda variant end-to-end far enough to **compile and construct** a closure (all exhaustive `Expr` matches get their arm at once). Calling closures is Task 3; here a lambda parses, resolves, type-checks to a `Ty::Fn`, and evaluates to a `Value::Closure`.

**Files:**
- Modify: `src/ast.rs` (variant + `pretty_expr` arm)
- Modify: `src/parse.rs` (`atom()` dispatch + `lambda_expr()`)
- Modify: `src/resolve.rs` (`check_expr` Lambda arm)
- Modify: `src/exhaust.rs` (`walk_expr` Lambda arm)
- Modify: `src/types.rs` (`infer_expr` Lambda arm; `is_syntactic_value` Lambda arm)
- Modify: `src/eval.rs` (`Value::Closure`; construct arm in tree `eval_expr` and CEK `eval`)
- Test: `src/parse.rs` inline round-trip; `tests/adt.rs` construct-only + type-level polymorphism/negative-control

**Interfaces:**
- Produces: `Expr::Lambda { params: Vec<Spanned<Param>>, body: Rc<Spanned<Block>> }`; `Value::Closure { params: Rc<[String]>, body: Rc<Spanned<Block>>, env: Env }`.
- Consumes: `self.fresh()`, `self.fresh_row()`, `self.resolve_row`, `self.infer_block`, `Ty::Fn`, `EffectRow::open`, `env.push/pop/insert`, `Env::clone`, `eval_block`/`eval_block_state` (existing).

- [ ] **Step 1: Add the AST node + pretty arm.** In `src/ast.rs`, add to `enum Expr` (after `Match`):

```rust
    /// An anonymous function (Slice 4b-1): `fn(x, y) { … }`. Uncurried, block-bodied,
    /// untyped params (HM infers). Evaluates to a `Value::Closure` capturing its env.
    Lambda {
        params: Vec<Spanned<Param>>,
        body: Rc<Spanned<Block>>,
    },
```

and to `pretty_expr`:

```rust
        Expr::Lambda { params, body } => {
            s.push_str("(fn (");
            for (i, p) in params.iter().enumerate() {
                if i > 0 {
                    s.push(' ');
                }
                s.push_str(&p.node.name);
            }
            s.push_str(") ");
            pretty_block(&body.node, s);
            s.push(')');
        }
```

- [ ] **Step 2: Add the parser.** In `src/parse.rs` `atom()`, add the first arm:

```rust
            TokenKind::KwFn => self.lambda_expr(),
```

and the method near `match_expr`:

```rust
    /// Parse an anonymous function `fn ( params ) { block }` in expression position.
    fn lambda_expr(&mut self) -> Option<Spanned<Expr>> {
        let start = self.peek_span();
        self.bump(); // `fn`
        if !self.eat(&TokenKind::LParen) {
            self.error(self.peek_span(), "expected `(` after `fn` in a lambda");
            return None;
        }
        let mut params = Vec::new();
        if self.peek() != Some(&TokenKind::RParen) {
            loop {
                let pspan = self.peek_span();
                match self.peek()?.clone() {
                    TokenKind::Lower(pn) => {
                        self.bump();
                        if self.eat(&TokenKind::Colon) {
                            self.skip_type_annotation();
                        }
                        params.push(spanned(Param { name: pn }, pspan));
                    }
                    _ => {
                        self.error(pspan, "expected parameter name");
                        return None;
                    }
                }
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
        }
        if !self.eat(&TokenKind::RParen) {
            self.error(self.peek_span(), "expected `)`");
            return None;
        }
        let body = self.block()?;
        let end = body.span;
        Some(spanned(
            Expr::Lambda {
                params,
                body: Rc::new(body),
            },
            start.merge(end),
        ))
    }
```

- [ ] **Step 3: Add the resolver arm.** In `src/resolve.rs` `check_expr` (mirror the `Expr::Handle` clause scoping):

```rust
            Expr::Lambda { params, body } => {
                scope.push(HashSet::new());
                for p in params {
                    scope.last_mut().unwrap().insert(p.node.name.clone());
                }
                self.check_block(&body.node, scope);
                scope.pop();
            }
```

- [ ] **Step 4: Add the exhaustiveness-walk arm.** In `src/exhaust.rs` `walk_expr`, add before the `_ => {}` catch-all (so a `match` inside a lambda body is still checked):

```rust
        Expr::Lambda { body, .. } => walk_block(&body.node, sib, out),
```

- [ ] **Step 5: Add the lambda typing arm + `is_syntactic_value` arm.** In `src/types.rs` `infer_expr`, add an `Expr::Lambda` arm:

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
                    param_tys.push(pv);
                }
                // The lambda has its OWN latent effect row: infer the body under a
                // fresh ambient. Creating the closure performs nothing, so the row
                // is NOT added to the enclosing `amb`; only *calling* it pours the
                // row in (infer_call). This mirrors top-level fn typing (spec §2.1).
                let lam_amb = self.fresh_row();
                let body_ty = self.infer_block(&body.node, env, lam_amb);
                env.pop();
                let row = self.resolve_row(&EffectRow::open(lam_amb));
                Ty::Fn(param_tys, row, Box::new(body_ty))
            }
```

and extend the `is_syntactic_value` `matches!` with `| Expr::Lambda { .. }`.

- [ ] **Step 6: Add the value variant + construction on both evaluators.** In `src/eval.rs` `enum Value`, add:

```rust
    /// A closure (Slice 4b-1): a lambda plus the environment it captured. The
    /// effect row is NOT stored — effects thread dynamically to the call site;
    /// the row lives only in the type. Capturing `env` is an O(1) `Rc` clone.
    Closure {
        params: Rc<[String]>,
        body: Rc<Spanned<Block>>,
        env: Env,
    },
```

In the tree `eval_expr`, add:

```rust
            Expr::Lambda { params, body } => {
                let names: Rc<[String]> = params.iter().map(|p| p.node.name.clone()).collect();
                Ok(Value::Closure {
                    params: names,
                    body: body.clone(),
                    env: env.clone(),
                })
            }
```

In the CEK `eval`, add:

```rust
            Expr::Lambda { params, body } => {
                let names: Rc<[String]> = params.iter().map(|p| p.node.name.clone()).collect();
                State::Return(
                    Value::Closure {
                        params: names,
                        body: body.clone(),
                        env: env.clone(),
                    },
                    k,
                )
            }
```

The hand-written `PartialEq for Value` already has a `_ => false` fallthrough (a closure compares `false` — functions are not comparable). If the compiler flags any *other* exhaustive `match` on `Value` lacking a catch-all (e.g. `apply_binop`), add a `_ =>` arm returning that function's existing "unsupported operand" error. `match_pattern` already falls through (a closure never matches a data/literal pattern).

- [ ] **Step 7: Write the tests.** In `src/parse.rs` tests:

```rust
#[test]
fn lambda_parses_and_round_trips() {
    let (e, d) = parse_expr_str(&Session::new(), "fn(x, y) { x + y }");
    assert!(d.is_empty(), "parse: {d:?}");
    assert_eq!(pretty_expr_public(&e.unwrap().node), "(fn (x y) (+ x y))");
}

#[test]
fn nullary_lambda_parses() {
    let (e, d) = parse_expr_str(&Session::new(), "fn() { 0 }");
    assert!(d.is_empty(), "parse: {d:?}");
    assert_eq!(pretty_expr_public(&e.unwrap().node), "(fn () 0)");
}
```

In `tests/adt.rs` (construct-only run + type-level polymorphism + negative control; none of these *call* a closure, so they pass before Task 3):

```rust
#[test]
fn lambda_constructs_and_runs() {
    // The closure is built and discarded; the program runs on both evaluators.
    let src = "pub fn main() {\n\
                 let _f = fn(n) { n + 1 }\n\
                 io.println(\"ok\")\n\
               }\n";
    assert_eq!(run_both(src), "ok\n");
}

#[test]
fn lambda_bound_identity_is_polymorphic() {
    // `id` is a syntactic value (a lambda) -> generalized -> typable at two types.
    // (Type-level only; running it needs the call path from Task 3.)
    let src = "pub fn main() {\n\
                 let id = fn(x) { x }\n\
                 let _ = id(1)\n\
                 let _ = id(\"a\")\n\
                 io.println(\"ok\")\n\
               }\n";
    assert!(
        check_source("t.elya", src).is_ok(),
        "{:?}",
        check_source("t.elya", src)
    );
}

#[test]
fn value_restriction_blocks_nonvalue_generalization() {
    // The RHS is a *call* (a non-value), so `r` keeps its monotype and cannot be
    // used at two types. The value restriction with teeth.
    let src = "pub fn main() {\n\
                 let r = (fn(x) { x })(1)\n\
                 let _ = r + 1\n\
                 io.println(r)\n\
               }\n";
    let err = check_source("t.elya", src).unwrap_err();
    assert!(err.contains("E0400"), "expected a type mismatch, got: {err}");
}
```

- [ ] **Step 8: Build, then run.**

Run: `cargo build 2>&1 | grep -E "^error" | head` → expect none (all exhaustive matches handled).
Run: `cargo test --lib parse::tests::lambda parse::tests::nullary_lambda 2>&1 | grep -E "test result|FAILED"` → PASS.
Run: `cargo test --test adt lambda_constructs lambda_bound_identity value_restriction_blocks 2>&1 | grep -E "test result|FAILED"` → PASS.
Run the full suite: `cargo test 2>&1 | grep -E "test result|FAILED"` → all green.

- [ ] **Step 9: Commit + push.**

```bash
cargo fmt --all && sh scripts/check.sh
git add src/ast.rs src/parse.rs src/resolve.rs src/exhaust.rs src/types.rs src/eval.rs tests/adt.rs
git commit -m "$(printf 'feat: lambda literals fn(x){...} — parse, resolve, type, construct\n\nExpr::Lambda end-to-end: parses in expression position, params scoped by the\nresolver, matches inside a lambda body are exhaustiveness-checked, types to\nTy::Fn with its own latent ambient row (creating a closure performs nothing),\nand evaluates to Value::Closure (capturing the env, O(1) Rc clone) on both\nevaluators. Lambda is a syntactic value -> lambda-bound names are soundly\npolymorphic; a non-value binding is not over-generalized. Calling is Task 3.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

---

## Task 3: Calling closures + constructor-values on both evaluators

**Files:**
- Modify: `src/eval.rs` (tree `Expr::Call` fallthrough; CEK `apply_callee` arms)
- Test: `tests/adt.rs` (`run_both` goldens that *call* closures)

**Interfaces:**
- Consumes: `Value::Closure` (Task 2); `Env::extend`, `eval_block`, `eval_block_state`, `CalleeSlot::Value`, `CtorArgs` (existing).

- [ ] **Step 1: Write the failing goldens.** In `tests/adt.rs`:

```rust
#[test]
fn closure_call_and_capture_run() {
    let src = "pub fn main() {\n\
                 let inc = fn(n) { n + 1 }\n\
                 let by = 10\n\
                 let bump = fn(n) { n + by }\n\
                 let _ = inc(41)\n\
                 let r = bump(5)\n\
                 if r == 15 { io.println(\"ok\") } else { io.println(\"no\") }\n\
               }\n";
    assert_eq!(run_both(src), "ok\n");
}

#[test]
fn higher_order_map_over_list() {
    let src = "type List(a) { Nil, Cons(a, List(a)) }\n\
               fn map(xs, f) { match xs { Nil -> Nil  Cons(h, t) -> Cons(f(h), map(t, f)) } }\n\
               fn sum(acc, xs) { match xs { Nil -> acc  Cons(h, t) -> sum(acc + h, t) } }\n\
               pub fn main() {\n\
                 let xs = Cons(1, Cons(2, Cons(3, Nil)))\n\
                 let ys = map(xs, fn(n) { n * 10 })\n\
                 if sum(0, ys) == 60 { io.println(\"ok\") } else { io.println(\"no\") }\n\
               }\n";
    assert_eq!(run_both(src), "ok\n");
}
```

- [ ] **Step 2: Run — expect failure** ("value is not callable").

Run: `cargo test --test adt closure_call_and_capture higher_order_map 2>&1 | grep -E "test result|FAILED|callable"`
Expected: FAIL.

- [ ] **Step 3: Tree-walker call path.** In `src/eval.rs` `eval_expr` `Expr::Call`, replace the `let Value::Fn(fname) = callee_v else { … }` tail with a full match (keeps the existing `Value::Fn` behavior, adds `Closure` and callable `Ctor`):

```rust
                let callee_v = eval_expr(interp, callee, env, fns)?;
                match callee_v {
                    Value::Fn(fname) => {
                        let fdecl = fns
                            .get(fname.as_str())
                            .copied()
                            .ok_or_else(|| rt(span, format!("unknown function `{fname}`")))?;
                        if fdecl.params.len() != args.len() {
                            return Err(rt(
                                span,
                                format!(
                                    "`{}` expects {} argument(s), got {}",
                                    fname,
                                    fdecl.params.len(),
                                    args.len()
                                ),
                            ));
                        }
                        let mut bindings = Vec::with_capacity(fdecl.params.len());
                        for (p, a) in fdecl.params.iter().zip(args.iter()) {
                            bindings.push((p.node.name.clone(), eval_expr(interp, a, env, fns)?));
                        }
                        let call_env = Env::new().extend(&bindings);
                        eval_block(interp, &fdecl.body.node, &call_env, fns)
                    }
                    Value::Closure { params, body, env: cenv } => {
                        if params.len() != args.len() {
                            return Err(rt(span, "closure applied to the wrong number of arguments"));
                        }
                        let mut bindings = Vec::with_capacity(params.len());
                        for (name, a) in params.iter().zip(args.iter()) {
                            bindings.push((name.clone(), eval_expr(interp, a, env, fns)?));
                        }
                        let call_env = cenv.extend(&bindings);
                        eval_block(interp, &body.node, &call_env, fns)
                    }
                    // A bare constructor value (`Some`, `Cons`) applied: append the
                    // args to build the saturated `Ctor`. Saturation is guaranteed
                    // by the type checker (§2.3, §6).
                    Value::Ctor(name, existing) => {
                        let mut vals: Vec<Value> = (*existing.0).clone();
                        for a in args.iter() {
                            vals.push(eval_expr(interp, a, env, fns)?);
                        }
                        Ok(Value::Ctor(name, CtorArgs(Rc::new(vals))))
                    }
                    _ => Err(rt(callee.span, "value is not callable")),
                }
```

- [ ] **Step 4: CEK call path.** In `apply_callee`, add arms before the `CalleeSlot::Value(_)` catch-all:

```rust
            CalleeSlot::Value(Value::Closure { params, body, env: cenv }) => {
                if params.len() != args.len() {
                    return Err(rt(span, "closure applied to the wrong number of arguments"));
                }
                let bindings: Vec<(String, Value)> = params.iter().cloned().zip(args).collect();
                let call_env = cenv.extend(&bindings);
                Ok(eval_block_state(&body.node, call_env, k)) // reuses `k` — TCE-preserving
            }
            CalleeSlot::Value(Value::Ctor(name, existing)) => {
                let mut vals: Vec<Value> = (*existing.0).clone();
                vals.extend(args);
                Ok(State::Return(Value::Ctor(name, CtorArgs(Rc::new(vals))), k))
            }
```

- [ ] **Step 5: Run — expect pass, `cek == tree` green.**

Run: `cargo test --test adt 2>&1 | grep -E "test result|FAILED|divergence"`
Expected: PASS (`run_both` asserts cek == tree by construction).

- [ ] **Step 6: Commit + push.**

```bash
cargo fmt --all && sh scripts/check.sh
git add src/eval.rs tests/adt.rs
git commit -m "$(printf 'feat(eval): call closures + constructor-values, TCE-preserving\n\nThe closure-call path binds params onto the captured env and reuses the\ncontinuation (no frame), so a closure tail call is bounded like a named-fn call;\nsame on tree and CEK. Bare constructor values are callable (append args ->\nsaturated Ctor). cek == tree hard gate green on the closure/capture/HOF goldens.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

---

## Task 4: Bare constructors as function values — retire E0433's bare case

**Files:**
- Modify: `src/types.rs` (`Expr::Var` ctor branch → instantiate scheme; `emit_unapplied_ctor` message)
- Modify: `tests/ui/partial_ctor.elya` (new) + `tests/ui.rs`; reconcile any 4a `unapplied_ctor.elya` fixture
- Test: `tests/adt.rs` (bare ctor as value runs on both evaluators)

**Interfaces:**
- Consumes: `self.instantiate`, `env.lookup`, `self.ctor_arity` (existing); the Task 3 callable-`Value::Ctor` runtime.

- [ ] **Step 1: Write the positive + negative tests.** In `tests/adt.rs`:

```rust
#[test]
fn bare_constructor_is_a_function_value() {
    let src = "type Option(a) { None, Some(a) }\n\
               type List(a) { Nil, Cons(a, List(a)) }\n\
               fn map(xs, f) { match xs { Nil -> Nil  Cons(h, t) -> Cons(f(h), map(t, f)) } }\n\
               pub fn main() {\n\
                 let xs = Cons(1, Cons(2, Nil))\n\
                 let _ys = map(xs, Some)\n\
                 io.println(\"ok\")\n\
               }\n";
    assert_eq!(run_both(src), "ok\n");
}
```

Create `tests/ui/partial_ctor.elya`:

```elya
type List(a) { Nil, Cons(a, List(a)) }
pub fn main() {
  let _ = Cons(1)
  io.println("x")
}
//~ ERROR[E0433]
```

Register it in `tests/ui.rs`. If a 4a `tests/ui/unapplied_ctor.elya` fixture asserts E0433 on a *bare* constructor, update it to a partial application (or delete it) — the bare case is no longer an error.

- [ ] **Step 2: Run — expect failure** (bare `Some` still emits E0433).

Run: `cargo test --test adt bare_constructor 2>&1 | grep -E "test result|FAILED"`
Expected: FAIL.

- [ ] **Step 3: Instantiate the bare constructor scheme.** In `src/types.rs` `infer_expr` `Expr::Var(name)`, remove the bare-ctor E0433 guard so a bare constructor instantiates its (arrow) scheme like any name:

```rust
            Expr::Var(name) => {
                // A bare constructor is a first-class function value (Slice 4b-1):
                // its scheme is already an arrow (`Some : ∀a. (a) -> Option(a)`),
                // so instantiate it like any other name. Partial application stays an
                // error, caught at the *call* site (E0433, infer_call).
                match env.lookup(name) {
                    Some(s) => {
                        let s = s.clone();
                        self.instantiate(&s)
                    }
                    None => Ty::Error, // unresolved names are E0200 from resolution
                }
            }
```

- [ ] **Step 4: Narrow the E0433 message.** Update `emit_unapplied_ctor` (`src/types.rs`) to the partial-application-only wording (preserve the existing label/span structure; change only the message + help):

```rust
            .with_help(format!(
                "Elya constructors are not curried: apply all {arity} arguments, or wrap in a lambda (e.g. `fn(x) {{ {name}(x, …) }}`)"
            )),
```

and the top-line message to e.g. `format!("constructor `{name}` takes {arity} argument(s) but was applied to the wrong number")`. (Keep the `E0433` code and the `.with_label(span, …)`.)

- [ ] **Step 5: Run — expect pass.**

Run: `cargo test --test adt bare_constructor --test ui 2>&1 | grep -E "test result|FAILED"`
Expected: PASS (bare `Some` runs; `Cons(1)` still errors E0433 with the new message).

- [ ] **Step 6: Regression — full suite.**

Run: `cargo test 2>&1 | grep -E "test result|FAILED"`
Expected: all green.

- [ ] **Step 7: Commit + push.**

```bash
cargo fmt --all && sh scripts/check.sh
git add src/types.rs tests/adt.rs tests/ui tests/ui.rs
git commit -m "$(printf 'feat(types): bare constructors are function values; E0433 narrowed\n\nA bare n-ary constructor (Some, Cons) instantiates its arrow scheme -> a\nfirst-class function value, retiring E0433 for the unapplied case (the runtime\nalready makes Value::Ctor callable, Task 3). E0433 retained for partial\napplication (Cons(1)) with a curry-free message.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

---

## Task 5: TCE through a closure call — two-sided

**Files:**
- Create: `tests/tce_closure.rs`

**Interfaces:**
- Consumes: `elya::eval::run_module` + `Interp::peak_kont_depth()` + `check_source` + `parse_module` (the exact harness from `tests/tce_match.rs`).

- [ ] **Step 1: Write the test file.** Create `tests/tce_closure.rs`:

```rust
//! TCE through a *closure* call is a measured guarantee (spec §3.3). The
//! closure-call path reuses the continuation, so a tail call to a closure adds no
//! net depth. We drive a million alternating tail calls between a top-level `drive`
//! and a closure `k` (the closure references top-level fns by name — no recursive
//! local closures, and no self-application, which HM's occurs-check would reject).
//! A per-closure-call frame leak would drive the peak toward the iteration count;
//! the non-tail grow control proves the bound has teeth. Do NOT raise the constant
//! to hide a regression — fix the machine.

use elya::parse::parse_module;
use elya::Session;

// Pinned from the first measurement (mirrors K_MAX_MATCH). If the observed peak
// differs, set this to that value AND confirm it is CONSTANT across N (below) —
// never raise it to mask growth.
const K_MAX_CLOSURE: usize = 6;

fn run_peak(src: &str) -> (String, usize) {
    assert!(
        elya::check_source("t.elya", src).is_ok(),
        "{:?}",
        elya::check_source("t.elya", src)
    );
    let (m, d) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    let interp = elya::eval::run_module(&m).unwrap();
    (interp.output().to_string(), interp.peak_kont_depth())
}

fn drive_prog(n: i64) -> String {
    // `drive`'s else-branch tail-calls the closure `k`; `k`'s body tail-calls
    // `drive` with a fresh closure from `mk`. Both tail positions fire ~n times.
    format!(
        "fn drive(n, k) {{ if n == 0 {{ 0 }} else {{ k(n) }} }}\n\
         fn mk() {{ fn(m) {{ drive(m - 1, mk()) }} }}\n\
         pub fn main() {{ let _ = drive({n}, mk())\n io.println(\"done\") }}\n"
    )
}

#[test]
fn tail_closure_call_is_bounded() {
    let (out, peak) = run_peak(&drive_prog(1_000_000));
    assert_eq!(out, "done\n", "closure tail-call loop must run to completion");
    assert!(
        peak <= K_MAX_CLOSURE,
        "closure tail-call peak={peak} exceeds K_MAX_CLOSURE={K_MAX_CLOSURE}"
    );
}

#[test]
fn tail_closure_call_peak_is_constant_in_n() {
    // The real property: peak does not grow with the iteration count.
    let (_a, small) = run_peak(&drive_prog(100_000));
    let (_b, large) = run_peak(&drive_prog(1_000_000));
    assert_eq!(
        small, large,
        "closure tail-call peak must be constant in N: {small} vs {large}"
    );
}

#[test]
fn non_tail_closure_call_grows_with_length() {
    // `f(h) + fold(t, f)` — the recursive call is under `+`, so each element leaves
    // a frame; peak grows with the list length. Proves the bound distinguishes
    // tail (flat) from non-tail (grows).
    let prog = |n: i64| {
        format!(
            "type List(a) {{ Nil, Cons(a, List(a)) }}\n\
             fn range(n, acc) {{ if n == 0 {{ acc }} else {{ range(n - 1, Cons(n, acc)) }} }}\n\
             fn fold(xs, f) {{ match xs {{ Nil -> 0  Cons(h, t) -> f(h) + fold(t, f) }} }}\n\
             pub fn main() {{ let _ = fold(range({n}, Nil), fn(x) {{ x }})\n io.println(\"done\") }}\n"
        )
    };
    let (_s, shallow) = run_peak(&prog(5));
    let (_d, deep) = run_peak(&prog(50));
    assert!(deep > shallow, "non-tail closure fold must grow: {shallow} vs {deep}");
    assert!(deep >= 45, "expected depth ~proportional to n=50, got {deep}");
}
```

- [ ] **Step 2: Run and read the measured peak.**

Run: `cargo test --test tce_closure 2>&1 | grep -E "test result|FAILED|exceeds|constant"`
Expected: `tail_closure_call_peak_is_constant_in_n` and `non_tail_closure_call_grows_with_length` PASS. If `tail_closure_call_is_bounded` fails with `peak=N exceeds K_MAX_CLOSURE`, read the reported peak: if it is a **small constant** (confirmed by the constant-in-N test passing), set `K_MAX_CLOSURE` to that value and re-run. If the peak **grows with N** (the constant-in-N test fails), that is a real TCE regression in the closure-call arm — the arm must reuse `k`, not push a frame. Fix the machine; do not raise the constant.

- [ ] **Step 3: Commit + push.**

```bash
cargo fmt --all && sh scripts/check.sh
git add tests/tce_closure.rs
git commit -m "$(printf 'test(tce): two-sided TCE through a closure call (K_MAX_CLOSURE)\n\nA million alternating tail calls between a top-level driver and a closure stay\nbounded (peak constant in N); a non-tail closure fold grows with length. No\nrecursive local closures or self-application (HM occurs-check-safe).\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

---

## Task 6: Integration & the Slice-4b-1 exit gate

**Files:**
- Modify: `tests/crosscheck.rs` (one effect-free higher-order program)

- [ ] **Step 1: Broaden the differential oracle.** In `tests/crosscheck.rs` `hand_written()`, add:

```rust
        // Effect-free higher-order program: the cek==tree oracle extends to closures.
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         fn map(xs, f) { match xs { Nil -> Nil  Cons(h, t) -> Cons(f(h), map(t, f)) } }\n\
         fn sum(acc, xs) { match xs { Nil -> acc  Cons(h, t) -> sum(acc + h, t) } }\n\
         pub fn main() {\n\
           let ys = map(Cons(1, Cons(2, Cons(3, Nil))), fn(n) { n + 100 })\n\
           if sum(0, ys) == 306 { io.println(\"ho\") } else { io.println(\"no\") }\n\
         }\n",
```

- [ ] **Step 2: Run the cross-check.**

Run: `cargo test --test crosscheck 2>&1 | grep -E "test result|FAILED|divergence"`
Expected: PASS.

- [ ] **Step 3: The full Slice-4b-1 exit gate.**

```bash
cargo fmt --all
sh scripts/check.sh
```
Expected: exit 0; full suite green (133 prior + all 4b-1 additions). Confirm these rows pass: `value_restriction_keeps_values_polymorphic`, `lambda_parses_and_round_trips`, `lambda_constructs_and_runs`, `lambda_bound_identity_is_polymorphic`, `value_restriction_blocks_nonvalue_generalization`, `closure_call_and_capture_run`, `higher_order_map_over_list`, `bare_constructor_is_a_function_value`, `tail_closure_call_is_bounded`, `tail_closure_call_peak_is_constant_in_n`, `non_tail_closure_call_grows_with_length`, `cek_matches_tree_on_hand_written_corpus`, and every `tests/ui` fixture.

- [ ] **Step 4: Commit + push.**

```bash
git add tests/crosscheck.rs
git commit -m "$(printf 'test(crosscheck): higher-order program in the differential corpus; Slice-4b-1 exit gate\n\ncek == tree extends to closures. Slice 4b-1 complete: lambdas + closures on both\nevaluators, value restriction (behavior-preserving proven), bare constructors as\nfunction values (E0433 narrowed), TCE-through-closure-call bounded two-sided.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')"
git push origin main
```

### Exit criterion

Closures capture and run on both evaluators with `cek == tree` green; lambda-bound names are soundly polymorphic (value restriction proven behavior-preserving on the existing corpus); bare n-ary constructors are first-class function values (E0433 retired for the bare case, retained with a curry-free message for partial application); a closure tail call is bounded (`K_MAX_CLOSURE`, peak constant in N, non-tail grow control); the full suite is green; `cargo fmt --all` + `sh scripts/check.sh` clean.

---

## Self-Review

- **Spec coverage:** §1.1 (lambdas/closures/first-class/E0433) → Tasks 2–4; §2.1 lambda typing → Task 2; §2.2 (call already built) → Task 2's typing uses `infer_call`'s general path unchanged, exercised in Task 3; §2.3 bare ctors → Task 4; §2.4 value restriction → Task 1 (+ negative control Task 2); §3.1 representation & no-cycle → Task 2; §3.2 construction/call → Tasks 2 (construct) & 3 (call); §3.3 TCE two-sided → Task 5; §3.4 cross-check → Tasks 3 & 6; §4 resolution → Task 2 (`check_expr` Lambda arm + the exhaust walk arm so nested matches are checked); §5 diagnostics (no new code; E0433 narrowed) → Task 4; §6 saturation → Task 4; §8 tests → every task; §9 build order → the task sequence.
- **Reorder note (vs spec §9):** the value restriction is pulled to **Task 1** (spec listed it as Task 4) to honor the approval instruction that behavior-preservation be proven on the existing corpus *before* any lambda syntax exists. `Expr::Lambda` is introduced whole in **Task 2** (not split across tasks) because it breaks every exhaustive `match` over `Expr` at once — splitting it would leave a red build. The vertical slice still runs a lambda by Task 2 (construct) and calls one in Task 3.
- **Refinement note (flagged):** the spec's `Value::CtorFn { name, arity }` is implemented as *making `Value::Ctor(name, …)` callable* — behavior-identical, but avoids a new value variant and threading a constructor-arity table through the evaluators (which carry no arity table — they recognize constructors by casing). A bare ctor already evaluates to `Value::Ctor(name, [])`; calling it appends args. Saturation is guaranteed by the type checker.
- **Placeholder scan:** none — every code step is concrete. `K_MAX_CLOSURE` has a concrete starting value plus a measure-and-pin procedure and a constant-in-N invariant that does not depend on the exact number.
- **Type consistency:** `Value::Closure { params: Rc<[String]>, body: Rc<Spanned<Block>>, env: Env }` and `Expr::Lambda { params: Vec<Spanned<Param>>, body: Rc<Spanned<Block>> }` are used consistently across Tasks 2–5; `is_syntactic_value(&Expr) -> bool` is defined in Task 1 and given its `Expr::Lambda` arm in Task 2; `emit_unapplied_ctor` keeps its `E0433` code across the 4a definition and the Task 4 message change.
- **Compile-green invariant:** Task 2 adds Lambda arms to `pretty_expr`, `resolve::check_expr`, `exhaust::walk_expr`, `types::infer_expr`, and both evaluators simultaneously, and adds `Value::Closure` (whose only exhaustive-match fallout — `PartialEq` — already has a `_` arm); every other task modifies existing arms only. No task leaves the crate non-compiling.
