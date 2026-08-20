# Elya Slice 4d-2 — Affine Resources & the Multi-Shot Capture Guarantee — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `linear type` values affine — a second use is E0428, a use across a perform of a `multi` effect is E0429 — turning the E0426 double-free hazard into a type-checked guarantee, scoped honestly to the intra-function local first cut.

**Architecture:** One bounded inference reach: `infer` records which bindings are linear-typed (a `HashSet<Span>` of affine sites) and exposes it via `infer_with_sites`. A new self-contained `affine::check` pass (shaped like `exhaust.rs`) walks each function body, enforcing ≤1 use (E0428) and no-use-across-a-`multi`-perform (E0429), rebuilding the op→multi table from the declarations. No `unify_row`/HM change; `eval.rs` and the GC are untouched.

**Tech Stack:** Rust 2021, `logos`, `ariadne`, `insta`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-16-elya-slice-4d2-affine-resources-design.md`

## Global Constraints

- **Rust 2021, MSRV 1.75. No new dependencies. Two new diagnostic codes: E0428, E0429** (the next free effect/analysis codes — E0420–E0427 taken).
- **Every shell session:** `export PATH="$HOME/.cargo/bin:$PATH"` and `export CARGO_INCREMENTAL=0`.
- **Before every `sh scripts/check.sh`:** run `cargo fmt --all` (fmt-check fails hard).
- **Commits:** end every message with `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`. Use **explicit `git add` paths** (never `git add -A`). Push to `origin/main` after each task. **Gate atomically** — `if sh scripts/check.sh; then git commit …; fi` — so a red gate never commits.
- **TWO-SIDED TEETH (enforced):** E0428 and E0429 each get BOTH sides — the violation fires **and** the legal case is accepted. For E0428: a second use fails, a single use is accepted. For E0429: a use across a **`multi`**-perform fails, the **same** use across a **one-shot** perform is accepted (proving the check keys on `multi`-ness, not on "any perform"). A one-sided test is a plan failure.
- **SCOPED-GUARANTEE HONESTY (enforced):** the guarantee is intra-function local + create-and-consume. Task 4 pins the callee-duplication program as **currently accepted** with an output-verified test (not a comment), so the future inter-procedural fix flips it. Exit criteria state the guarantee is SCOPED and name both tracked obligations; never claim a blanket no-double-use guarantee.
- **Runtime & GC untouched:** do not modify `eval.rs`. Affine is a static use-discipline.
- Scratch/debug files go in the session scratchpad **outside** the repo.

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `src/lex.rs` | Lexer | add `#[token("linear")] KwLinear`. |
| `src/ast.rs` | AST | `TypeDecl.is_linear: bool`. |
| `src/parse.rs` | Parser | accept `linear` before `type`. |
| `src/types.rs` | Inference | record affine binding sites (`HashSet<Span>`); expose `infer_with_sites`. |
| `src/affine.rs` (new) | The affine analysis pass | `pub fn check(module, affine_sites) -> Vec<Diagnostic>` — per-body use-count (E0428) + multi-cross (E0429). |
| `src/lib.rs` | Pipeline | sequence `affine::check` after `infer` (via `infer_with_sites`). |
| `tests/arch/layering.rs` | Layering | add `("affine", 5)` + reference-scan entry. |
| `tests/affine.rs` (new) | Affine guarantee | two-sided E0428/E0429 + the gap-pin. |
| `tests/ui/*.elya` + `tests/ui.rs` | Fixtures | E0428, E0429 negatives. |

`eval.rs`, `resolve.rs` unchanged.

---

## Task 1: `linear` marker + affine-site inference exposure

**Files:**
- Modify: `src/lex.rs`, `src/ast.rs` (`TypeDecl`), `src/parse.rs` (`type_decl` ~320), `src/types.rs` (`Infer` + `infer`/`infer_with_sites`, `Stmt::Let` ~798)
- Test: `src/types.rs` inline (via a new `infer_with_sites`)

