# Elya Slice 4a — Parametric ADTs & Pattern Matching Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add parametric algebraic data types (`type List(a) { Nil, Cons(a, List(a)) }`) and total pattern matching (`match`), with Maranget exhaustiveness/useless-arm checking and witness-quality diagnostics.

**Architecture:** Vertical-slice discipline *inside* 4a — the real risk is the four-subsystem integration surface, not any single task. A **nullary monomorphic ADT runs end-to-end (lex→parse→resolve→type→eval) by Task 2**; every later task *widens that working path* (parameters+payloads → TCE → literals → exhaustiveness → CLI) rather than stacking untested layers. Constructors are ordinary polymorphic `Scheme`s (so "generics" reuses HM); `match` uses a transient CEK frame (so TCE is preserved); effect-free ADT programs rejoin the `cek == tree` cross-check.

**Tech Stack:** Rust 2021; existing deps only (`logos`, `ariadne`, `insta`). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-08-elya-slice-4a-adts-design.md` (approved). "spec §X" refers there.

## Global Constraints

- **Rust edition 2021**, toolchain `stable`, MSRV 1.75. No new dependencies.
- **Module layer map (pinned):** `span/diag=0, lex=1, ast=1, parse=2, resolve=3, types=4, core=5, eval=6, main=7`. `tests/arch/layering.rs` enforces it. No new module.
- **Diagnostic codes:** `E043x` reserved for ADTs/patterns — `E0430` non-exhaustive (error, witness), `E0431` useless arm (**warning**), `E0432` unknown constructor/type (resolve error), `E0433` unapplied/under-applied constructor (error, 4b-pointing). Type/arity mismatches reuse `E0400`/`E0402`. Diagnostic discipline: named identifiers, zonked types, **no `%`-token** (the E0420–E0426 standard).
- **Constructors are `Upper`, functions/vars `lower`** — the lexer already distinguishes (`TokenKind::Upper`/`Lower`).
- **Ordering discipline:** `cargo fmt --all` first, then `sh scripts/check.sh`, commit **only** on gate exit 0. The `scripts/githooks/pre-commit` fmt guard is installed. PATH prepends `~/.cargo/bin` (project memory). Push to `origin/main` after each commit.
- **Scratch/debug work runs in the session scratchpad, never under repo `examples/`** (project memory: a stray `rm` there has twice deleted tracked fixtures). For a debug render, add a throwaway `#[test] -- --nocapture`, not `cargo run --example`.
- **Preservation gate (every task):** the entire existing suite stays green — Slice-3 effect goldens, `tce.rs` (`K_MAX = 3`) + effect-TCE (`K_MAX_EFF = 4`), E042x fixtures, cross-check.

---

## File Structure

| File | Responsibility | 4a change |
|---|---|---|
| `src/ast.rs` | AST + pretty | `Decl::Type(TypeDecl)`, `VariantDecl`; `Expr::Match`, `MatchArm`, `Pattern`; pretty arms |
| `src/parse.rs` | parser | `type` declarations, `match` expressions, pattern grammar |
| `src/resolve.rs` | name resolution | register types/constructors; resolve ctor names + pattern binding; `E0432`; saturation → `E0433` |
| `src/types.rs` | HM inference | `Ty::Con` (+ unify/resolve/free-vars/subst/display); constructor schemes (two-pass); `elaborate_ty` for ADTs; pattern typing; `match` typing |
| `src/exhaust.rs` | **new** — Maranget usefulness | exhaustiveness (`E0430` + witness), useless-arm (`E0431`); own module, called from the pipeline after typing |
| `src/eval.rs` | evaluators | `Value::Ctor`; construction + `match` on tree + CEK; `MatchK` frame; constructor arity table |
| `src/lib.rs` | pipeline | run `exhaust::check` after `types::infer`; warnings stay non-fatal |
| `src/main.rs` | CLI | render `Severity::Warning` to stderr on a successful compile |
| `tests/adt.rs` | **new** — ADT goldens (type+run, cross-checked) | new |
| `tests/tce_match.rs` | **new** — TCE-through-match (two-sided) | new |
| `tests/ui/*.elya` | E0430/E0431/E0432/E0433 fixtures | new fixtures |

