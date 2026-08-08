# Elya — Slice 4a Design Specification: Parametric Algebraic Data Types & Pattern Matching

- **Codename:** Elya
- **Slice:** 4a — the data layer (first sub-slice of Slice 4, "ADTs + generics + lambdas")
- **Status:** Design draft — awaiting review before an implementation plan is written
- **Date:** 2026-08-08
- **Depends on:** Slice 3 (complete, `6b2c35a`) — HM types with row-polymorphic effect inference, algebraic effects on the CEK machine, measured TCE (pure + effectful), and the zonk/no-gibberish diagnostic discipline (E0400–E0426).
- **Companion documents:** the language design spec (`.../2026-08-05-elya-language-design.md`, "design spec"); the Slice 3 effects spec (`.../2026-08-06-elya-slice-3-effects.md`, "effects spec").

---

## 0. How to read this document

This is a **design spec** for Slice 4a — **parametric algebraic data types** (sum types with type parameters) and **pattern matching** with **exhaustiveness + useless-arm checking**. It is the first of two sub-slices in Slice 4; **4b** (lambdas/closures) follows and is out of scope here (§10).

Section refs: "design spec §X" → the language design spec; "effects spec §X" → the Slice 3 spec; bare "§X" → this document. Diagnostic codes continue the scheme: `E043x` are **reserved for pattern matching** and defined here.

**The load-bearing realization (from brainstorming):** "generics" is not a separate feature. Hindley–Milner already gives generic *functions* (let-generalization, shipped in Slice 2). The only genuinely-new generic machinery is **type constructors with parameters** — which *is* the ADT type-system work. So "ADTs + generics" collapses into **parametric ADTs**, and generic *effects* (`State(s)`) become a small follow-on once 4a + 4b exist (§10). Slice 4a is therefore effect-free and closure-free: it touches the effect system **not at all**, landing the data layer on solid ground before 4b entangles closures with effect rows.

---

## 1. Scope

### 1.1 What Slice 4a delivers

1. **Parametric type declarations** — `type Name(params) { Variant, Variant(fields) … }`: sum types whose variants are constructors with **positional, typed payloads**; recursive and mutually-recursive declarations.
2. **Construction** — constructors applied like functions (`Some(3)`, `Cons(1, Nil)`); nullary constructors as bare values (`None`, `Nil`). **Saturated only** (§6): no partial application until 4b.
3. **Pattern matching** — `match e { pat -> body … }` with constructor, variable, wildcard, and **literal** patterns (Int/Bool/String/Unit), nested arbitrarily.
4. **Exhaustiveness + useless-arm checking** (Maranget) — non-exhaustive matches are `E0430` with a **constructed witness** (including nested witnesses); redundant arms are `E0431` (warning). This is the algorithmic heart, held to the E042x diagnostic discipline (§4).
5. **Type system** — a new applied-type-constructor former (`Ty::Con`), constructors typed as ordinary polymorphic schemes, pattern typing.
6. **Opportunistic fold-in** — **CLI warning surfacing** in `main.rs` (the §11 obligation from the effects spec): `Severity::Warning` diagnostics (E0426, now E0431) reach stderr on a successful compile.

### 1.2 Surface additions

Over Slice 3's surface, Slice 4a adds:

```elya
type Option(a) { None, Some(a) }
type List(a)   { Nil, Cons(a, List(a)) }
type Tree(a)   { Leaf, Node(Tree(a), a, Tree(a)) }

fn length(xs) {
  match xs {
    Nil        -> 0
    Cons(_, t) -> 1 + length(t)
  }
}

fn unwrap_or(o, d) {
  match o {
    None    -> d
    Some(x) -> x
  }
}
```

New syntax: `type NAME(p, …) { V, V(T, …), … }`; `match EXPR { PAT -> EXPR … }`; patterns `Ctor(p…)`, `x` (bind), `_` (wildcard), and literals (`0`, `True`, `"s"`, `Unit`). Constructor application reuses call syntax; nullary constructors are bare names.