**Interfaces:**
- Produces: `TypeDecl { name, params, is_linear: bool, variants }`; `pub fn infer_with_sites(session: &Session, module: &Module) -> (Vec<Diagnostic>, std::collections::HashSet<Span>)`; `pub fn infer(...) -> Vec<Diagnostic>` (delegates, drops sites — signature unchanged, so existing test callers still compile).

- [ ] **Step 1: Lexer + AST + parser.**
  - `src/lex.rs`: add `#[token("linear")]\n    KwLinear,` beside the other keyword tokens.
  - `src/ast.rs` `TypeDecl`: add `pub is_linear: bool,` (after `params`).
  - `src/parse.rs` `type_decl` (~320): the function does `self.bump(); // type`. Immediately before it (the caller dispatches on `KwType`), accept an optional `linear`. Simplest: in the declaration dispatch (where `module()` matches `KwType`), also handle `KwLinear` as a `linear`-prefixed type. Concretely, in `type_decl`, replace the opening so it consumes an optional leading `linear`:

```rust
    fn type_decl(&mut self) -> Option<Spanned<Decl>> {
        let start = self.peek_span();
        let is_linear = self.eat(&TokenKind::KwLinear);
        if is_linear {
            // `linear` must be followed by `type`
            if !self.eat(&TokenKind::KwType) {
                self.error(self.peek_span(), "expected `type` after `linear`");
                return None;
            }
        } else {
            self.bump(); // type
        }
        // … existing name/params/variants parsing …
```

and set `is_linear` in the `TypeDecl { … }` it builds. Then in the top-level declaration dispatcher (`module()`), route a leading `KwLinear` to `type_decl` (add `Some(TokenKind::KwLinear) => self.type_decl(),` beside the `KwType` arm).

- [ ] **Step 2: Fix the compile fallout.** `cargo build 2>&1 | grep -E "^error"` — the `TypeDecl { … }` literals (parser + any `ast.rs`/test pretty construction) need `is_linear: false`; add it where the compiler flags.

- [ ] **Step 3: Record affine sites in inference.** In `src/types.rs`:
  - Add to `struct Infer`: `affine_sites: std::collections::HashSet<Span>,` and init `affine_sites: HashSet::new(),` in `new()`.
  - Add a helper: a set of linear type names, computed from the module (in `infer`/`infer_with_sites` setup): `let linear_types: HashSet<String> = module.decls.iter().filter_map(|d| if let Decl::Type(t) = &d.node { t.is_linear.then(|| t.name.clone()) } else { None }).collect();` — store it on `Infer` (`linear_types: HashSet<String>`), or pass it where needed.
  - A predicate `fn ty_is_linear(&self, t: &Ty) -> bool { matches!(self.resolve(t), Ty::Con(n, _) if self.linear_types.contains(&n)) }`.
  - In `infer_block`'s `Stmt::Let { name, value }` arm (~798): after the binding's type `t` is computed and *before* moving on, if `self.ty_is_linear(&t)`, insert the binding's span: `self.affine_sites.insert(value.span);` — use the `let`-value span as the site key (a stable per-binding span; the affine pass keys uses to the binding by name+scope, and reports the *binding* via this span). Also record top-level fn params: where a fn parameter is bound (the group-inference param loop), if the param's resolved type is linear, insert its `Spanned<Param>` span.
  - Expose it. Refactor so both entry points share the run:

```rust
pub fn infer_with_sites(
    session: &Session,
    module: &Module,
) -> (Vec<Diagnostic>, std::collections::HashSet<Span>) {
    // … the existing body of `infer`, but return (inf.diags, inf.affine_sites) …
}

pub fn infer(session: &Session, module: &Module) -> Vec<Diagnostic> {
    infer_with_sites(session, module).0
}
```

(Move the current `infer` body into `infer_with_sites`; `infer` delegates. All existing callers of `infer` keep compiling.)

- [ ] **Step 4: Test the exposure.** In `src/types.rs` tests:

