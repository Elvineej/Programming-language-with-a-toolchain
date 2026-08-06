# Elya — Slice 2 Design Specification: Hindley–Milner Types & the CEK Machine

- **Codename:** Elya
- **Slice:** 2 of the Slices 1–3 implementation cycle
- **Status:** Design draft — awaiting review before an implementation plan is written
- **Date:** 2026-08-06
- **Depends on:** Slice 1 (merged to `main`, commit `1e7947d`) — the working tree-walking interpreter.
- **Companion documents:** the language design spec (`docs/superpowers/specs/2026-08-05-elya-language-design.md`, "the design spec" below) and the Slice 1 plan (`docs/superpowers/plans/2026-08-05-elya-slice-1-interpreter.md`).

---

## 0. How to read this document

This is a **design spec** for Slice 2, not the bite-sized implementation plan. It fully specifies the two headline deliverables — **Hindley–Milner type inference (Algorithm J)** and the **CEK abstract-machine refactor** — plus the **TCE bounded-depth guarantee** as a testable assertion. Everything Slice 2 leans on that isn't built yet is flagged in §9 rather than papered over.

Section-number references of the form "design spec §X" point at the language design spec; bare "§X" is this document.

---

## 1. Slice 2 Scope

### 1.1 What Slice 2 delivers

1. **A real type checker**: Hindley–Milner inference via **Algorithm J** (union-find substitution), with let-polymorphism, top-level recursive-group handling by SCC dependency analysis, and typed diagnostics that meet the design spec's "no raw type-variable gibberish" discipline (design spec §9, here applied to *types* as it is there to effect rows).
2. **The CEK abstract machine**: the recursive tree-walker is refactored into an explicit **Control / Environment / Kontinuation** state machine driven by an iterative step loop. This is the load-bearing refactor the design spec sequences *before* effects (Slice 3) and it is what makes both continuation capture (Slice 3) and TCE (below) possible.
3. **TCE as a tested correctness guarantee**: Slice 1 shipped only a depth *grow-control*. Slice 2 delivers the real promise — **deep tail recursion runs in bounded continuation depth**, asserted against a pinned constant, with paired non-tail controls that must grow.

### 1.2 Surface additions (deliberately small)

Slice 2 **deepens the pipeline more than it widens the surface.** The runnable language is Slice 1's (literals; `Int`/`Float` arithmetic with the distinct-operator split; `<>`; comparisons; `let`; `if/else`; top-level functions; calls; `io.println`) **plus one addition**: a **minimal `case`** over *literal, wildcard, and variable-binding* patterns.

```elya
fn classify(n) {
  case n {
    0 -> "zero"
    1 -> "one"
    other -> "many"     // variable pattern binds `other`
  }
}
```

`case` scrutinees may be `Int`, `Bool`, or `String`; patterns are integer/bool/string **literals**, the wildcard `_`, or a lowercase **variable** that binds the scrutinee. **Constructor patterns, tuple/list patterns, and exhaustiveness checking are NOT in Slice 2** — they arrive in Slice 3 with algebraic data types (§9).

> **Scope note (open for your review):** `case` is the one place Slice 2 widens the surface, and the design spec §12 lists it under Slice 2 while listing "pattern matching / enums / exhaustiveness" under Slice 3. Minimal literal-`case` is coherent in Slice 2 and makes inference more interesting (all arms unify), but it is also cleanly deferrable to Slice 3 where it joins full pattern matching. **Recommendation: keep the minimal `case` in Slice 2** (fully specified in §2.7 and §3). If you'd rather Slice 2 be purely "types + CEK + TCE over the exact Slice-1 surface," say so and I'll cut `case` from this spec.

### 1.3 What Slice 2 explicitly does NOT do

Algebraic data types, enums, records, constructor pattern matching, exhaustiveness, user-defined generic *types*, traits/type-classes, lambdas / first-class function *expressions*, a surface-level checked type-annotation syntax, a separate Core IR, effect-row typing/enforcement, and handlers/`resume`. Each is placed and justified in §9.

Polymorphism in Slice 2 is **parametric polymorphism over top-level functions**, produced by HM generalization — e.g. `fn id(x) { x }` infers `∀a. a -> a`. No user syntax is required or accepted for it; this is the substrate on which Slice 3's generic *data types* are later built.