### 1.3 What Slice 4a does NOT do (deferred — §10)

- **Lambdas / closures / first-class functions** → 4b. Consequently: **partial constructor application** (`map(Some, xs)`) is a clear error in 4a (§6), not silently typed.
- **Records / named fields** (`type P { MkP { x: Int, y: Int } }`) — positional payloads only; records are a separate feature.
- **Or-patterns** (`A | B ->`), **guards** (`Cons(h, _) if h > 0 ->`) — deferred.
- **GADTs, existentials, higher-kinded types, typeclasses/traits** — deferred (design spec / effects spec §11).
- **Generic *effects*** and **parameter-passing `State`** — enabled by 4a + 4b, land as a small follow-on.

---

## 2. Type declarations & the type former

### 2.1 Representation

A user type introduces a **type constructor** of fixed arity and a set of **value constructors**:

```
TypeDecl  { name: String, params: Vec<String>, variants: Vec<VariantDecl> }
VariantDecl { name: String, fields: Vec<TypeAnn> }        // positional field types
```

The type former grows one variant:

```
Ty ::= Var | Base(TyCon) | Fn([Ty], EffectRow, Ty) | Tuple([Ty]) | Con(String, [Ty]) | Error
```

`Con(name, args)` is an **applied, nominal** type constructor: `List(Int)` = `Con("List",[Int])`, `Option(a)` = `Con("Option",[Var])`, `Tree(Bool)` = `Con("Tree",[Bool])`. Built-in scalars stay `Base` (Int/Float/Bool/Str/Unit); only user types use `Con`.

### 2.2 Constructors as schemes (why generics is free)

Each value constructor is registered as an ordinary **polymorphic `Scheme`** — the same `Scheme` the type checker already instantiates for top-level functions:

```
Cons : forall a. (a, List(a)) -> List(a)     // Scheme{ vars:[a], ty: Fn([a, Con("List",[a])], pure, Con("List",[a])) }
Nil  : forall a. List(a)                     // Scheme{ vars:[a], ty: Con("List",[a]) }
Some : forall a. (a) -> Option(a)
None : forall a. Option(a)
```

So an *applied* constructor (`Cons(h,t)`) type-checks through the existing call path (look up the scheme, `instantiate` freshens `a`, unify), and a *nullary* constructor (`Nil`) type-checks through the existing `Var` path (instantiate → `Con("List",[fresh])`). **No new inference concept** — constructors are polymorphic values/functions. This is the concrete cash-out of "generics collapses into ADTs."

### 2.3 Unification, resolution, printing

- **Unify:** `(Con(n1,a1), Con(n2,a2))` succeeds iff `n1 == n2` and `a1.len() == a2.len()`, unifying `a1[i] ~ a2[i]` pairwise (nominal head, structural args — like `Tuple`, plus the name). Otherwise `E0400` with a zonked, named display.
- `resolve` / `occurs` / `free_vars` / `free_row_vars` / `subst_vars` recurse into `args` (mechanical, mirroring `Tuple`).
- **Display:** `Con("List",[Int])` → `List(Int)`; `Con("Tree",[Var])` → `Tree(a)` (free vars named `a, b, …` as today; no `%`-token). `List(Option(a))` nests.
- **Two-pass registration:** all `type` decls register their type constructor + value-constructor schemes **before** any function body is typed, so recursive (`List(a)` mentions `List(a)`) and mutually-recursive types resolve. Mirrors how effect operations register first.

### 2.4 Pattern typing

`check_pattern(pat, expected: Ty) -> Vec<(name, Ty)>` (the bindings), threaded into the arm body's scope:

| Pattern | Rule |
|---|---|
| `_` (wildcard) | matches `expected`; no binding. |
| `x` (variable) | binds `x : expected`. |
| literal `l` | unify `expected` with the literal's base type (`Int`/`Bool`/`Str`/`Unit`). |
| `Ctor(p…)` | look up the constructor's scheme, instantiate → field types `[F…]` and result `R`; unify `R ~ expected`; recurse `check_pattern(p[i], F[i])`. Arg-count ≠ field-count is a constructor-arity error (`E0402`-style, pattern-flavored message). |

