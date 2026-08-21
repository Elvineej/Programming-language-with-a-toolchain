# Slice 5a-1: Typed-Inference Output — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make inference *persist* the zonked type it already computes for every expression node — a `Span → Ty` side-table — proven directly by a snapshot corpus, isolating the Core IR arc's one risky pivot (materialized per-node types) before any Core datatype depends on it.

**Architecture:** One new `Infer` field (`node_types`), recorded at a single point via a thin `infer_expr` wrapper, zonked in one final pass after the SCC loop, and exposed through a new `infer_with_types` entry that renders through a *shared* `Names` for cross-node coherence. No Core module, no `eval`/CEK touch, no `unify`/`unify_row` reach. Three tasks in strict order: span-audit gate → the table → the bug-surface corpus.

**Tech Stack:** Rust 2021 (MSRV 1.75); `insta` for snapshots (already a dev-dep); no new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-21-elya-slice-5a1-core-ir-typed-inference-design.md` — the plan argues from it; executors read both.

## Global Constraints

Every task's requirements implicitly include this section.

- **Toolchain (Windows/Bash):** prepend `~/.cargo/bin` to `PATH`; `export CARGO_INCREMENTAL=0` (the incremental cache hangs here).
- **Atomic gate (never commit red):** `cargo fmt --all` then `if sh scripts/check.sh; then git commit …; fi`. `check.sh` fmt-checks and runs `clippy -D warnings` + the full suite; it fails hard.
- **Git:** explicit `git add <paths>` only, never `git add -A`. Push `origin main` after each task. Commit messages end with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.
- **NON-INTERFERENCE (spec §6), enforced:** no new `src` module; `eval.rs`/the CEK machine untouched; `unify`/`unify_row`/`bind`/`instantiate`/`generalize` logic unchanged (recording only *reads* results via `resolve`); `tests/arch/layering.rs` unchanged; the three existing entries (`infer`, `infer_schemes`, `infer_with_sites`) behavior-preserved and **no existing snapshot churns**.
- **SPAN-AUDIT GATE (spec §2), hard:** Task 1 must pass before any table code exists. If it fails, **STOP and re-plan for Shape B (NodeId)** — do not work around it.
- **§10 RESOLVE-SIDE-ONLY (strict hold):** if the row-materialization surface leaks a row var, the *only* in-scope fix is a **read-side completeness fix** in `resolve_row` (resolve a var that was not being resolved), provably **not** a unification-behavior change. Any fix that would touch `unify_row` or change how rows unify is a **STOP-and-re-scope** signal — it means the reach exceeds 5a-1's boundary.
- **STAY-POLYMORPHIC (spec §4):** var-typed nodes are expected and correct; never monomorphize to make a snapshot look tidier.
- **HONESTY LINE:** exit criteria claim only the **feeder** (correct per-node type materialization), never that Core IR exists, lowers, or executes.

## File Structure

- `tests/span_audit.rs` (Create, Task 1) — the hard prerequisite gate: exhaustive expression-span collector + uniqueness/non-EMPTY assertions.
- `src/span.rs` (Modify, Task 2) — derive `PartialOrd, Ord` on `Span` (needed to key a `BTreeMap`; purely additive).
- `src/types.rs` (Modify, Task 2) — `Infer.node_types` field + init; the `infer_expr` record wrapper; `infer_all`'s `want_types` param + final zonk + 4th return; the three callers; `infer_with_types`; one smoke test.
- `tests/typed_inference.rs` + `tests/snapshots/` (Create, Task 3) — the bug-surface corpus (seven surfaces + three cross-cutting invariants).

---

## Task 1: The span-uniqueness audit (hard gate)

**Files:**
- Create: `tests/span_audit.rs`

**Interfaces:**
- Consumes: `elya::parse::parse_module`, `elya::ast::*`, `elya::span::{Span, Spanned}`, `elya::Session`.
- Produces: the GO/NO-GO decision for Shape A. Nothing later consumes its code except the decision.

- [ ] **Step 1: Write the audit** (exhaustive walker — no wildcard arm, so a future `Expr` variant forces a compile error rather than silent under-coverage). Create `tests/span_audit.rs`:

```rust
//! Slice 5a-1 Task 1 — the hard prerequisite gate (spec §2). The typed table is
//! Shape A: keyed on `Span`. That is only sound if no two distinct expression
//! nodes share a span and none is `Span::EMPTY`. This audit proves it over the
//! examples + synthetic-span-prone constructs, and is retained as a regression
//! guard. If it fails, fall back to Shape B (NodeId) — do NOT work around it.