```rust
#[test]
fn infer_flags_linear_let_binding_as_affine() {
    let src = "linear type Tok { Tok }\n\
               pub fn main() {\n\
                 let t = Tok\n\
                 let n = 1\n\
                 io.println(\"x\")\n\
               }\n";
    let (m, pd) = crate::parse::parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (d, sites) = infer_with_sites(&Session::new(), &m);
    assert!(d.is_empty(), "{d:?}");
    assert_eq!(sites.len(), 1, "exactly the `let t = Tok` binding is affine, not `let n = 1`");
}
```

- [ ] **Step 5: Build + run + atomic gate + commit.**

```bash
cargo build 2>&1 | grep -E "^error" | head || echo OK
cargo test --lib infer_flags_linear_let_binding_as_affine 2>&1 | grep -E "test result|FAILED"
cargo fmt --all && sh scripts/check.sh && \
git add src/lex.rs src/ast.rs src/parse.rs src/types.rs && \
git commit -m "$(printf 'feat(types): linear type modifier + affine binding-site inference exposure\n\nKwLinear + TypeDecl.is_linear + `linear type` parse; infer records which\nlet-bindings/params are linear-typed (HashSet<Span>) and exposes it via\ninfer_with_sites (infer delegates, signature unchanged). No checking yet; the\nbounded inference reach the affine pass consumes. No unify_row/HM change.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')" && \
git push origin main
```

---

## Task 2: The `affine::check` pass + E0428 (use-at-most-once), two-sided

**Files:**
- Create: `src/affine.rs`; Modify: `src/lib.rs` (sequence the pass), `tests/arch/layering.rs`
- Create: `tests/affine.rs`, `tests/ui/affine_double_use.elya`; Modify: `tests/ui.rs`

**Interfaces:**
- Consumes: `infer_with_sites` (Task 1); `Module`/`Expr`/`Block`/`Stmt` from `ast`.
- Produces: `pub fn check(module: &Module, affine_sites: &HashSet<Span>) -> Vec<Diagnostic>`.

- [ ] **Step 1: Create the pass skeleton.** Create `src/affine.rs`:

```rust
//! Slice 4d-2: the affine-resource analysis pass. A `linear type`'s values are
//! affine — used at most once (E0428) and never live across a perform of a
//! `multi` effect (E0429). Self-contained (shaped like `exhaust.rs`): it reads
//! the AST + the affine binding sites `infer` found + an op->multi table it
//! rebuilds from the effect declarations. Intra-function local, create-and-
//! consume; the callee-duplication and over-approximation gaps are documented
//! tracked obligations (spec §6).

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::span::{Span, Spanned};
use std::collections::{HashMap, HashSet};

/// Per-function state: each in-scope affine binding (by name) -> (use count,
/// crossed-a-multi-perform).
struct Ctx<'a> {
    affine_sites: &'a HashSet<Span>,
    multi_ops: &'a HashSet<String>,
    // name -> (uses, crossed)
    live: HashMap<String, (u32, bool)>,
    out: Vec<Diagnostic>,
}

pub fn check(module: &Module, affine_sites: &HashSet<Span>) -> Vec<Diagnostic> {
    // Operation names that belong to a `multi`-declared effect (rebuilt locally,
    // exactly as 4d-1 built Infer.effect_multi — keeps the pass self-contained).
    let mut multi_ops: HashSet<String> = HashSet::new();
    for d in &module.decls {
        if let Decl::Effect(e) = &d.node {
            if e.is_multi {
                for op in &e.ops {
                    multi_ops.insert(op.node.name.clone());
                }
            }
        }
    }
    let mut out = Vec::new();
    for d in &module.decls {
        if let Decl::Fn(f) = &d.node {
            let mut ctx = Ctx { affine_sites, multi_ops: &multi_ops, live: HashMap::new(), out: Vec::new() };
            ctx.walk_block(&f.body.node);
            out.append(&mut ctx.out);
        }
    }
    out
}
```