`match e { arm… }`: infer `e : S`; for each arm, `check_pattern(arm.pat, S)` → bind, infer `arm.body : Ti` under the bindings; unify all `Ti` to one result `R`; the match's type is `R`.

---

## 3. Evaluation

### 3.1 Values & construction

`Value` gains one variant: `Ctor(String, Vec<Value>)` — lifetime-free, recursively nested (a `Tree` value holds `Tree` values). Construction reuses the existing apply/var paths (as effect operations do): a saturated call to a constructor name builds `Value::Ctor(name, arg_vals)`; a bare nullary constructor evaluates to `Value::Ctor(name, [])`. The machine (and the tree-walker) thread a **constructor arity table** (name → arity), built from the type decls like `fn_table`/`op_table`.

### 3.2 `match` on the CEK machine

Eval the scrutinee under a **transient `MatchK` frame** carrying the arms + env; on return, a pure `match_pattern(value, pat) -> Option<bindings>` tries arms **in order**, first match wins, and the winning body is evaluated **with the bindings, in the match's own continuation slot**:

```
match_pattern(v, Wild)          = Some([])
match_pattern(v, Var(x))        = Some([(x, v)])
match_pattern(v, Lit(l))        = if v == literal(l) { Some([]) } else { None }
match_pattern(Ctor(n, vs), Ctor(m, ps)) = if n==m && vs.len()==ps.len()
                                             { concat(match_pattern(vs[i], ps[i])…) } else { None }
```

Exhaustiveness (§4) guarantees some arm matches; a runtime fall-through is a **defensive** `E0300` (unreachable for well-typed programs).

### 3.3 TCE through `match` — two-sided (held to prior discipline)

Because the winning arm body is evaluated in the match's continuation slot (the `MatchK` frame is transient — pushed to evaluate the scrutinee, popped before the body), **a `match` in tail position preserves tail-call elimination**. This gets the *same two-sided teeth* as every TCE claim in this project (`tce.rs` `K_MAX = 3`, effect-TCE `K_MAX_EFF = 4`):

- **Bounded:** build a huge list tail-recursively (`range(n, acc) = if n == 0 { acc } else { range(n-1, Cons(n, acc)) }`), then fold it tail-recursively **through `match`** (`sum(acc, xs) = match xs { Nil -> acc; Cons(h, t) -> sum(acc+h, t) }`) — assert `peak_kont_depth ≤ K_MAX_MATCH` (pinned from first measurement).
- **Grow control (must increase):** a non-tail fold (`Cons(h, t) -> h + sum(t)`, the recursive call under `+`) must grow `peak` with list length — proving the machine distinguishes tail from non-tail `match`, so the bound can't be loosened to hide a leak. **Fix the machine, don't raise the constant.**

### 3.4 Cross-check net *extends*

The tree-walker cannot capture continuations (so it is not an oracle for *effect* programs, effects spec §4.6), but it **can** evaluate ADTs and `match` — no capture needed. So the Slice-2 `cek == tree` cross-check **extends to effect-free ADT programs** (both evaluators gain construction + `match`). A net *gain* in coverage, not a loss.

---

## 4. Exhaustiveness & useless-arm checking (the algorithmic core — its own task)

This is the one part of 4a with real algorithmic depth, and — like the effect diagnostics — its **error-message quality is what makes `match` usable rather than infuriating**. It is budgeted as its own substantial task and held to the zonk/no-gibberish, named-label discipline of E0420–E0426.

### 4.1 Algorithm