---

## 2. Type System — Hindley–Milner, Algorithm J

### 2.1 Types and schemes

```
Type    ::= Var(v)                     -- unification variable
          | Con(name, [Type])          -- Int, Float, Bool, String, Unit, and applied constructors
          | Fn([Type], Type)           -- (params) -> return  (no effect row in Slice 2; see §9)
          | Tuple([Type])

Scheme  ::= Forall([v], Type)          -- generalized type; [] quantifiers => a monotype
```

The base constructors present in Slice 2 are the nullary `Int`, `Float`, `Bool`, `String`, `Unit`. `Con` carries an argument list to remain forward-compatible with Slice-3 generic data types, but Slice 2 constructs only nullary base types plus `Fn`/`Tuple`.

`Fn` carries **no effect row** in Slice 2 (design spec's arrow gains its row in Slice 3; §9). This is a deliberate, temporary simplification — the arrow is the exact structure that later grows an effect-row field, mirroring how Slice 3 will extend it.

### 2.2 Unification (union-find + occurs-check)

Algorithm J represents the substitution *in place* via a union-find over `Var`s. Each `Var` points to either "unbound (with a rank/level)" or "bound to a `Type`". Unification:

- `unify(Var a, t)` / `unify(t, Var a)`: follow `a` to its representative; if unbound, **occurs-check** `a` in `t` (fail → **E0401 infinite type**), then bind `a := t`.
- `unify(Con(n, xs), Con(m, ys))`: if `n != m` or `xs.len() != ys.len()` → **E0400 type mismatch**; else unify pairwise.
- `unify(Fn(ps, r), Fn(qs, s))`: if `ps.len() != qs.len()` → **E0402 arity mismatch**; else unify parameters pairwise and returns.
- `unify(Tuple(xs), Tuple(ys))`: lengths must match (E0400) then pairwise.
- any other shape pair → **E0400 type mismatch**.

Unification failures carry the two **spans** whose typing induced the constraint (see §2.8), not just the types.

### 2.3 Inference rules (constraint generation)

Inference walks the AST in a typing environment `Γ : name → Scheme`, returning a `Type` and mutating the substitution. The rules for Slice 2 constructs:

| Expression | Rule |
|---|---|
| `Int` / `Float` / `Str` / `Bool` / `Unit` literal | the corresponding base `Con`. |
| `Var(x)` | look up `x` in `Γ`; **instantiate** its scheme (§2.4). Unknown name is already an **E0200** from resolution (§5); inference treats it as a fresh error var to avoid cascades. |
| `Binary{op, l, r}` | infer `l`, `r`; unify both operands and the result with the operator's fixed monotype (§2.6). |
| `Unary{op, e}` | infer `e`; unify with the operator's monotype. |
| `If{c, t, e}` | infer `c`, unify with `Bool` (**E0403** if it can't); infer `t` and `e`, unify their types; result is that type. |
| `Block{stmts, tail}` | thread `Γ` through statements (`let` extends it, §2.4); result is the `tail` type, or `Unit` if no tail. |
| `Call{callee, args}` | infer `callee : F`; infer each `arg : Aᵢ`; make a fresh result var `R`; `unify(F, Fn([A₁..], R))`; result `R`. Arity surfaces as **E0402**. |
| `Qualified{m, n}` used as a call callee | resolved to a **builtin** type (§2.6); a bare (uncalled) qualified reference is an **E0201**-adjacent error (kept from Slice 1's resolver). |
| `Case{scrutinee, arms}` | §2.7. |

### 2.4 Generalization, instantiation, let-polymorphism

- **Instantiation** of `Forall([vs], t)`: substitute each quantified `v` with a fresh unbound `Var`. Every *use* of a polymorphic binding instantiates independently — this is what lets `id` be used at `Int` and `String`.
- **Generalization** of a `Type t` under `Γ`: quantify exactly the `Var`s free in `t` that are **not** free in `Γ`. (We use the standard *level/rank* optimization so generalization is O(size of `t`) rather than scanning all of `Γ`.)
- **`let x = e1; …`** in a block: infer `e1 : t`, generalize `t` under the current `Γ` to a scheme, extend `Γ` with `x : scheme` for the rest of the block. Sound because Slice 2 has no mutable references (no value-restriction hazard). *(Without lambdas, local `let` rarely produces a generalizable variable, but the rule is stated correctly so it stays correct when lambdas arrive.)*

### 2.5 Top-level functions: recursive groups via SCC analysis

Top-level functions may be mutually recursive (Slice 1 already puts all of them in scope for each other). Correct HM treatment:

1. Build the **call graph** over top-level function names (an edge `f → g` if `f`'s body references `g`).
2. Compute **strongly-connected components** (Tarjan) and process them in **topological order**.
3. For each SCC (a minimal mutually-recursive group): assign every member a fresh **monotype** variable, infer all member bodies with those monotypes in `Γ` (so recursion within the group is **monomorphic**), unify, then **generalize each member** and add the generalized schemes to `Γ` for later SCCs.

This yields proper let-polymorphism across the top level — `id` (its own singleton SCC) generalizes to `∀a. a -> a` and a later function/`main` may use it at several types — while keeping mutual recursion sound.

**Flag (§9):** *polymorphic recursion* — a function using itself at two different types *within its own SCC* — is unsupported (it is undecidable without annotations). Deferred; would require the checked-annotation syntax also deferred to Slice 3.

### 2.6 Builtins and operators

Monomorphic, matching the design spec's "distinct numeric operators, no numeric type-classes" decision:

- `+ - * /` `%`   : `Fn([Int, Int], Int)`
- `+. -. *. /.`   : `Fn([Float, Float], Float)`
- `<> `           : `Fn([String, String], String)`
- `< <= > >=`     : `Fn([Int, Int], Bool)` *(Int ordering only in Slice 2; `Float` ordering operators and their dotted forms are parsed by the lexer but out of Slice-2 scope — §9)*
- `== !=`         : **structural over a single type** — `Fn([a, a], Bool)` for a fresh `a` per use (both operands must have the same type; the result is `Bool`). This is the one operator with a type variable; it is *parametric*, not ad-hoc, so it needs no traits.
- unary `-`       : `Int -> Int` or `Float -> Float` — resolved by unifying the operand; if ambiguous with no constraint, defaults are **not** applied (an unconstrained unary-minus is an **E0400**; in practice the operand is always constrained).
- unary `!`       : `Bool -> Bool`.
- **`io.println`** : typed `Fn([String], Unit)`. **Its effect row is ignored in Slice 2** (§9).

`main` is typed `Fn([], Unit)` (its `/ {IO}` annotation is parsed and retained by name, not typed — §9).

### 2.7 Minimal `case` typing

For `case scrut { p1 -> e1; … }`: infer `scrut : S`; for each arm, type the pattern against `S` (a literal pattern unifies `S` with its literal's base type; `_` imposes no constraint; a variable pattern binds the variable to `S` in that arm's environment), infer the arm body, and unify all arm-body types to a single result `R`. Result is `R`. **No exhaustiveness check** (§9); a scrutinee matching no arm is a **runtime** error (E03xx) in the evaluator.

### 2.8 Diagnostics and the anti-gibberish discipline

Type errors extend the code scheme: **E0400** type mismatch, **E0401** infinite type (occurs-check), **E0402** arity mismatch, **E0403** non-`Bool` condition. (E0404–E0419 reserved for future general type errors; **E0420–E0429 remain reserved for effects**, design spec §9.)

The discipline mirrors the effect-row rule (design spec §9): **never print a raw internal type-variable.** Before rendering, the reporter **zonks** the types (applies the current substitution) and pretty-prints remaining free variables as readable letters (`a`, `b`, `c`, …) assigned per-report, never `%t7`. Each error carries **both spans** that induced the mismatch and states the two conflicting types. Illustrative fixtures (these become UI tests):

```
error[E0400]: type mismatch
  ┌─ t.elya:1:16
1 │ fn f() { 1 + "a" }
  │            - --- this is `String`
  │            |
  │            `+` requires `Int` on both sides
  = expected `Int`, found `String`
```

```
error[E0401]: infinite type
  ┌─ t.elya:1:10
1 │ fn f(x) { x(x) }
  │          ^^^^ `x` would have to be `a` and `(a) -> b` at once
  = cannot construct the infinite type `a = (a) -> b`
```

**UI-test invariant:** every type-error fixture asserts the message contains the readable type names and contains **no** `%t`/`%v` internal-variable token — the same gate Slice 1 applied to effect rows.

### 2.9 Honest deferrals inside the type system

- **Effect rows are not typed.** `/ {IO}` is parsed and retained by name (as in Slice 1) but neither inferred nor enforced. A function that performs IO need not declare it; effect *safety* is not yet checked. Row-polymorphic effect inference is the whole of Slice 3.
- **Type annotations are parsed-and-discarded** (Slice 1 behavior retained). Slice 2 does **pure inference**; it does not build a surface `Type` AST nor check annotations against inferred types. This lands in Slice 3, where ADT type syntax (`Tree(a)`) is exactly what annotations reference.
- **No ADTs, generics-syntax, traits, or lambdas** — §9.
- **Polymorphic recursion** unsupported — §2.5.

---

## 3. The CEK Abstract Machine

### 3.1 Why now

The design spec sequences the CEK refactor into Slice 2, *before* effects, for three reasons: (a) making the continuation an explicit, capturable data structure is the prerequisite for Slice-3 handlers and `resume`; (b) an iterative step loop gives TCE and removes host-stack limits; (c) it is the stepping stone to a bytecode VM later. Slice 1's tree-walker is retained as a **test oracle** (§3.8), not deleted.

### 3.2 State, values, environment, continuation

```
State ::= Eval(expr, Env, Kont)        -- evaluate `expr`
        | Return(Value, Kont)          -- deliver `Value` to the top frame of `Kont`

Value ::= Int | Float | Str | Bool | Unit
        | Closure(fn_decl, Env)        -- top-level fns close over the globals Env

Env  = persistent, Rc-linked scopes:  Env ::= Scope{ vars: Map<Name,Value>, parent: Option<Rc<Scope>> }
Kont = persistent, Rc-linked frames:  Kont ::= Nil | Rc<(Frame, Kont)>
```

Both `Env` and `Kont` are **persistent and immutable** (structural sharing via `Rc`). No new dependency is required — this uses only `std::rc::Rc`.

> **Flag (§9 — designed now for Slice 3):** a plain `Vec` continuation would suffice for a *non-capturing* Slice-2 machine and would also give TCE. We choose the persistent representation now specifically so Slice 3's handlers can **capture and re-enter** a continuation (`resume`, including multi-shot) with no rewrite — exactly the "one-shot and multi-shot share the representation" claim in design spec §8.4. This is deliberate forward-investment, and it is the one place Slice 2's implementation is shaped by a Slice-3 requirement.

### 3.3 Frame set (Slice-2 subset)

| Frame | Held data | Meaning |
|---|---|---|
| `BinRight{op, rhs, env}` | operator, unevaluated rhs, env | waiting for the **lhs** value; then evaluate rhs |
| `BinApply{op, lval}` | operator, evaluated lhs | waiting for the **rhs** value; then apply the operator |
| `UnApply{op}` | operator | waiting for the operand value; then apply |
| `IfBranch{then_blk, else_blk, env}` | both blocks, env | waiting for the condition; then pick a branch |
| `LetCont{name, rest, env}` | binding name, rest-of-block, env | waiting for the `let` value; then bind and continue the block |
| `SeqDrop{rest, env}` | rest-of-block, env | waiting for a statement-expression value (discarded); then continue |
| `CallArgs{callee_val, done, pending, env}` | evaluated callee, values so far, args left, env | evaluating arguments left-to-right; when none left, apply |
| `CaseArms{arms, env}` | remaining arms, env | waiting for the scrutinee; then match |

`CallArgs` is written so that, once the callee and all arguments are values, **application happens without leaving a residual frame** (§3.5).

### 3.4 The step function

`step(State) -> Step` where `Step ::= Continue(State) | Done(Value) | RuntimeError`. **Eval** transitions decompose an expression, pushing at most one frame and moving to a sub-expression or to `Return`. **Return** transitions consume the top frame. Representative transitions (complete set covers every frame in §3.3):

Eval:
- `Eval(Lit v, _, k)` → `Return(v, k)`
- `Eval(Var x, env, k)` → `Return(lookup(env, x), k)`
- `Eval(Binary{op,l,r}, env, k)` → `Eval(l, env, BinRight{op,r,env} :: k)`
- `Eval(If{c,t,e}, env, k)` → `Eval(c, env, IfBranch{t,e,env} :: k)`
- `Eval(Block{[], tail}, env, k)` → `Eval(tail, env, k)`  *(empty-stmt block: tail in tail position — no frame added)*
- `Eval(Block{[Let{x,v}, …rest], tail}, env, k)` → `Eval(v, env, LetCont{x, Block{rest,tail}, env} :: k)`
- `Eval(Block{[ExprStmt e, …rest], tail}, env, k)` → `Eval(e, env, SeqDrop{Block{rest,tail}, env} :: k)`
- `Eval(Call{callee,args}, env, k)` → `Eval(callee, env, CallArgs{callee_val: none, done: [], pending: args, env} :: k)`
- `Eval(Case{scrut,arms}, env, k)` → `Eval(scrut, env, CaseArms{arms, env} :: k)`

Return:
- `Return(v, BinRight{op,r,env} :: k)` → `Eval(r, env, BinApply{op, v} :: k)`
- `Return(v, BinApply{op, lval} :: k)` → `Return(apply_binop(op, lval, v)?, k)`
- `Return(Bool b, IfBranch{t,e,env} :: k)` → `Eval(if b {t} else {e}, env, k)`  *(chosen branch inherits `k` — tail position)*
- `Return(v, LetCont{x, rest, env} :: k)` → `Eval(rest, extend(env, x, v), k)`
- `Return(_, SeqDrop{rest, env} :: k)` → `Eval(rest, env, k)`
- `Return(v, CaseArms{arms, env} :: k)` → match `v` against `arms`; `Eval(chosen_body, env', k)` (arm body in tail position) or `RuntimeError` if no arm matches
- `Return(v, CallArgs{…} :: k)` → advance argument evaluation; when the callee and all args are values, **apply** (§3.5)
- `Return(v, Nil)` → `Done(v)`

The machine is driven by a plain **loop** — `while let Continue(s) = step(s) { … }` — so recursion in the *object* language never consumes *host* stack.

### 3.5 Application is the TCE lever

When `CallArgs` has the callee value and every argument value:

- **Builtin callee** (`io.println`): perform the effect (append to output), then `Return(Unit, k)`.
- **`Closure(fn, cenv)`**: build `env' = extend(cenv, params ↦ arg_values)` — a fresh small scope on the **closure's captured environment**, *not* on the caller's — and transition to `Eval(fn.body, env', k)`.

Crucially, **application pushes no frame**: the callee's body is evaluated under the *same* continuation `k` that was in force at the call. Therefore:
- if the call is in **tail position**, `k` is the enclosing function's own continuation and the machine simply swaps `control`+`env` — **no growth**;
- if the call is **non-tail** (e.g. `f(x) + 1`), a `BinApply`/other frame is already sitting on `k`, so the pending work is preserved.

The tail/non-tail distinction is thus **automatic** from continuation shape; no separate "is-tail" flag is needed at runtime. (A static tail-position analysis is still specified in §4.1 for *testing* the guarantee and for the future `musttail` native lowering.)

### 3.6 Builtins

`io.println` remains the only builtin, dispatched by the qualified name at application time, exactly as Slice 1 — it appends `arg <> "\n"` to the interpreter's output buffer and returns `Unit`.

### 3.7 Oracle cross-check

The Slice-1 tree-walker is retained as `eval::tree::run_module`; the CEK machine is `eval::cek::run_module`, sharing `Value`/`Env`/`RuntimeError`. A cross-check test asserts **`cek_output == tree_output`** for every example program and every evaluator unit test. The public pipeline switches to CEK; the tree-walker exists only to keep the CEK machine honest until the Slice-4 VM takes over that role (design spec §11.3).

Cross-checking uses **shallow** programs only: the tree-walker recurses on the host stack and would overflow on the deep programs used for the TCE tests, which therefore run on the CEK machine alone (§4.4).

### 3.8 Honest deferrals in the machine

- **No Core IR.** The CEK machine steps over the **typed AST directly**. The design spec's Core IR (desugaring, decision-tree compilation of patterns) becomes worthwhile with ADTs and is deferred to Slice 3 (§9).
- **Persistent kont/env is forward-investment** for Slice-3 capture (§3.2 flag), not used by any Slice-2 feature.

---

## 4. TCE — the Bounded-Depth Guarantee (testable)

### 4.1 Tail position (Slice-2 subset)

An expression is in **tail position** of a function body when its value is the function's result with no pending work:
- the `tail` expression of the function body block;
- both branch blocks of a tail-position `if` (their tails);
- every arm body of a tail-position `case`;
- a `let`/expression *statement* is **never** in tail position (there is always a following statement or the block tail).

A **tail call** is a `Call` in tail position.

### 4.2 Machine rule

By §3.5, application pushes no frame, so a tail call evaluates the callee body under the enclosing continuation with **no increase in `|Kont|`**. A non-tail call necessarily has a pending frame already on `Kont`. This is the mechanism; §4.1's static definition exists to *test* it and to drive the future native `musttail`.

### 4.3 Both `Kont` and `Env` stay bounded

TCE requires *two* things not to grow with recursion depth:
- **`Kont`**: guaranteed by §4.2 (no frame pushed on tail calls).
- **`Env`**: guaranteed by §3.5 — application builds `env'` from the **closure's** environment plus params, replacing the caller's environment rather than nesting on it. For a top-level recursive function the closure environment is the fixed-depth globals scope, so each tail call's `env'` has constant depth. (A naive Slice-1-style `Env` that clones-and-pushes the caller's scopes would defeat TCE by growing memory linearly; the persistent replace-don't-nest representation of §3.2 is what prevents that.)

### 4.4 The testable assertions (the deliverable)

Instrument the machine with `peak_kont_depth` (max `|Kont|` observed over a run). The regression suite asserts:

1. **Bounded self-tail-recursion.** For
   ```elya
   fn count_down(n) { if n == 0 { 0 } else { count_down(n - 1) } }
   pub fn main() { let _ = count_down(1000000)  io.println("done") }
   ```
   assert `peak_kont_depth <= K_MAX`, where **`K_MAX` is a single-digit constant pinned at first measurement** (equality-style tight bound, per design spec §11.4). A per-iteration frame leak would drive the depth toward 1,000,000 and fail; a constant off-by-one would exceed the pinned ceiling and fail.

2. **Bounded mutual tail-recursion (intra-file).**
   ```elya
   fn is_even(n) { if n == 0 { True } else { is_odd(n - 1) } }
   fn is_odd(n)  { if n == 0 { False } else { is_even(n - 1) } }
   ```
   `is_even(1000000)` runs at bounded `peak_kont_depth` — proving tail calls survive the function-boundary/name-resolution path.

3. **Existence proof.** The deep run (1) *completes at all*; the retained tree-walker oracle would overflow the host stack on the same program — so the test also asserts the CEK machine returns while the tree-walker is (correctly) not asked to.

4. **Grow control.** A non-tail counterpart
   ```elya
   fn sum(n) { if n == 0 { 0 } else { n + sum(n - 1) } }   // `n + …` makes the call non-tail
   ```
   must show `peak_kont_depth` *increasing with n* (e.g. `peak(sum, 50) > peak(sum, 5)`), proving the machine genuinely distinguishes tail from non-tail rather than never pushing frames.

**Deferred TCE cases (design spec §11.4), and why:** the *cross-module* (multi-file) mutual-recursion case needs a real multi-file module system (not yet built — §9); the *tail-through-resume*, *tail-in-handler*, and *composed tight-ceiling* cases need effects/handlers (Slice 3). Slice 2 delivers the self- and mutual-tail-recursion assertions above; the remaining §11.4 cases turn green in Slice 3.

---

## 5. Pipeline and Module Changes

- **New module `types`** (layer 4 in the pinned layering): the `Type`/`Scheme` representation, union-find unification, the inference walk, SCC grouping, generalization/instantiation, and the type reporter (zonk + letter-naming). Public entry: `types::infer(&Session, &Module) -> Vec<Diagnostic>` (and, for tests, an entry returning the inferred top-level schemes).
- **`eval` module** (layer 6): gains a `cek` submodule (the machine) alongside a `tree` submodule (the retained oracle), sharing `Value`/`Env`/`RuntimeError`. `Env` moves from Slice-1's clone-and-push `Vec<HashMap>` to the persistent `Rc`-scope of §3.2. Public `eval::run_module` switches to the CEK machine; `eval::run_module_tree` is retained for cross-check.
- **Pipeline** (`lib.rs`): `run_source` becomes `parse → resolve → types::infer → eval(cek)`; `check_source` becomes `parse → resolve → types::infer`. Type errors (E04xx) surface in both and **stop the pipeline before evaluation**.
- **Type errors move from runtime to compile time.** Programs the Slice-1 evaluator would have rejected at runtime with "type error in binary operator" (E0300) are now rejected at compile time (E0400). The CEK machine's operator-application paths become **defensive internal invariants** (reachable only if the checker has a bug); `1 / 0` remains a genuine **runtime** error (the checker cannot catch it).
- **Layer map unchanged.** `types = 4`, `core = 5` (still unimplemented), `eval = 6` are already pinned in the Slice-1 architecture test; Slice 2 fills `types` and extends `eval`. The `tests/arch/layering.rs` DAG check continues to guard the boundaries.

---

## 6. Testing Strategy (Slice-2 additions)

- **Inference unit tests**: assert inferred schemes for `id : ∀a. a -> a`, `const : ∀a b. (a,b) -> a`, `pick : ∀a. (Bool,a,a) -> a`, and monomorphic functions; assert `id` used at `Int` and `String` in one program type-checks (instantiation), and that mutual-recursion groups type correctly.
- **Type-error UI fixtures** (`//~ ERROR[E0400|E0401|E0402|E0403]`): mismatch, occurs-check, arity, non-`Bool` condition — each asserting readable type names and the **no-`%t` invariant** (§2.8).
- **Type snapshot tests** (`insta`): pretty-printed inferred schemes for the example programs.
- **CEK cross-check**: for every example + a corpus of small programs, `assert_eq!(cek_output, tree_output)`.
- **TCE regression**: the four assertions of §4.4 (bounded self- and mutual-tail-recursion, existence, grow-control), upgrading Slice-1's grow-only harness to the real bounded guarantee.
- **Regression**: all Slice-1 tests still pass, except those that fed deliberately ill-typed input to the evaluator, which are re-pointed at the type checker (now a compile-time E0400 rather than a runtime E0300).
- **Gate unchanged**: `scripts/check.sh` (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all`) stays green; the module-layering and no-remote/local-only conventions are unchanged.

---

## 7. Build Order (vertical sub-slices for the eventual plan)

Each sub-slice ends green and demoable, mirroring the Slice-1 discipline:

1. **2a** — `Type`/`Scheme` + union-find unification + occurs-check (unit-tested in isolation).
2. **2b** — inference walk over expressions with **monomorphic** top-level functions; wire `types::infer` into `check_source`; first E0400/E0403 fixtures.
3. **2c** — generalization + instantiation + **SCC** grouping → polymorphism (`id`/`const` tests); E0401/E0402 fixtures; the zonk/letter-naming reporter.
4. **2d** — persistent `Env`; the **CEK machine** for the current constructs; the tree-walker cross-check goes green.
5. **2e** — **TCE** instrumentation and the §4.4 bounded assertions.
6. **2f** — **minimal `case`**: parser + AST node, typing (§2.7), CEK `CaseArms` frame, cross-check + fixtures. *(Cut this sub-slice if `case` is deferred per the §1.2 scope note.)*

---

## 8. Risks & Mitigations

- **Unification cycles / infinite types** → occurs-check on every `Var` bind (E0401), property-tested.
- **`Env`/`Kont` growth defeating TCE** → persistent replace-don't-nest `Env` (§4.3) and no-push application (§4.2); the §4.4 bounded assertion is the guard.
- **CEK ≠ tree-walker divergence** → the cross-check test on every program catches any behavioral drift the moment it appears.
- **Type-error quality debt** → the zonk-before-print discipline and the no-`%t` UI invariant are specified and tested from sub-slice 2b, not bolted on.
- **Scope creep** → `case` is minimal and flagged as cuttable; ADTs/traits/lambdas/effects are explicitly out (§9).
- **SCC/recursion subtlety** → monomorphic-within-SCC is the standard, well-understood treatment; polymorphic recursion is explicitly unsupported and flagged.

---

## 9. Deferred / Honestly-Flagged (consolidated)

| Deferred item | Where it lands | Why it's safe to defer |
|---|---|---|
| Effect-row typing & enforcement (`/ {IO}`) | Slice 3 | Slice 2 types values/functions; effect rows are parsed and retained by name. Row-polymorphic inference is Slice 3's headline. |
| ADTs, enums, records, constructor patterns, exhaustiveness | Slice 3 | `Con` already carries an argument list; the type machinery is ready. Minimal literal-`case` (§2.7) needs none of it. |
| User generic *types* (`Tree(a)`) / traits | Slice 3 | Parametric polymorphism over functions (via generalization) exists in Slice 2; generic *data* and ad-hoc polymorphism build on it later. |
| Lambdas / first-class function *expressions* | Slice 3 (or a dedicated sub-slice) | HM polymorphism is already demonstrable via top-level functions; `Closure` values and application are built now, so adding lambda syntax later is additive. |
| Checked type-annotation syntax + surface `Type` AST | Slice 3 | Annotations are parsed-and-discarded (Slice-1 behavior). They reference ADT type names, which arrive in Slice 3. |
| Separate Core IR / pattern-match compilation | Slice 3 | The CEK machine runs on the AST directly; Core IR pays off with ADTs. |
| Polymorphic recursion | Later (needs annotations) | Undecidable without annotations; monomorphic recursion within an SCC is the standard treatment. |
| Handlers / `resume` / multi-shot | Slice 3 | The persistent `Env`/`Kont` representation is built now precisely so this is additive, not a rewrite (§3.2). |
| Multi-file modules; cross-module TCE case | Slice 3 (with the module/package work) | Slice 2 tests self- and mutual-tail-recursion within one file, which exercises the same tail-call mechanism. |
| Float ordering operators (`<.` etc.), `%`-on-float | Slice 3 | Lexed already; typing them is trivial to add and not needed for Slice-2 goals. |

**Manifesto check:** nothing here competes with the design spec's manifesto. Slice 2 keeps purity-by-default (no new side-effecting constructs), keeps "no subtyping" (HM + parametric polymorphism only), and prepares — but does not pre-empt — the effects bet. Where Slice 2 leans on something unbuilt, it is the *absence* of a check (effect rows) or a feature (ADTs), never a contradiction of the design.

---

## 10. Milestone Checklist (Slice 2)

- [ ] `Type`/`Scheme` + union-find unification + occurs-check (E0401).
- [ ] Inference walk; monomorphic top-level typing wired into `check_source`; E0400/E0402/E0403.
- [ ] Generalization + instantiation + SCC grouping → `id`/`const`/`pick` polymorphism.
- [ ] Type reporter: zonk + letter-naming; no-`%t` UI invariant tested.
- [ ] Persistent `Env`; CEK machine for all current constructs; **cross-check `cek == tree` green**.
- [ ] TCE instrumentation; §4.4 bounded assertions (self + mutual tail recursion) green; grow-control retained.
- [ ] Minimal `case` (parse + type + CEK + fixtures) — *or* explicitly cut per §1.2.
- [ ] Pipeline switched to `parse → resolve → infer → cek`; type errors compile-time; all Slice-1 tests green (adjusted); full gate (`scripts/check.sh`) green.

---

*Slice 2 turns the interpreter typed and the evaluator into a machine — the two changes that make Slice 3's effects possible — while proving tail-call elimination as a measured guarantee rather than a hope. Review gate: this spec is for your approval before any implementation plan is written.*