- [ ] **Step 2: The walk + E0428.** Add to `impl Ctx` (evaluation-order traversal mirroring `exhaust::walk_expr`/`walk_block`; only the affine-relevant arms need bodies, the rest recurse):

```rust
impl<'a> Ctx<'a> {
    fn walk_block(&mut self, b: &Block) {
        for st in b.stmts.iter() {
            match &st.node {
                Stmt::Let { name, value } => {
                    self.walk_expr(value);
                    // Register a newly-bound affine value (its RHS span is the site).
                    if self.affine_sites.contains(&value.span) {
                        self.live.insert(name.clone(), (0, false));
                    }
                }
                Stmt::Expr(e) => self.walk_expr(e),
            }
        }
        if let Some(t) = &b.tail {
            self.walk_expr(t);
        }
    }

    fn walk_expr(&mut self, e: &Spanned<Expr>) {
        match &e.node {
            Expr::Var(name) => {
                if let Some((uses, crossed)) = self.live.get_mut(name) {
                    *uses += 1;
                    let crossed = *crossed;
                    let n = *uses;
                    if n == 2 {
                        self.out.push(
                            Diagnostic::error("E0428", format!("affine value `{name}` used more than once"))
                                .with_label(e.span, "second use here")
                                .with_help(format!("`{name}` has an affine (`linear`) type; an affine value may be used at most once")),
                        );
                    }
                    // (E0429 handled in Task 3.)
                    let _ = crossed;
                }
            }
            Expr::Call { callee, args } => {
                self.walk_expr(callee);
                for a in args.iter() { self.walk_expr(a); }
            }
            Expr::Binary { lhs, rhs, .. } => { self.walk_expr(lhs); self.walk_expr(rhs); }
            Expr::Unary { expr, .. } => self.walk_expr(expr),
            Expr::If { cond, then_block, else_block } => {
                self.walk_expr(cond);
                self.walk_block(&then_block.node);
                self.walk_block(&else_block.node);
            }
            Expr::Block(b) => self.walk_block(b),
            Expr::Match { scrutinee, arms } => {
                self.walk_expr(scrutinee);
                for arm in arms.iter() { self.walk_expr(&arm.node.body); }
            }
            Expr::Handle { body, handler } => {
                self.walk_expr(body);
                for c in &handler.clauses { self.walk_expr(&c.node.body); }
                if let Some(r) = &handler.ret { self.walk_expr(&r.body); }
            }
            Expr::Lambda { body, .. } => self.walk_block(&body.node),
            Expr::Resume { arg } => self.walk_expr(arg),
            _ => {}
        }
    }
}
```

- [ ] **Step 3: Sequence the pass.** In `src/lib.rs` `front_end`, replace the `infer` call so it captures sites and runs `affine::check`:

```rust
    if diags.is_empty() {
        let (idiags, affine_sites) = types::infer_with_sites(session, &module);
        diags.extend(idiags);
        if !diags.iter().any(|d| d.severity == Severity::Error) {
            diags.extend(exhaust::check(&module));
            diags.extend(affine::check(&module, &affine_sites));
        }
    }
```

and add `pub mod affine;` to `lib.rs`.

- [ ] **Step 4: Layering.** In `tests/arch/layering.rs`, add `("affine", 5)` to the layer map and `"affine"` to the module reference-scan list (beside `"exhaust"`).

- [ ] **Step 5: Two-sided E0428 tests.** Create `tests/affine.rs`:

```rust
//! Slice 4d-2: affine-resource guarantee (two-sided). CEK-only where it runs.
use elya::check_source;

fn err(src: &str) -> String {
    check_source("t.elya", src).expect_err("should fail to type-check")
}

#[test]
fn affine_single_use_is_accepted() {
    // Legal side: a linear value used exactly once type-checks.
    let src = "linear type Tok { Tok }\n\
               fn use1(t) { match t { Tok -> \"ok\" } }\n\
               pub fn main() { let t = Tok\n io.println(use1(t)) }\n";
    assert!(check_source("t.elya", src).is_ok(), "{:?}", check_source("t.elya", src));
}

#[test]
fn affine_double_use_is_e0428() {
    // Violation side: two uses of the same affine binding.
    let src = "linear type Tok { Tok }\n\
               fn use1(t) { match t { Tok -> \"ok\" } }\n\
               pub fn main() { let t = Tok\n let _ = use1(t)\n io.println(use1(t)) }\n";
    let e = err(src);
    assert!(e.contains("E0428"), "second use must be E0428: {e}");
}
```