**Maranget's usefulness algorithm** over the pattern matrix (Luc Maranget, "Warnings for pattern matching"). A pattern *vector* `q` is *useful* w.r.t. a *matrix* `P` (the rows above it) if some value matches `q` but no row of `P`. The recursion uses constructor **specialization** `S(c, P)`, the **default** matrix `D(P)`, and column **completeness** against the type's constructor **signature**. It runs **after typing** (it needs each column's type to know its constructor set).

Two products:

1. **Exhaustiveness → `E0430` (error), with a constructed witness.** The match is exhaustive iff a wildcard vector is *not* useful against the arm matrix. If it *is* useful, the algorithm reconstructs an **actual uncovered pattern** — with correct constructor names and **nested** structure — which becomes the diagnostic:
   ```
   error[E0430]: non-exhaustive match: `Cons(_, Nil)` is not covered
   ```
   The witness is a real pattern, `_` for don't-cares, never "some case is missing."
2. **Useless / unreachable arm → `E0431` (warning).** Arm *i* is useless if it is *not* useful w.r.t. arms `0..i` (subsumed). A **warning** (matches Rust; leverages 3d's non-fatal-warning infrastructure — a redundant arm should not fail the build):
   ```
   warning[E0431]: unreachable match arm — already covered by an earlier pattern
   ```

### 4.2 Finite vs. infinite signatures (test both ways)

Completeness of a column depends on the type's signature:

- **Finite, exhaustible:** an ADT's constructor set (`List = {Nil, Cons}`) and **`Bool = {True, False}`**. `match b { True -> …; False -> … }` is exhaustive with **no** catch-all.
- **Infinite:** `Int` / `Float` / `String` — the literal "constructor" set is unbounded, so the column is *never* complete; **a wildcard or variable is required**. `match n { 0 -> … }` is non-exhaustive (witness: a fresh `_` — "`_` not covered", i.e. any non-zero value).

This finite/infinite boundary is where the coverage logic is most likely to be subtly wrong, so it is tested **both ways** (§8): `Bool` exhausted by `True`/`False` (no catch-all) *is accepted*; `Int` matched on a literal *demands* a catch-all.

### 4.3 Witness quality (test nested)

A naive witness builder handles only top-level constructors and breaks on nesting. The corpus therefore requires **nested witnesses**: e.g. `match xs { Nil -> …; Cons(_, Nil) -> … }` over `List` must report the missing `` `Cons(_, Cons(_, _))` `` — the specialization recursion must reassemble the nested uncovered shape correctly. This is where witness correctness *and* message quality are actually exercised.

---

## 5. Diagnostics (new codes)

Same discipline as E0400–E0426: named identifiers, zonked types, no `%`-token.

- **`E0430` — non-exhaustive match** (error): a constructed witness pattern, nested where needed (§4.1, §4.3).
- **`E0431` — unreachable match arm** (warning): an arm subsumed by earlier arms (§4.1); non-fatal (3d infrastructure).
- **`E0432` — unknown constructor or type** (resolve error): a pattern or expression names a constructor/type that no `type` declaration introduced.
- **`E0433` — unapplied / under-applied constructor** (error): a bare or partially-applied n-ary constructor, with the 4b-pointing message (§6).
- Constructor **arity** mismatches (wrong number of args, in a call or a pattern) reuse the arity machinery with a pattern-aware message; type mismatches reuse `E0400`.

(`E0430`–`E0433` are the `E043x` block reserved for pattern matching / ADTs.)

---

## 6. Saturated constructors & the partial-application error

In 4a a constructor value is either **nullary** (a value: `None`, `Nil`) or **fully applied** (`Some(3)`, `Cons(h, t)`). A **bare, unapplied n-ary constructor** (`Some`, `Cons`) would be a first-class *function value*, which needs 4b's closures — so it is an **error in 4a**, with a message that names the real situation rather than a baffling type error (because `map(Some, xs)` is the natural thing a user tries first):

```
error[E0433]: constructor `Some` needs 1 argument
  = unapplied constructors become first-class function values in Slice 4b; apply it here, e.g. `Some(x)`
```

(`E0433` — under-applied/unapplied constructor.) This keeps 4a closure-free and holds the 4a/4b seam cleanly; 4b removes the restriction by making constructors first-class.

---

## 7. Pipeline & module changes

- **AST (`ast.rs`, `parse.rs`):** `Decl::Type(TypeDecl)`, `VariantDecl`; `Expr::Match { scrutinee: Rc<…>, arms: Vec<Spanned<MatchArm>> }`, `MatchArm { pat, body: Rc<…> }`; `Pattern` (`Ctor`/`Var`/`Wild`/`Lit`). Construction reuses `Call`/`Var` (no new node). Pretty-printer arms. Parser: `type` declarations, `match` expressions, pattern grammar.
- **Resolve (`resolve.rs`):** register type + constructor names; resolve constructor names in expressions and patterns; bind pattern variables in the arm body scope; `E0432` unknown constructor/type; saturation check feeding `E0433`.
- **Types (`types.rs`):** `Ty::Con` + its unify/resolve/free-vars/subst/display arms; the constructor-scheme table (registered two-pass); pattern typing (§2.4); `match` typing. **Exhaustiveness/useless-arm is a separate pass** (`E0430`/`E0431`) run after typing, over the typed matches.
- **Eval (`eval.rs`):** `Value::Ctor`; construction on both the tree-walker and the CEK machine; `match` dispatch (`MatchK` frame + `match_pattern`); constructor arity table threaded like `fn_table`. TCE-through-match falls out of the transient-frame design (§3.3).
- **CLI (`main.rs`):** render `Severity::Warning` diagnostics to stderr on a successful compile (the effects-spec §11 obligation), now that warnings (E0426, E0431) exist and matter.
- **Pipeline / layering:** `parse → resolve → infer → exhaustiveness → eval`; the exhaustiveness pass sits inside/after type-checking. **Layer map unchanged** (`types = 4`, `eval = 6`); `tests/arch/layering.rs` still guards it.

---

## 8. Testing strategy

- **Type + run goldens** (effect-free ⇒ **cross-checked `cek == tree`**): `length`/`sum` over `List`, `unwrap_or` over `Option`, `depth` over `Tree`, deeply nested `match`.
- **Exhaustiveness — both directions of the finite/infinite boundary (§4.2):** `Bool` exhausted by `True`/`False` with no catch-all *accepts*; `Int` on a literal *demands* a catch-all (`E0430`); a missing ADT constructor is `E0430`.
- **Witness corpus — nested (§4.3):** at least one fixture whose witness is nested (`Cons(_, Cons(_, _))`), asserting the exact witness text, named constructors, no `%`-token.
- **Useless arm:** a subsumed arm produces `E0431` (warning) and the program still type-checks/runs (non-fatal, exercising 3d infra).
- **Partial application:** `map(Some, xs)`-shaped program produces the clear `E0433` (§6), not a raw type error.
- **TCE through `match` — two-sided (§3.3):** a million-element tail fold bounded at a pinned `K_MAX_MATCH`; a non-tail fold grows with length.
- **CLI warnings:** a program that emits `E0431`/`E0426` compiles+runs and its warning reaches stderr (surfacing test).
- **Retained nets:** effect goldens, `tce.rs`/effect-TCE bounds, and E042x fixtures all stay green; the full gate is green and pushed.

---

## 9. Build order (tasks — a plan per this sub-slice)

1. **AST nodes + pretty** (`TypeDecl`, `Match`, `Pattern`); behavior-preserving placeholder arms downstream.
2. **Parser** — `type` declarations, `match`, pattern grammar (incl. literals).
3. **Resolve** — register types/constructors; resolve ctor names + pattern binding; `E0432`; saturation → `E0433`.
4. **Types** — `Ty::Con` (+ unify/resolve/display fan-out); constructor schemes (two-pass); pattern typing; `match` typing.
5. **Exhaustiveness + useless-arm (Maranget) — the substantial task** — `E0430` with nested witnesses, `E0431` warning; finite/infinite signatures.
6. **Eval** — `Value::Ctor`, construction, `match` dispatch on tree + CEK; TCE-through-match; extend cross-check to ADT programs.
7. **CLI warning surfacing + integration + exit gate** — `main.rs` stderr warnings; the §8 corpus; full gate green + pushed.

### Exit criterion
ADT goldens type-check, run, and **cross-check `cek == tree`**; exhaustiveness accepts/rejects correctly across the finite/infinite boundary with **nested witnesses**; useless arms warn; partial application gives the clear `E0433`; TCE-through-match is bounded at a pinned constant with a growing non-tail control; warnings reach stderr; all Slice-3 nets stay green; full gate green and pushed to `origin/main`.

---

## 10. Risks & mitigations

- **Exhaustiveness correctness/wording** (the highest-risk piece) → Maranget by the book; **witness construction tested at nesting depth ≥ 2**; finite/infinite tested both ways; named-label, no-`%` invariant reused.
- **`Ty::Con` fan-out** → mechanical (mirror `Tuple` + a name); land it behind the existing type suite before pattern typing consumes it.
- **TCE regressing under `match`** → the transient-`MatchK` design + the two-sided pinned-constant test (bounded fold + grow control).
- **Constructor/function namespace confusion** → constructors are `Upper`, functions/vars `lower` (lexer already distinguishes); constructors live in the type env as schemes and in an eval arity table.
- **Scope creep** → records, or-patterns, guards, GADTs, and *all* of lambdas explicitly out (§1.3, §10-deferred); 4a is data + match only.

---

## 11. Deferred / Honestly-Flagged

| Deferred | Where it lands | Why safe |
|---|---|---|
| Lambdas / closures / first-class functions | 4b | The next sub-slice; 4a stays closure-free by requiring saturated constructors (§6). |
| Partial constructor application (`map(Some, xs)`) | 4b | Needs first-class functions; 4a gives the clear `E0433` (§6) instead of a silent/confusing type. |
| Records / named-field constructors | later | Positional payloads demonstrate ADTs; records are an additive surface later. |
| Or-patterns, guards | later | Maranget extends to both; not needed for the data layer. |
| GADTs, existentials, higher-kinded, typeclasses/traits | later / not planned | Out of the HM core; effects spec §11 keeps these deferred. |
| Generic *effects* (`State(s)`), parameter-passing `State` | small follow-on after 4a + 4b | `EffectLabel` is ready to carry type args once ADTs + closures exist; discharges the effects-spec §11 obligations then. |

**Manifesto check (design spec §1):** ADTs + pattern matching are the data layer every real program needs and the foundation the frontier work (linear/affine types, shape-checking) stands on. 4a keeps purity-by-default (construction and `match` are pure), keeps "no subtyping" (nominal `Con` unifies by name + args, no variance), and stays HM-inferable (constructors are ordinary schemes). It adds expressiveness without competing with the manifesto.

---

## 12. Milestone Checklist (Slice 4a)

- [ ] `type` declarations (parametric, recursive, mutually-recursive) parsed + resolved; constructors registered as schemes.
- [ ] `Ty::Con` unifies/resolves/prints (`List(Option(a))`), no `%`-token.
- [ ] Construction (saturated) + `match` on tree-walker **and** CEK; effect-free ADT programs cross-check `cek == tree`.
- [ ] Pattern typing; `match` typing; `E0432` unknown ctor/type; `E0433` clear partial-application error.
- [ ] Exhaustiveness `E0430` with **nested witnesses**; useless-arm `E0431` (warning); finite/infinite tested **both ways**.
- [ ] TCE-through-`match` bounded at a pinned constant + a growing non-tail control.
- [ ] `main.rs` surfaces `Severity::Warning` to stderr on success; warning-surfacing test.
- [ ] All Slice-3 nets green; full gate green; pushed.

---

*Slice 4a lays Elya's data layer — parametric sum types and total pattern matching — on top of the effect system, with exhaustiveness as a first-class, witness-quality diagnostic. It is the ground the frontier stands on: 4b's effect-carrying closures, then linear/affine types that turn the best-effort `E0426` lint into a real guarantee. Review gate: this is the foundation for the rest of Slice 4; the design is for your approval before an implementation plan is written.*