**Layer note:** `exhaust` sits at layer 4-5 (consumes `types`, feeds diagnostics). Add it to the pinned layer map in `tests/arch/layering.rs` (e.g. `exhaust = 5`) as part of Task 6.

---

## Task 1: AST nodes + parser (`type`, `match`, patterns)

**Files:**
- Modify: `src/ast.rs` (nodes + pretty), `src/parse.rs`
- Test: inline `#[cfg(test)]` in `src/ast.rs`, `src/parse.rs`

**Interfaces (produces — later tasks rely on these exact shapes):**
```rust
// ast.rs — reuses Slice-3 TypeAnn for field/param types
pub struct TypeDecl { pub name: String, pub params: Vec<String>, pub variants: Vec<Spanned<VariantDecl>> }
pub struct VariantDecl { pub name: String, pub fields: Vec<Spanned<TypeAnn>> }
// enum Decl gains: Type(TypeDecl)
pub struct MatchArm { pub pat: Spanned<Pattern>, pub body: Rc<Spanned<Expr>> }
#[derive(Clone, Debug, PartialEq)]
pub enum Pattern { Wild, Var(String), Ctor { name: String, args: Vec<Spanned<Pattern>> } } // Lit added in Task 5
// enum Expr gains: Match { scrutinee: Rc<Spanned<Expr>>, arms: Rc<[Spanned<MatchArm>]> }
```
Construction reuses `Expr::Call`/`Expr::Var` (no new node). `Rc<[…]>` on `arms` so the CEK `MatchK` frame captures it cheaply.

- [ ] **Step 1: Write the failing pretty test** (in `ast.rs` tests):
```rust
#[test]
fn pretty_prints_type_decl_and_match() {
    // type Opt(a) { None, Some(a) } ; fn f(o) { match o { None -> 0  Some(x) -> x } }
    // build the AST directly and assert:
    // "(module (type Opt (None) (Some a)) (fn f (o) (block (match o (None 0) (Some (x) x)))))"
}
```
- [ ] **Step 2: Run to verify it fails** — `cargo test --lib ast::tests::pretty_prints_type_decl_and_match` (types not found).
- [ ] **Step 3: Add the AST nodes + pretty arms.** In `ast.rs`: the structs above (`#[derive(Clone, Debug, PartialEq)]`), `Decl::Type`, `Expr::Match`. Pretty: `type` → `(type NAME (V) (V FIELD…))`; `match` → `(match SCRUT (PAT BODY)…)`; pattern → `NAME`/`_`/`(NAME p…)`. Add placeholder/skip arms wherever `Decl`/`Expr`/`Pattern` are matched downstream so the crate still compiles (resolve/types/eval get `Decl::Type => {}` and `Expr::Match => <todo error>` provisional arms, tightened in Task 2).
- [ ] **Step 4: Run the pretty test** — passes.
- [ ] **Step 5: Write failing parser tests** (in `parse.rs` tests):
```rust
#[test]
fn parses_type_decl() {
    let src = "type List(a) {\n  Nil,\n  Cons(a, List(a))\n}\n";
    // assert pretty == "(module (type List (Nil) (Cons a (List a))))"
}
#[test]
fn parses_match() {
    let src = "fn f(o) {\n  match o {\n    None -> 0\n    Some(x) -> x\n  }\n}\n";
    // assert pretty contains "(match o (None 0) (Some (x) x))"
}
```
- [ ] **Step 6: Implement the parser.** `module()` gains `KwType => self.type_decl()` (add a `type` keyword to the lexer if absent — check `src/lex.rs`; likely add `TokenKind::KwType`). `type_decl()`: name (`Upper`), optional `(p, …)` params (`Lower`), `{ Variant, … }` where each variant is `Upper` + optional `(TypeAnn, …)` (reuse Slice-3 `type_ann()`). `atom()` gains `KwMatch => self.match_expr()`; `match_expr()`: scrutinee `expr(0)`, `{ arm… }`, each arm `pattern() -> body expr(0)`. `pattern()`: `Upper name` → `Ctor{name, args?}` (optional `(pat,…)`); `Lower name` → `Var(name)` (but `_` → `Wild`); parse nested patterns recursively. (Literal patterns are **not** parsed yet — Task 5.)
- [ ] **Step 7: Run parser tests + fmt + gate** — `cargo fmt --all && sh scripts/check.sh`; all green (existing suite unaffected — new nodes only appear in new tests).
- [ ] **Step 8: Commit + push** (`feat(ast,parse): type declarations, match expressions, and pattern grammar`).