Create `tests/ui/affine_double_use.elya`:

```elya
linear type Tok { Tok }
fn use1(t) { match t { Tok -> "ok" } }
pub fn main() {
  let t = Tok
  let _ = use1(t)
  io.println(use1(t))
}
//~ ERROR[E0428] more than once
```

Register `affine_double_use` in `tests/ui.rs`.

- [ ] **Step 6: Run + atomic gate + commit.**

```bash
cargo test --test affine 2>&1 | grep -E "test result|FAILED" | head
cargo fmt --all && sh scripts/check.sh && \
git add src/affine.rs src/lib.rs tests/arch/layering.rs tests/affine.rs tests/ui/affine_double_use.elya tests/ui.rs && \
git commit -m "$(printf 'feat(affine): affine use-at-most-once check (E0428), two-sided\n\nNew exhaust.rs-shaped affine::check pass sequenced after infer; a linear-typed\nbinding used twice is E0428, a single use is accepted. Self-contained (rebuilds\nthe op->multi table from decls). No eval/GC touch.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')" && \
git push origin main
```

---

## Task 3: E0429 (multi-shot capture) + two-sided teeth

**Files:**
- Modify: `src/affine.rs` (the multi-cross marking + E0429)
- Modify: `tests/affine.rs`; Create: `tests/ui/affine_multi_capture.elya`; Modify: `tests/ui.rs`

**Interfaces:**
- Consumes: `multi_ops` (Task 2, already built in `check`).

- [ ] **Step 1: Two-sided E0429 tests (write first).** In `tests/affine.rs`:

```rust
#[test]
fn affine_across_multi_perform_is_e0429() {
    // Violation: `t` is used after a perform of a `multi` effect (its captured
    // continuation could re-run and use `t` again).
    let src = "effect multi Flip { fn flip() -> Bool }\n\
               linear type Tok { Tok }\n\
               fn use1(t) { match t { Tok -> \"ok\" } }\n\
               pub fn main() {\n\
                 let t = Tok\n\
                 io.println(handle {\n\
                   let _ = flip()\n\
                   use1(t)\n\
                 } with multi { Flip.flip() -> resume(True) })\n\
               }\n";
    let e = err(src);
    assert!(e.contains("E0429"), "use across a multi-perform must be E0429: {e}");
}

#[test]
fn affine_across_oneshot_perform_is_accepted() {
    // Legal side (the teeth): the SAME shape over a ONE-SHOT effect is fine — a
    // one-shot continuation resumes at most once, so no capture hazard.
    let src = "effect Ask { fn ask() -> Bool }\n\
               linear type Tok { Tok }\n\
               fn use1(t) { match t { Tok -> \"ok\" } }\n\
               pub fn main() {\n\
                 let t = Tok\n\
                 io.println(handle {\n\
                   let _ = ask()\n\
                   use1(t)\n\
                 } with { Ask.ask() -> resume(True) })\n\
               }\n";
    assert!(check_source("t.elya", src).is_ok(), "{:?}", check_source("t.elya", src));
}
```

(Add `use elya::check_source;` already present.)

- [ ] **Step 2: Run — expect `affine_across_multi_perform_is_e0429` to FAIL** (no E0429 yet), the one-shot one to PASS.

Run: `cargo test --test affine affine_across 2>&1 | grep -E "test result|FAILED"`
Expected: the multi test FAILS, the one-shot test PASSES.