use std::collections::HashMap;
use std::fs;

use elya::ast::*;
use elya::parse::parse_module;
use elya::span::{Span, Spanned};
use elya::Session;

fn collect_block(b: &Block, out: &mut Vec<Span>) {
    for st in b.stmts.iter() {
        match &st.node {
            Stmt::Let { value, .. } => collect_expr(value, out),
            Stmt::Expr(e) => collect_expr(e, out),
        }
    }
    if let Some(t) = &b.tail {
        collect_expr(t, out);
    }
}

fn collect_expr(e: &Spanned<Expr>, out: &mut Vec<Span>) {
    out.push(e.span);
    // EXHAUSTIVE — no `_` arm. Mirrors the exact node class `infer_expr` visits.
    match &e.node {
        Expr::Int(_)
        | Expr::Float(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Unit
        | Expr::Var(_)
        | Expr::Qualified { .. } => {}
        Expr::Call { callee, args } => {
            collect_expr(callee, out);
            for a in args.iter() {
                collect_expr(a, out);
            }
        }
        Expr::Unary { expr, .. } => collect_expr(expr, out),
        Expr::Binary { lhs, rhs, .. } => {
            collect_expr(lhs, out);
            collect_expr(rhs, out);
        }
        Expr::If {
            cond,
            then_block,
            else_block,
        } => {
            collect_expr(cond, out);
            collect_block(&then_block.node, out);
            collect_block(&else_block.node, out);
        }
        Expr::Block(b) => collect_block(b, out),
        Expr::Handle { body, handler } => {
            collect_expr(body, out);
            for c in &handler.clauses {
                collect_expr(&c.node.body, out);
            }
            if let Some(r) = &handler.ret {
                collect_expr(&r.body, out);
            }
        }
        Expr::Resume { arg } => collect_expr(arg, out),
        Expr::Match { scrutinee, arms } => {
            collect_expr(scrutinee, out);
            for arm in arms.iter() {
                collect_expr(&arm.node.body, out);
            }
        }
        Expr::Lambda { body, .. } => collect_block(&body.node, out),
    }
}

fn audit(name: &str, src: &str) -> Vec<String> {
    let (m, diags) = parse_module(&Session::new(), src);
    assert!(diags.is_empty(), "{name}: parse errors (fix the program, not the audit): {diags:?}");
    let mut spans = Vec::new();
    for d in &m.decls {
        if let Decl::Fn(f) = &d.node {
            collect_block(&f.body.node, &mut spans);
        }
    }
    let mut problems = Vec::new();
    if spans.iter().any(|s| *s == Span::EMPTY) {
        problems.push(format!("{name}: an expression node has Span::EMPTY"));
    }
    let mut seen: HashMap<Span, usize> = HashMap::new();
    for s in &spans {
        *seen.entry(*s).or_insert(0) += 1;
    }
    for (s, c) in seen {
        if c > 1 {
            problems.push(format!("{name}: span {}..{} shared by {c} distinct nodes", s.start, s.end));
        }
    }
    problems
}

#[test]
fn expression_spans_are_unique_and_nonempty() {
    let mut problems = Vec::new();
    for path in ["examples/01_hello.elya", "examples/02_arith.elya"] {
        let src = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        problems.extend(audit(path, &src));
    }
    // Constructs most likely to emit synthetic or duplicated spans.
    let corpus: &[(&str, &str)] = &[
        ("bare_ctor_value", "type Option(a) { None, Some(a) }\nfn f() { let g = Some\n g(1) }\n"),
        ("lambda", "fn f() { let g = fn(x) { x }\n g(1) }\n"),
        ("handler", "effect Ask { fn ask() -> Bool }\nfn f() { handle { ask() } with { Ask.ask() -> resume(True) } }\n"),
        ("match_nested", "type List(a) { Nil, Cons(a, List(a)) }\nfn f(xs) { match xs { Nil -> 0  Cons(_, ys) -> 1 } }\n"),
        ("linear", "linear type Tok { Tok }\nfn f() { let t = Tok\n t }\n"),
        ("nested_calls", "fn f(a) { a }\nfn g() { f(f(f(1))) }\n"),
    ];
    for (name, src) in corpus {
        problems.extend(audit(name, src));
    }
    assert!(
        problems.is_empty(),
        "SPAN AUDIT FAILED — fall back to Shape B (NodeId); do NOT work around:\n{}",
        problems.join("\n")
    );
}
```

- [ ] **Step 2: Run the audit.**

Run: `cargo test --test span_audit 2>&1 | grep -E "test result|FAILED|SPAN AUDIT"`
Expected: **PASS.** If any corpus program fails to *parse*, simplify that program to a known-parsing shape that still exercises the same construct (do not weaken the walker). **If the audit itself FAILS** (a duplicate or EMPTY span): STOP. Report the failing spans and re-plan Tasks 2–3 for Shape B (`NodeId` on AST nodes) — this is a re-scope, not a fix.

- [ ] **Step 3: Atomic gate + commit + push.**

```bash
cargo fmt --all
if sh scripts/check.sh; then
  git add tests/span_audit.rs
  git commit -m "$(printf 'test(types): span-uniqueness audit — Shape A prerequisite gate (5a-1)\n\nExhaustive expression-span collector over examples + synthetic-span-prone\nconstructs, asserting no duplicate and no Span::EMPTY among typed nodes.\nGates the per-node type table (span-keyed); a failure means fall back to\nShape B (NodeId). Retained as a regression guard. No src change.\n\nCo-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>')"
  git push origin main
fi
```

---

## Task 2: The typed table (Shape A) + `infer_with_types`

**Files:**
- Modify: `src/span.rs` (derive `Ord`)
- Modify: `src/types.rs` (`Infer.node_types`; the record wrapper; `infer_all` threading; `infer_with_types`; a smoke test)

**Interfaces:**
- Consumes: `resolve` ([types.rs:241](../../../src/types.rs#L241)), `write_ty` + `Names` ([types.rs:581](../../../src/types.rs#L581),[607](../../../src/types.rs#L607)), `infer_all` ([types.rs:1637](../../../src/types.rs#L1637)).
- Produces: `pub fn infer_with_types(&Session, &Module) -> (Vec<Diagnostic>, BTreeMap<Span, String>)` — the span-sorted, rendered table Task 3 consumes.

- [ ] **Step 1: Derive `Ord` on `Span`** (a `BTreeMap<Span, _>` needs it; field order `(start, end)` gives the right lexicographic order). In `src/span.rs`, change:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
```
to:
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
```

- [ ] **Step 2: Add the `node_types` field.** In `src/types.rs`, in `pub struct Infer` after the `affine_sites` field ([types.rs:166](../../../src/types.rs#L166)):

```rust
    /// Every expression node's inferred type, zonked at the end of inference
    /// (Slice 5a-1). The Core IR arc's feeder; recorded unconditionally, exposed
    /// only through `infer_with_types`. Keyed by span (Shape A; the span-audit
    /// gate proves the key is unique).
    node_types: HashMap<Span, Ty>,
```
and in `Infer::new()` after `affine_sites: HashSet::new(),` ([types.rs:180](../../../src/types.rs#L180)):
```rust
            node_types: HashMap::new(),
```

- [ ] **Step 3: Record at a single point via an `infer_expr` wrapper.** In `src/types.rs`, rename the existing method's signature line ([types.rs:846](../../../src/types.rs#L846)) from `pub fn infer_expr` to `fn infer_expr_inner` (body unchanged — its internal recursive `self.infer_expr(...)` calls now route through the new wrapper), then add the wrapper immediately above it:

```rust
    /// Type an expression and record its (pre-zonk) type against its span. The
    /// single record point (Slice 5a-1); the final zonk pass runs in `infer_all`.
    pub fn infer_expr(&mut self, e: &Spanned<Expr>, env: &mut TyEnv, amb: RowVar) -> Ty {
        let ty = self.infer_expr_inner(e, env, amb);
        self.node_types.insert(e.span, ty.clone());
        ty
    }
```

- [ ] **Step 4: Thread `want_types` through `infer_all` + final zonk pass.** In `src/types.rs`:
  - Change the signature ([types.rs:1637](../../../src/types.rs#L1637)) to:
    ```rust
    fn infer_all(
        module: &Module,
        want_types: bool,
    ) -> (
        Vec<(String, String)>,
        Vec<Diagnostic>,
        HashSet<Span>,
        BTreeMap<Span, Ty>,
    ) {
    ```
  - Replace the final return ([types.rs:1845](../../../src/types.rs#L1845)) `(schemes_out, inf.diags, inf.affine_sites)` with the zonk-then-return:
    ```rust
        // Record-then-zonk (spec §3): resolve every recorded node type ONCE here,
        // after all SCC groups are solved — never at record-time. Skipped unless a
        // caller wants the table, so the hot inference paths pay nothing.
        let node_types: BTreeMap<Span, Ty> = if want_types {
            inf.node_types
                .iter()
                .map(|(s, t)| (*s, inf.resolve(t)))
                .collect()
        } else {
            BTreeMap::new()
        };
        (schemes_out, inf.diags, inf.affine_sites, node_types)
    ```
  - Update the three existing callers to pass `false` and ignore the 4th element (behavior-preserving):
    - `infer_schemes` ([types.rs:1622](../../../src/types.rs#L1622)): `let (schemes, diags, _sites, _typed) = infer_all(module, false);`
    - `infer_with_sites` ([types.rs:1630](../../../src/types.rs#L1630)): `let (_schemes, diags, sites, _typed) = infer_all(module, false);`
    - `infer` is unchanged (it delegates to `infer_schemes`).

- [ ] **Step 5: Add `infer_with_types`** (rendered through ONE shared `Names` for cross-node coherence — `display_ty` seeds a fresh `Names` per call, so per-node rendering would break coherence). In `src/types.rs`, next to `infer_with_sites`:

```rust
/// Inference plus the per-node type table (Slice 5a-1): every expression node's
/// zonked type, rendered span-sorted through one shared `Names` so a variable
/// shared across nodes renders identically. The Core IR arc's feeder; 5a-2's
/// lowering consumes the underlying `node_types` directly.
pub fn infer_with_types(
    _session: &Session,
    module: &Module,
) -> (Vec<Diagnostic>, BTreeMap<Span, String>) {
    let (_schemes, diags, _sites, typed) = infer_all(module, true);
    let mut names = Names::default();
    let mut rendered: BTreeMap<Span, String> = BTreeMap::new();
    for (span, ty) in &typed {
        let mut out = String::new();
        write_ty(ty, &mut names, &mut out);
        rendered.insert(*span, out);
    }
    (diags, rendered)
}
```

- [ ] **Step 6: Smoke test** (Task 2's independently-testable deliverable — the plumbing works and the table is zonked). In `src/types.rs`'s `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn infer_with_types_produces_a_zonked_table() {
        let (m, pd) = crate::parse::parse_module(&Session::new(), "fn add1(n) { n + 1 }\n");
        assert!(pd.is_empty(), "{pd:?}");
        let (diags, table) = infer_with_types(&Session::new(), &m);
        assert!(diags.is_empty(), "{diags:?}");
        // A concrete `Int` node proves recording + the final zonk both ran (a
        // pre-zonk node would render as a variable, not `Int`).
        assert!(table.values().any(|v| v == "Int"), "no zonked Int node: {table:?}");
    }
```

- [ ] **Step 7: Confirm behavior-preservation, then atomic gate + commit + push.**

Run: `cargo test 2>&1 | grep -E "test result|FAILED|churn"` — full suite green, **no existing snapshot churns**, layering test still passes.
```bash
cargo fmt --all
if sh scripts/check.sh; then
  git add src/span.rs src/types.rs
  git commit -m "$(printf 'feat(types): persist per-node types — infer_with_types (5a-1 table)\n\nThe Core IR arc feeder. A single record wrapper over infer_expr stores every\nexpression node type; infer_all zonks the table in one final pass (gated by\nwant_types so the hot paths pay nothing) and returns it; infer_with_types\nrenders it span-sorted through one shared Names for cross-node coherence.\nSpan gains Ord for BTreeMap keying. The three existing entries are\nbehavior-preserved; no unify/unify_row reach; eval/CEK untouched.\n\nCo-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>')"
  git push origin main
fi
```

---

## Task 3: The bug-surface corpus (the deliverable)

**Files:**
- Create: `tests/typed_inference.rs`, `tests/snapshots/` (insta-generated)

**Interfaces:**
- Consumes: `elya::types::{infer_with_types, infer_with_sites}`, `elya::parse::parse_module`, `elya::span::Span`, `elya::Session`.

The **targeted assertions are the machine-checkable teeth** (they auto-fail on a regression); the **snapshot is the human-auditable record**, reviewed once against spec §5 then frozen as a regression guard.

- [ ] **Step 1: Helpers + the seven surfaces + the three invariants.** Create `tests/typed_inference.rs`:

```rust
//! Slice 5a-1 Task 3 — the bug-surface corpus (spec §5). Each surface is pinned
//! by a snapshot (human-auditable) AND targeted assertions (machine teeth) that
//! fail loudly on lingering vars, wrong generalization, or unresolved rows.

use std::collections::BTreeMap;

use elya::parse::parse_module;
use elya::span::Span;
use elya::types::{infer_with_sites, infer_with_types};
use elya::Session;

fn table_of(src: &str) -> BTreeMap<Span, String> {
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (diags, table) = infer_with_types(&Session::new(), &m);
    assert!(diags.is_empty(), "unexpected type errors: {diags:?}");
    table
}

fn render(src: &str, table: &BTreeMap<Span, String>) -> String {
    let mut out = String::new();
    for (span, ty) in table {
        let slice = &src[span.start as usize..span.end as usize];
        out.push_str(&format!("{}..{} `{}` : {}\n", span.start, span.end, slice, ty));
    }
    // Invariant: no internal inference token leaks into a materialized type.
    for bad in ["%r", "%e", "%s", "%t", "%v", "%row"] {
        assert!(!out.contains(bad), "leaked internal token {bad}:\n{out}");
    }
    out
}

/// A rendered type is a bare type variable iff it is `[a-z][0-9]*` (base types
/// are capitalized, `fn(...)`/`Con(...)` are not single-letter).
fn is_var(s: &str) -> bool {
    let mut cs = s.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_lowercase()) && cs.all(|c| c.is_ascii_digit())
}

// --- Surface 1: monomorphic fn — every node concrete, no variable ------------
#[test]
fn surface1_monomorphic_fn() {
    let src = "fn add1(n) { n + 1 }\n";
    let t = table_of(src);
    assert!(t.values().all(|v| !is_var(v)), "monomorphic program has a var-typed node: {t:?}");
    assert!(t.values().any(|v| v == "Int"), "expected an Int node: {t:?}");
    insta::assert_snapshot!(render(src, &t));
}

// --- Surface 2: polymorphic fn — the body node is a variable ------------------
#[test]
fn surface2_polymorphic_fn_has_var_node() {
    let src = "fn id(x) { x }\n";
    let t = table_of(src);
    assert!(t.values().any(|v| is_var(v)), "polymorphic id should have a var-typed node: {t:?}");
    insta::assert_snapshot!(render(src, &t));
}

// --- Surface 3: use-site instantiation — two distinct concrete instances ------
#[test]
fn surface3_use_site_instantiation() {
    let src = "fn id(x) { x }\nfn u() { let a = id(5)  let b = id(True)  0 }\n";
    let t = table_of(src);
    assert!(t.values().any(|v| v == "fn(Int) -> Int"), "missing Int instance: {t:?}");
    assert!(t.values().any(|v| v == "fn(Bool) -> Bool"), "missing Bool instance: {t:?}");
    insta::assert_snapshot!(render(src, &t));
}

// --- Surface 4: effectful arrow — row materialized WITH args (hard req) -------
#[test]
fn surface4_row_materialization() {
    // `worker` performs State(Int); referenced as a value, its arrow carries the
    // resolved row {State(Int)} onto a node. Exercises resolve_row's arg path.
    let src = "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
               fn worker() { set(1) }\n\
               fn use_it() { let g = worker  g() }\n";
    let t = table_of(src);
    assert!(
        t.values().any(|v| v.contains("State(Int)")),
        "row not materialized with its Int arg (resolve_row leak?): {t:?}"
    );
    insta::assert_snapshot!(render(src, &t));
}

// --- Surface 5: let-generalized lambda (value restriction) --------------------
#[test]
fn surface5_value_restriction_lambda() {
    let src = "fn demo() { let f = fn(x) { x }  let a = f(1)  let b = f(True)  0 }\n";
    let t = table_of(src);
    assert!(t.values().any(|v| v == "fn(Int) -> Int"), "f not instantiated at Int: {t:?}");
    assert!(t.values().any(|v| v == "fn(Bool) -> Bool"), "f not instantiated at Bool: {t:?}");
    assert!(t.values().any(|v| is_var(v)), "the lambda body should be a var: {t:?}");
    insta::assert_snapshot!(render(src, &t));
}

// --- Surface 6: match --------------------------------------------------------
#[test]
fn surface6_match() {
    let src = "type Option(a) { None, Some(a) }\n\
               fn m(o) { match o { None -> 0  Some(x) -> x } }\n";
    let t = table_of(src);
    assert!(t.values().any(|v| v == "Int"), "match result should be Int: {t:?}");
    insta::assert_snapshot!(render(src, &t));
}

// --- Surface 7: linear binding — table agrees with affine_sites ---------------
#[test]
fn surface7_linear_binding_agrees_with_affine_sites() {
    let src = "linear type Tok { Tok }\nfn lin() { let t = Tok\n t }\n";
    let (m, _) = parse_module(&Session::new(), src);
    let (_d, sites) = infer_with_sites(&Session::new(), &m);
    let t = table_of(src);
    assert!(!sites.is_empty(), "expected an affine site");
    for s in &sites {
        assert_eq!(
            t.get(s).map(String::as_str),
            Some("Tok"),
            "affine site {}..{} not typed Tok in the table: {t:?}",
            s.start,
            s.end
        );
    }
    insta::assert_snapshot!(render(src, &t));
}

// --- Invariant A: record-then-zonk ordering ----------------------------------
#[test]
fn record_then_zonk_ordering() {
    // `y` is a fresh var when first recorded, then forced to Int by `y + 1`. If
    // the table were zonked at record-time, early nodes would be vars.
    let src = "fn ord(x) { let y = x  y + 1 }\n";
    let t = table_of(src);
    assert!(t.values().all(|v| !is_var(v)), "record-then-zonk violated (a node stayed a var): {t:?}");
    assert!(t.values().any(|v| v == "Int"), "expected Int nodes: {t:?}");
}

// --- Invariant B: cross-node variable coherence (shared Names) ----------------
#[test]
fn cross_node_variable_coherence() {
    // Two DISTINCT params → two distinct vars in separate nodes. Under a shared
    // Names they render as two distinct letters (a, b); fresh-per-node Names would
    // wrongly render both as `a`.
    let src = "fn two(x, y) { let p = x  let q = y  0 }\n";
    let t = table_of(src);
    let vars: std::collections::HashSet<&String> = t.values().filter(|v| is_var(v)).collect();
    assert!(vars.len() >= 2, "shared Names should give >=2 distinct var letters: {t:?}");
}
```

- [ ] **Step 2: Run; review the pending snapshots against spec §5; accept.**

Run: `cargo test --test typed_inference 2>&1 | grep -E "test result|FAILED"` — the targeted assertions must PASS; the snapshots come up **pending** on first run.
Then review each `tests/snapshots/*.snap.new` against the spec §5 expected shapes (concrete where it should be concrete, a variable where it should be polymorphic, `{State(Int)}` fully resolved) and accept: `cargo insta accept` (or review + rename `.snap.new` → `.snap`). Re-run: all green.

If **surface 4 leaks a row var** (e.g. `State(a)` / a `%` token): this is the spec §10 escape hatch — apply the **read-side** completeness fix to `resolve_row` only (resolve a var that was not being resolved). If any fix would touch `unify_row` or change how rows unify, **STOP and re-scope** (the reach exceeds 5a-1).

- [ ] **Step 3: Atomic gate + commit + push.**

```bash
cargo fmt --all
if sh scripts/check.sh; then
  git add tests/typed_inference.rs tests/snapshots
  git commit -m "$(printf 'test(types): bug-surface corpus for the typed table (5a-1 deliverable)\n\nSeven surfaces pinned by snapshot + targeted teeth: monomorphic (no var),\npolymorphic (var node), use-site instantiation (distinct instances),\neffectful-arrow row materialization (State(Int) resolved incl args),\nvalue-restriction lambda, match, linear binding (table agrees with\naffine_sites). Plus record-then-zonk ordering and cross-node variable\ncoherence (shared Names). Proves the feeder; Core IR is not claimed.\n\nCo-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>')"
  git push origin main
fi
```

### Exit criterion

`sh scripts/check.sh` exits 0 with the full suite green, including: `span_audit::expression_spans_are_unique_and_nonempty`, `types::tests::infer_with_types_produces_a_zonked_table`, all seven `surface*` rows, `record_then_zonk_ordering`, `cross_node_variable_coherence`. The three existing entries and every prior snapshot are unchanged; no new `src` module; `eval`/CEK and `unify`/`unify_row` untouched; layering test green. Claims cover the **feeder** only — Core IR is not built, does not lower, does not execute.

---

## Self-Review

- **Spec coverage:** §2 span-audit gate → Task 1 (retained regression test, Shape B stop-condition). §3 table shape + record-then-zonk → Task 2 (Steps 2–4) + Task 3 `record_then_zonk_ordering`. §4 stay-polymorphic → surfaces 2/5 assert var-typed nodes are accepted, never monomorphized. §5 corpus (7 surfaces + 3 invariants) → Task 3. §6 non-interference → Global Constraints + Task 2 Step 7. §7 pipeline (`infer_with_types`, shared `Names`, `BTreeMap`) → Task 2 Steps 4–5. §10 resolve-side-only → Global Constraints + Task 3 Step 2.
- **Placeholder scan:** none — every step has runnable code. The one deliberate contingency (Task 3 Step 2's resolve_row fix) is bounded by the resolve-side-only rule with an explicit STOP condition, not a vague "handle errors."
- **Type consistency:** `infer_with_types -> (Vec<Diagnostic>, BTreeMap<Span, String>)` is used identically in Task 2 Step 5 and Task 3's helpers; `infer_all`'s 4-tuple return matches all three updated callers + `infer_with_types`; `Span: Ord` (Task 2 Step 1) is what makes every `BTreeMap<Span, _>` compile; `is_var`/`render`/`table_of` signatures are consistent across Task 3.
- **Ordering soundness:** the audit (Task 1) precedes any table code, so Shape A is only built on a proven-unique key; `Span: Ord` (Task 2 Step 1) precedes its first `BTreeMap` use (Step 4).