---

## Task 2: Nullary ADT runs end-to-end (resolve + minimal types + eval)

**The vertical-thread milestone:** `type Bool2 { T, F }` + a `match` on it type-checks and runs on **both** evaluators, cross-checked — before parameters, payloads, literals, or exhaustiveness exist.

**Files:** Modify `src/resolve.rs`, `src/types.rs`, `src/eval.rs`. Test: `tests/adt.rs` (new), inline tests.

**Interfaces:**
```rust
// types.rs
// Ty gains: Con(String, Vec<Ty>)         // nullary here: Con("Bool2", [])
// A constructor table built from TypeDecls, inserted into TyEnv as Schemes:
//   T : Scheme{ vars:[], row_vars:[], ty: Con("Bool2", []) }
// eval.rs
// Value gains: Ctor(String, Vec<Value>)  // T => Ctor("T", [])
// type Ctors<'a> = HashMap<&'a str, usize>;  // constructor name -> arity, threaded like fn_table
```

- [ ] **Step 1: Write the failing end-to-end test** (`tests/adt.rs`):
```rust
use elya::{run_source, check_source};
#[test]
fn nullary_adt_runs_end_to_end() {
    let src = "type Bool2 { T, F }\n\
               fn to_int(b) { match b { T -> 1  F -> 0 } }\n\
               pub fn main() { let _ = to_int(T)  io.println(\"ok\") }\n";
    assert!(check_source("t.elya", src).is_ok(), "{:?}", check_source("t.elya", src));
    assert_eq!(run_source("t.elya", src).unwrap(), "ok\n");
}
```
- [ ] **Step 2: Run to verify it fails** — currently `Decl::Type`/`Expr::Match` hit the Task-1 placeholder.
- [ ] **Step 3: Resolver.** In `resolve::check`: gather **type + constructor names** from `Decl::Type` (constructor `name`s into a `ctors: HashSet<String>`; type names into `types: HashSet<String>`). `resolves_var` treats a constructor name as resolved. `check_expr` `Expr::Match { scrutinee, arms }`: check the scrutinee, then per arm push a scope, **bind the pattern's variables** (walk the pattern collecting `Var` names; a `Ctor` pattern's `name` must be a known constructor else `E0432`), check the body, pop. Add `fn pattern_binders(&self, pat, scope, out)` that also validates constructor names. `Decl::Type` bodies need no resolution (signatures only).
- [ ] **Step 4: Types.** Add `Ty::Con(String, Vec<Ty>)`; fan it out mechanically (mirror `Ty::Tuple`) through `resolve`, `occurs`, `unify` (arm: `(Con(n1,a1),Con(n2,a2)) if n1==n2 && a1.len()==a2.len() => unify args pairwise; else E0400`), `free_vars`, `free_row_vars`, `subst_vars`, `write_ty` (display `Bool2`, or `List(Int)` later). In `infer_schemes`, **before typing bodies**, build constructor `Scheme`s from `Decl::Type` and `env.insert` them (two-pass; nullary here: `T`/`F` → `Scheme{ty: Con("Bool2", [])}`). Add `check_pattern(&mut self, pat, expected, bindings)` handling `Wild`/`Var`/`Ctor` (nullary: instantiate the ctor scheme → result `R`, unify `R ~ expected`, no sub-patterns). Add the `Expr::Match` arm: infer scrutinee `S`, fresh `R`, per arm `check_pattern(pat, S)` → bind in a pushed scope → infer body → unify with `R`; thread the ambient `amb`.
- [ ] **Step 5: Eval.** Add `Value::Ctor(String, Vec<Value>)` (+ its `PartialEq` arm — `Ctor(n1,a1)==Ctor(n2,a2)` iff name+args equal). Build a `Ctors` arity table (`name -> arity`) in `run_module`, thread like `fns`/`ops` (tree + cek). Nullary construction: `Var(name)` where `name ∈ ctors` (arity 0) → `Value::Ctor(name, [])`. `Expr::Match`: **tree** — eval scrutinee, `match_pattern` each arm in order, eval the first match's body with bindings; **cek** — eval scrutinee under `Frame::MatchK { arms: Rc<[…]>, env }`, on return `match_pattern` and eval the winning body **in the match's continuation slot** (reuse `rest` — tail position). `fn match_pattern(v: &Value, p: &Pattern) -> Option<Vec<(String, Value)>>` (Wild/Var/Ctor). A fall-through (no arm matches) is a defensive `E0300` (Task 6 makes it statically unreachable).
- [ ] **Step 6: Run the end-to-end test + fmt + gate** — the Bool2 program type-checks and runs on both evaluators. Add a cross-check assertion (`tests/adt.rs` also runs it via the tree-walker and asserts equal output), or rely on `tests/crosscheck.rs` picking it up once ADT programs are added there in Task 7.
- [ ] **Step 7: Commit + push** (`feat: nullary ADTs run end-to-end (Ty::Con, Value::Ctor, match on tree+cek)`).