- [ ] **Step 3: Implement the multi-cross marking + E0429.** In `src/affine.rs`:
  - In `walk_expr`'s `Expr::Call` arm, detect a perform of a `multi` effect: if `callee.node` is `Expr::Var(op)` and `self.multi_ops.contains(op)`, then after walking args, **mark every in-scope affine binding crossed**: `for v in self.live.values_mut() { v.1 = true; }`.
  - In the `Expr::Var` arm, when a use is counted, if the binding is **crossed**, emit E0429:

```rust
                    if crossed {
                        self.out.push(
                            Diagnostic::error("E0429", format!("affine value `{name}` may be captured by a multi-shot handler"))
                                .with_label(e.span, "used after a multi-shot perform")
                                .with_help(format!("`{name}` is used after a perform of a `multi` effect; a multi-shot resume would use it more than once — consume it before the perform")),
                        );
                    }
```

  (Order in the `Expr::Var` arm: increment `uses`, read `crossed`, then emit E0428 if `uses == 2` and E0429 if `crossed`.)

- [ ] **Step 4: Run — both E0429 tests pass.**

Run: `cargo test --test affine affine_across 2>&1 | grep -E "test result|FAILED"`
Expected: both PASS (multi → E0429; one-shot → accepted). If the one-shot case wrongly emits E0429, the pass is keying on "any perform" not `multi` — fix `multi_ops` membership, do not weaken the test.

- [ ] **Step 5: UI fixture + gate + commit.** Create `tests/ui/affine_multi_capture.elya`:

```elya
effect multi Flip { fn flip() -> Bool }
linear type Tok { Tok }
fn use1(t) { match t { Tok -> "ok" } }
pub fn main() {
  let t = Tok
  io.println(handle {
    let _ = flip()
    use1(t)
  } with multi { Flip.flip() -> resume(True) })
}
//~ ERROR[E0429] multi-shot
```

Register `affine_multi_capture` in `tests/ui.rs`.

```bash
cargo fmt --all && sh scripts/check.sh && \
git add src/affine.rs tests/affine.rs tests/ui/affine_multi_capture.elya tests/ui.rs && \
git commit -m "$(printf 'feat(affine): multi-shot capture check (E0429), two-sided teeth\n\nA perform of a multi effect marks in-scope affine bindings crossed; a later use\nis E0429. Two-sided: the SAME shape over a one-shot effect is accepted, proving\nthe check keys on multi-ness, not on any perform. The E0426 hazard, now a\ntype-checked guarantee (scoped, spec §6).\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')" && \
git push origin main
```

---

## Task 4: The scoped-guarantee gap-pin + the exit gate

**Files:**
- Modify: `tests/affine.rs`

- [ ] **Step 1: Pin the callee-duplication gap as CURRENTLY ACCEPTED (output-verified).** In `tests/affine.rs`:

```rust
#[test]
fn callee_duplication_is_currently_accepted_known_gap() {
    // TRACKED SOUNDNESS OBLIGATION (affine-callee-duplication-obligation): the
    // intra-function local first cut counts `pair_use(t)` as ONE use of `t` and
    // does not recurse into `pair_use`, which uses its (generic, non-linear-typed)
    // parameter TWICE. So this program is CURRENTLY ACCEPTED even though `t` is
    // duplicated at runtime. Pinned so a future inter-procedural / param-
    // multiplicity tightening flips this assertion visibly. This is NOT a bug to
    // fix here; the first cut's guarantee is scoped (spec §6).
    let src = "linear type Tok { Tok }\n\
               fn keep(t) { match t { Tok -> \"k\" } }\n\
               fn pair_use(x) { let _ = keep(x)  keep(x) }\n\
               pub fn main() { let t = Tok\n io.println(pair_use(t)) }\n";
    assert!(
        check_source("t.elya", src).is_ok(),
        "known gap: callee-duplication is currently accepted (scoped guarantee): {:?}",
        check_source("t.elya", src)
    );
}
```

- [ ] **Step 2: Run — expect PASS** (the program type-checks; the gap is real and currently unflagged).

Run: `cargo test --test affine callee_duplication_is_currently_accepted 2>&1 | grep -E "test result|FAILED"`
Expected: PASS. If it FAILS (an E0428 fired inside `pair_use`), then params ARE being tracked as affine sites and the gap is partly closed — investigate: if `pair_use`'s `x` is monomorphically linear-typed here it *should* be caught, in which case rewrite the gap-pin to a genuinely polymorphic callee (`x` never constrained to `Tok`) so it exercises the true polymorphic hole; do not delete the pin.

- [ ] **Step 3: The Slice-4d-2 exit gate.**

```bash
cargo fmt --all
sh scripts/check.sh
```
Expected: exit 0; full suite green. Confirm these rows pass: `infer_flags_linear_let_binding_as_affine`, `affine_single_use_is_accepted`, `affine_double_use_is_e0428`, `affine_across_multi_perform_is_e0429`, `affine_across_oneshot_perform_is_accepted`, `callee_duplication_is_currently_accepted_known_gap`, `affine_double_use` + `affine_multi_capture` (UI). Non-linear code and the whole prior suite unchanged.

- [ ] **Step 4: Commit + push (atomic).**

```bash
git add tests/affine.rs && \
git commit -m "$(printf 'test(affine): pin the callee-duplication soundness gap as currently-accepted; Slice-4d-2 exit gate\n\nOutput-verified pin: a program that duplicates an affine value via a generic\ncallee is CURRENTLY ACCEPTED (the tracked affine-callee-duplication-obligation),\nso a future inter-procedural tightening flips it visibly. Slice 4d-2 complete:\nlinear type values are affine (E0428 on second use, E0429 across a multi-perform,\naccepted across a one-shot), guarantee SCOPED and both obligations named. eval/GC\nuntouched.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')" && \
git push origin main
```

### Exit criterion

`linear type` values are affine: a second use is **E0428** (single use accepted); a use across a **`multi`**-perform is **E0429** while the same use across a **one-shot** effect is accepted; non-linear code and the whole prior suite are unchanged; `eval.rs`/GC untouched; `cargo fmt --all` + `sh scripts/check.sh` clean. **The guarantee is SCOPED — the callee-duplication soundness gap is pinned as currently-accepted and the intra-function over-approximation is a named tracked obligation; no blanket no-double-use guarantee is claimed.**

---

## Self-Review

- **Spec coverage:** §2 `linear` marker + affine meaning → Task 1. §3 inference exposure → Task 1 (`infer_with_sites`). §4 the pass → Tasks 2–3. §5 E0428/E0429 → Tasks 2–3. §6 scoped guarantee + obligations → Task 4 pin + exit criterion. §7 pipeline (no `eval`) → honored; layering updated. §8 testing (two-sided + gap-pin) → Tasks 2–4. §9 build order → the 4-task sequence.
- **Two-sided teeth honored:** E0428 (Task 2: `affine_single_use_is_accepted` + `affine_double_use_is_e0428`); E0429 (Task 3: `affine_across_oneshot_perform_is_accepted` + `affine_across_multi_perform_is_e0429`). Neither is one-sided.
- **Gap-pin is a real output-verified test:** Task 4 asserts the callee-duplication program `is_ok()` (currently accepted), with a stop-condition if params turn out to be tracked (rewrite to a genuinely polymorphic callee, don't delete).
- **Placeholder scan:** none — every step is concrete code or an exact edit. Diagnostic strings, the pass structure, and the `lib.rs`/layering wiring are all spelled out.
- **Type/name consistency:** `TypeDecl.is_linear`, `infer_with_sites`, `affine::check(module, affine_sites)`, `multi_ops`, `Ctx.live`, E0428/E0429 used consistently across tasks; `check_source`/`err` in `tests/affine.rs`.
- **Reach bounded:** the only inference change is recording affine sites + `infer_with_sites`; the rest is `src/affine.rs` (new) + `lib.rs` sequencing. No `unify_row`, no HM change, `eval.rs`/GC untouched — stated in Global Constraints and §7.