---

## Task 3: Parametric ADTs — payloads + type parameters + saturation (`E0433`)

**Widen the working thread to the real data model:** `Option(a)`, `List(a)`, `Tree(a)` — constructors with fields and type parameters.

**Files:** Modify `src/types.rs` (elaboration + polymorphic ctor schemes + pattern recursion), `src/resolve.rs` (saturation), `src/eval.rs` (n-ary construction). Test: `tests/adt.rs`, `tests/ui/unapplied_ctor.elya`.

**Interfaces:**
```rust
// types.rs — elaborate a field/result TypeAnn under a param environment:
//   elaborate_ty(ann, &param_env: HashMap<String, Ty>, &known: HashMap<String, usize>) -> Ty
//   lowercase name in param_env -> its Ty::Var; Upper name -> Ty::Con(name, elaborated args)
//   (validate arity against `known` type-name -> arity; unknown/bad-arity -> E0432/E0400)
```

- [ ] **Step 1: Write failing goldens** (`tests/adt.rs`):
```rust
#[test]
fn list_length_and_option() {
    let src = "type Option(a) { None, Some(a) }\n\
               type List(a) { Nil, Cons(a, List(a)) }\n\
               fn length(xs) { match xs { Nil -> 0  Cons(_, t) -> 1 + length(t) } }\n\
               fn unwrap_or(o, d) { match o { None -> d  Some(x) -> x } }\n\
               pub fn main() {\n\
                 let xs = Cons(10, Cons(20, Nil))\n\
                 let n = length(xs)\n\
                 io.println(unwrap_or(Some(\"hi\"), \"default\"))\n\
               }\n";
    assert!(check_source("t.elya", src).is_ok());
    assert_eq!(run_source("t.elya", src).unwrap(), "hi\n");
}
```
(Also assert `length` infers `forall a. fn(List(a)) -> Int` via `infer_schemes` if convenient.)
- [ ] **Step 2: Run to verify it fails** — parametric ctor schemes / field typing not yet implemented.
- [ ] **Step 3: Polymorphic constructor schemes + elaboration.** In the two-pass registration: for each `TypeDecl`, allocate a fresh type var per `param` (a `param_env: name -> Ty::Var`), then for each variant build its `Scheme` — `vars` = the param var ids; `ty` = `Fn([elaborate(field)…], EffectRow::pure(), Con(type_name, [param vars]))` for n-ary, or `Con(type_name, [param vars])` for nullary. Implement `elaborate_ty` (Task-3 Interfaces): base names → `Base`; param names → their `Ty::Var`; `Upper(args)` → `Con` (validate arity against the known-types table; unknown → `E0432`). Recursive types work because the known-types table is built first.
- [ ] **Step 4: Pattern typing into fields.** Extend `check_pattern`'s `Ctor` arm: instantiate the ctor scheme (freshening params), read off field types `[F…]` and result `R`; unify `R ~ expected`; require `args.len() == fields.len()` (else `E0402` "constructor `Cons` expects 2 fields, found 1"); recurse `check_pattern(args[i], F[i])`.
- [ ] **Step 5: n-ary construction on the machines.** Add `CalleeSlot::Ctor { name }` (cek) recognized in the `Call` arm when the callee is `Var(name)` with `name ∈ ctors`; `apply_callee` for a `Ctor` builds `Value::Ctor(name, args)`. Tree-walker: analogous in its `Call` path. (Nullary bare-ctor already works from Task 2.)
- [ ] **Step 6: Saturation → `E0433`.** A bare or partially-applied **n-ary** constructor is an error, not a value. Detect it: in the resolver (or `infer_call`/`infer_expr`), an `Expr::Var(name)` where `name ∈ ctors` **and** arity > 0 (not the callee of a `Call` with matching arg count) ⇒ `E0433`:
```
error[E0433]: constructor `Some` needs 1 argument
  = unapplied constructors become first-class function values in Slice 4b; apply it here, e.g. `Some(x)`
```
Fixture `tests/ui/unapplied_ctor.elya` (`fn f() { Some }` or `map(Some, xs)`-shaped) with `//~ ERROR[E0433] needs 1 argument`. Register in `tests/ui.rs`.
- [ ] **Step 7: Run goldens + E0433 fixture + fmt + gate** — Option/List/Tree type-check, run, cross-check; E0433 message is the clear one.
- [ ] **Step 8: Commit + push** (`feat(types,eval): parametric ADTs — payloads, type params, applied unification; E0433 for unapplied constructors`).

---

## Task 4: TCE through `match` — two-sided teeth

**Files:** Create `tests/tce_match.rs`. (Conditional: `src/eval.rs`, only if measurement shows growth.)

The `MatchK` frame is transient (pushed to evaluate the scrutinee, popped before the body, which runs in the match's continuation slot), so a tail-position `match` should already be bounded. **Measure-first, same discipline as `tce_effects.rs`.**

- [ ] **Step 1: Write the bounded + grow tests** (`tests/tce_match.rs`):
```rust
fn run_peak(src: &str) -> (String, usize) { /* check_source ok; eval::run_module; (output, peak_kont_depth) */ }
const K_MAX_MATCH: usize = /* pinned from first measurement */;

#[test]
fn tail_fold_over_match_is_bounded() {
    // range builds a big list tail-recursively; sum folds it via a tail match.
    let src = "type List(a) { Nil, Cons(a, List(a)) }\n\
               fn range(n, acc) { if n == 0 { acc } else { range(n - 1, Cons(n, acc)) } }\n\
               fn sum(acc, xs) { match xs { Nil -> acc  Cons(h, t) -> sum(acc + h, t) } }\n\
               pub fn main() { let xs = range(1000000, Nil)\n let _ = sum(0, xs)\n io.println(\"done\") }\n";
    let (out, peak) = run_peak(src);
    assert_eq!(out, "done\n");
    assert!(peak <= K_MAX_MATCH, "tail match fold peak={peak} exceeds {K_MAX_MATCH}");
}

#[test]
fn non_tail_fold_grows_with_length() {
    // `h + sum(t)` — the recursive call is under `+`, so the continuation grows.
    let prog = |n: i64| format!(
        "type List(a) {{ Nil, Cons(a, List(a)) }}\n\
         fn range(n, acc) {{ if n == 0 {{ acc }} else {{ range(n - 1, Cons(n, acc)) }} }}\n\
         fn sum(xs) {{ match xs {{ Nil -> 0  Cons(h, t) -> h + sum(t) }} }}\n\
         pub fn main() {{ let _ = sum(range({n}, Nil))\n io.println(\"done\") }}\n");
    let (_s5, shallow) = run_peak(&prog(5));
    let (_s50, deep) = run_peak(&prog(50));
    assert!(deep > shallow, "non-tail match must grow: shallow={shallow}, deep={deep}");
    assert!(deep >= 45, "expected ~proportional depth, got {deep}");
}
```
- [ ] **Step 2: Measure + pin.** Run once; read the bounded loop's peak; set `K_MAX_MATCH` to it (expected small). Header comment: *"pinned from first measurement; a per-arm frame leak drives the peak toward the list length; do NOT raise K_MAX_MATCH — fix the machine."*
- [ ] **Step 3: Branch.** Bounded (expected) ⇒ tests pass, no machine change. If it grows ⇒ the `MatchK` handling leaks a frame in tail position; fix it (evaluate the winning body reusing `rest`, not pushing a new frame), re-measure, pin.
- [ ] **Step 4: fmt + gate + commit** (`test(eval): TCE through match — tail fold bounded at K_MAX_MATCH + non-tail grow control`); push. (Note the wall-clock like `tce_effects.rs`.)

---

## Task 5: Literal patterns

**Files:** Modify `src/ast.rs` (add `Pattern::Lit`), `src/parse.rs`, `src/types.rs`, `src/eval.rs`. Test: `tests/adt.rs`.

**Interfaces:** `Pattern` gains `Lit(Lit)` where `Lit` mirrors literal exprs — reuse a small enum `pub enum Lit { Int(i64), Bool(bool), Str(String), Unit }` (or the existing literal representation).

- [ ] **Step 1: Failing test** (`tests/adt.rs`):
```rust
#[test]
fn literal_patterns_run() {
    let src = "fn classify(n) { match n { 0 -> \"zero\"  _ -> \"other\" } }\n\
               fn name(b) { match b { True -> \"t\"  False -> \"f\" } }\n\
               pub fn main() { io.println(classify(0))\n io.println(name(False)) }\n";
    assert!(check_source("t.elya", src).is_ok());
    assert_eq!(run_source("t.elya", src).unwrap(), "zero\nf\n");
}
```
- [ ] **Step 2: Run to verify it fails** — literal patterns not parsed.
- [ ] **Step 3: Implement.** Parser `pattern()`: `Int`/`Str`/`True`/`False`/`Unit` tokens → `Pattern::Lit(...)`. Types `check_pattern` `Lit` arm: unify `expected` with the literal's base type. Eval `match_pattern` `Lit` arm: compare the value to the literal (`Value::Int(n) == Lit::Int(n)`, etc.). No exhaustiveness change yet (Task 6 consumes literals).
- [ ] **Step 4: Run + fmt + gate + commit** (`feat: literal patterns (Int/Bool/String/Unit) in match`); push.

---

## Task 6: Exhaustiveness + useless-arm (Maranget) — the substantial task

**Files:** Create `src/exhaust.rs`; modify `src/lib.rs` (call it), `tests/arch/layering.rs` (add `exhaust` to the layer map). Test: `tests/ui/*.elya`, inline `#[cfg(test)]` in `exhaust.rs`.

**Algorithm (Maranget, "Warnings for pattern matching"):** usefulness `U(P, q)` over the pattern matrix, via constructor **specialization** `S(c, P)`, the **default** matrix `D(P)`, and column **signature completeness**. Exhaustiveness = "is a wildcard vector useful against the arm matrix?" (useful ⇒ non-exhaustive, and the recursion **constructs the witness**). Useless arm *i* = "is arm *i* useful against arms `0..i`?" (not useful ⇒ redundant). It runs **after typing** and needs, per match, the scrutinee's type and the constructor **signature** of each column (from the type decls; `Bool` = `{True,False}`; `Int`/`Float`/`String` = infinite).

**Interfaces:**
```rust
// exhaust.rs
pub fn check(module: &Module, types: &TypeInfo) -> Vec<Diagnostic>;   // E0430 (error) + E0431 (warning)
// TypeInfo: what `types` must expose — per-constructor: which type it belongs to + field count;
//           per-type: its full constructor set (for completeness); this is the ctor table from Task 3.
// Internal:
//   enum Pat { Wild, Ctor { name, arity, args: Vec<Pat> }, LitInt(i64), LitBool(bool), ... }  // lowered from ast::Pattern (Var -> Wild)
//   fn useful(matrix: &[Vec<Pat>], q: &[Pat], sig: &Sig) -> Usefulness;   // Usefulness::{Useless, Witness(Vec<Pat>)}
//   fn specialize(ctor, matrix) -> Matrix;  fn default_matrix(matrix) -> Matrix;
//   fn render_witness(&Pat) -> String;   // "Cons(_, Cons(_, _))" — named ctors, `_` for don't-cares
```

- [ ] **Step 1: Write failing UI fixtures + inline tests.** Fixtures (register in `tests/ui.rs`):
  - `nonexhaustive_list.elya` — `match xs { Nil -> 0 }` over `List` ⇒ `//~ ERROR[E0430] Cons(_, _)` is not covered.
  - `nonexhaustive_nested.elya` — `match xs { Nil -> 0  Cons(_, Nil) -> 1 }` ⇒ `//~ ERROR[E0430] Cons(_, Cons(_, _))` (the **nested witness** — the correctness-critical case).
  - `nonexhaustive_int.elya` — `fn f(n) { match n { 0 -> 1 } }` ⇒ `//~ ERROR[E0430]` (infinite type demands a catch-all).
  - `useless_arm.elya` — `match o { Some(x) -> x  Some(y) -> y  None -> 0 }` ⇒ `//~ ERROR[E0431] unreachable` (warning; program still checks — see Step 4).
  Inline `exhaust.rs` tests: **finite exhausted accepts** — `match b { True -> 1  False -> 0 }` yields **no** diagnostic; `match b { True -> 1 }` yields `E0430`.
- [ ] **Step 2: Run to verify they fail** — `exhaust::check` doesn't exist.
- [ ] **Step 3: Implement `exhaust::check`.** Lower each `ast::Pattern` to the internal `Pat` (`Var`→`Wild`; keep literals). For each `Expr::Match`, build the 1-column matrix of arm patterns; run `useful(matrix_so_far, [Wild], sig)` for exhaustiveness (witness → `E0430`) and, incrementally, `useful(arms[0..i], arms[i], sig)` for each arm (not useful → `E0431`). `specialize`/`default_matrix`/completeness per Maranget; `render_witness` reassembles nested constructors with `_` for wildcards. Signatures come from `TypeInfo` (finite ADT/`Bool`; infinite `Int`/`Float`/`String`). Walk **all** matches in the module (nested matches too — recurse into expressions).
- [ ] **Step 4: Wire into the pipeline (non-fatal warnings preserved).** In `src/lib.rs`, after `types::infer` (when there are no type errors), `diags.extend(exhaust::check(&module, &type_info))`. `E0430` is an **error** (fails compilation via the existing `fail_if_errors` error-filter); `E0431` is a **warning** (non-fatal — the 3d infra). Add `exhaust = 5` to `tests/arch/layering.rs`.
- [ ] **Step 5: Run fixtures + inline tests + fmt + gate.** Both directions of the finite/infinite boundary pass; the nested witness text is exact; useless arm warns but the program still checks/runs; no `%`-token leaks (the `ui.rs` scan covers it).
- [ ] **Step 6: Commit + push** (`feat(exhaust): Maranget exhaustiveness (E0430 + nested witnesses) + useless-arm (E0431 warning)`).

---

## Task 7: CLI warning surfacing + integration + Slice-4a exit gate

**Files:** Modify `src/main.rs`. Test: `tests/adt.rs` (integration), confirm nets.

- [ ] **Step 1: Surface warnings in the CLI.** In `src/main.rs`, on a **successful** compile (no errors), render any `Severity::Warning` diagnostics to **stderr** (they no longer flow through `check_source`'s `Err` since 3d). This discharges the effects-spec §11 "warning CLI surfacing" obligation — now meaningful with `E0426`/`E0431`. (If `main.rs` currently drops the warning list, thread it out of the pipeline; keep it minimal.)
- [ ] **Step 2: Extend the cross-check corpus to ADTs.** In `tests/crosscheck.rs` (or its hand-written corpus), add an **effect-free ADT program** (e.g. `List` sum) asserting `cek == tree` — the net that *extends* to ADTs (spec §3.4).
- [ ] **Step 3: The Slice-4a exit gate.** `cargo fmt --all && sh scripts/check.sh` fully green:
  - ADT goldens (List/Option/Tree) type-check, run, **cross-check `cek == tree`**;
  - exhaustiveness accepts finite (`Bool` T/F) and rejects infinite-without-catch-all (`Int`), with **nested witnesses**; useless arms warn;
  - partial application gives the clear `E0433`;
  - TCE-through-`match` bounded at `K_MAX_MATCH` + growing non-tail control;
  - all Slice-3 nets byte-identical (effect goldens, `tce.rs`/`tce_effects.rs`, E042x); layering guard passes with `exhaust` added.
- [ ] **Step 4: Commit + push** (`test(adt): Slice-4a integration + exit gate; CLI surfaces warnings on success`).

---

## Self-Review

**1. Spec coverage (spec → tasks).**
- Parametric type formers + constructors-as-schemes (spec §2) → Tasks 2 (nullary `Con`), 3 (params/payloads/applied unification).
- Pattern typing + `match` typing (spec §2.4) → Task 2 (ctor/var/wild), 3 (into fields), 5 (literals).
- Exhaustiveness + useless-arm, nested witnesses, finite/infinite both ways (spec §4) → **Task 6**.
- `Value::Ctor` + `match` on tree+cek + TCE-through-match two-sided (spec §3) → Tasks 2 (dispatch), 4 (TCE).
- Saturated constructors + clear `E0433` (spec §6) → Task 3.
- Diagnostics `E0430`–`E0433` (spec §5) → Tasks 3 (E0433), 6 (E0430/E0431), 2 (E0432 in resolve).
- Cross-check extends to ADTs (spec §3.4) → Task 7.
- CLI warning surfacing (spec §7, §1.1) → Task 7.
- **Vertical-slice directive** (thread runs before parametric/Maranget/nested) → Task 2 is the end-to-end milestone; Tasks 3–6 widen it; exhaustiveness is deliberately last-but-one.

**2. Placeholder scan.** No `TBD`/`TODO`. `K_MAX_MATCH` is pinned-from-measurement (the `tce.rs` discipline, not a guess). The Task-1 downstream placeholder arms are *intentional, tested* scaffolding (the crate compiles; no golden hits them) replaced in Task 2 — as in Slice 3a.

**3. Type/name consistency.** `TypeDecl{name,params,variants}`, `VariantDecl{name,fields}`, `Pattern::{Wild,Var,Ctor,Lit}`, `Expr::Match{scrutinee,arms:Rc<[Spanned<MatchArm>]>}`, `MatchArm{pat,body}`, `Ty::Con(String,Vec<Ty>)`, `Value::Ctor(String,Vec<Value>)`, `Ctors = HashMap<&str,usize>`, `check_pattern(pat,expected,bindings)`, `match_pattern(v,pat)->Option<Vec<(String,Value)>>`, `exhaust::check(module,type_info)->Vec<Diagnostic>` are used identically across tasks. Codes `E0430`/`E0431`/`E0432`/`E0433` used consistently (E0432 resolve, E0433 Task 3, E0430/E0431 Task 6).

**4. Right-seam check.** Payloads + parameters are one task (Task 3), not split into monomorphic-with-fields then generic — the rejected "smaller pieces on the wrong axis." Exhaustiveness is its own substantial task (Task 6), as specced. The thread (Task 2) is the smallest end-to-end unit a reviewer can gate as "does an ADT run."

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-08-08-elya-slice-4a-adts.md`. **This is the review gate — no code until you approve the plan.** On approval, two execution options:

1. **Subagent-Driven** — a fresh subagent per task, review between tasks.
2. **Inline Execution** — execute Tasks 1–7 in this session with checkpoints (same as Slices 1–3).

Which approach — and any changes to the sequencing (Task 2 as the end-to-end thread; Task 6 exhaustiveness last-but-one) or the flagged design points?
