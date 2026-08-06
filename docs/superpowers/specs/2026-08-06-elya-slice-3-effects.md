# Elya — Slice 3 Design Specification: Algebraic Effects & Handlers

- **Codename:** Elya
- **Slice:** 3 — the signature feature
- **Status:** Design draft — awaiting review before an implementation plan is written
- **Date:** 2026-08-06
- **Depends on:** Slice 2 (merged to `main`, `2a53148`) — the HM-typed interpreter on the CEK machine, with the persistent `Rc`-frame continuation this slice's `resume` captures.
- **Companion documents:** the language design spec (`.../2026-08-05-elya-language-design.md`, "design spec"), the Slice 2 spec (`.../2026-08-06-elya-slice-2-types-and-cek.md`, "Slice 2 spec").

---

## 0. How to read this document

This is a **design spec** for Slice 3 — algebraic effects & handlers with **row-polymorphic effect inference**, Elya's radical bet (design spec §1, §8). It fully specifies: the effect-row type system and its inference, the handler/`resume` operational semantics **on the exact CEK machine Slice 2 built**, the effect-row diagnostics (same zonk-before-print, no-`%`-gibberish discipline), and the tail-through-resume / handler TCE guarantees as testable assertions. Every deferral is flagged in §11.

Section refs: "design spec §X" → the language design spec; "Slice 2 spec §X" → the Slice 2 spec; bare "§X" → this document. Diagnostic codes continue the scheme: `E042x` are **reserved for effects** (design spec §9) and are defined here.

**Honest framing up front:** Slice 3 is the largest slice — it touches the AST (one mechanical change, §7.1), the parser, the type system (a second union-find, for rows), and the machine (handler frames + continuation capture). §9 decomposes it into sub-slices, and it may warrant a plan per sub-slice. It is scoped to effects **over the current Slice-2 surface** (base types, top-level functions); generic effects, effects over ADTs, and an async runtime are deferred (§11).

---

## 1. Scope

### 1.1 What Slice 3 delivers

1. **Effect declarations** and **effect rows in function types**, with **row-polymorphic effect inference** — a conservative extension of Algorithm J that adds a second union-find over *rows*. Effect composition without subtyping (the "no subtyping" decision preserved, §3.6).
2. **Handlers and `resume`** on the CEK machine: `handle e with { … }`, deep-handler semantics, `resume` as a first-class captured continuation. **One-shot by default**, `with multi` for multi-shot — one-shot **enforced**, not assumed.
3. **Effect-row diagnostics** (`E0420`–`E0426`) meeting the design-spec §9 discipline: discharge checking is a *separate pass* from unification, rows are **zonked** and printed as **named labels + a readable residual**, never a raw row variable.
4. **TCE through effects**: tail-resumptive handlers and tail calls in handler clauses run in **bounded continuation depth**, asserted as pinned-constant regression tests (the §11.4-of-design-spec cases that Slice 2 deferred).

### 1.2 Surface additions

Over Slice 2's surface (literals, `Int`/`Float` arithmetic, `<>`, comparisons, `let`, `if/else`, top-level functions, calls, `io.println`), Slice 3 adds:

```elya
// effect declaration — operations are interfaces with base-typed signatures
effect Log {
  fn log(msg: String) -> Unit
}

// a function that performs Log; the row is written after the params
fn greet(name: String) / {Log} -> Unit {
  log("hi " <> name)          // operation call = perform
}

// handle installs a handler over a computation; `resume` is the continuation
pub fn main() / {IO} {
  handle greet("ada") with {
    Log.log(m)  -> { io.println(m)  resume(Unit) }   // deep handler; resume continues
    return(x)   -> x
  }
}
```

New syntax: `effect NAME { fn op(params) -> ret … }`; the effect row `/ { E1, E2 }` on function signatures (already **parsed and discarded** since Slice 1 — Slice 3 makes it meaningful); `handle EXPR with [multi] { OpClause… ReturnClause? }`; operation calls (an in-scope operation name applied like a function); and `resume(v)` inside handler clauses.

### 1.3 What Slice 3 does NOT do (deferred — §11)

- **Generic/parametric effects** (`State(s)`, `Choice { fn choose(xs: List(a)) -> a }`) — effect operations are **monomorphic over base types** here (`State { fn get() -> Int; fn put(v: Int) -> Unit }`). Effect *rows* are polymorphic (the headline); operation *payloads* are not, until ADTs/generics land.
- **ADTs, generics, traits, lambdas** — still deferred (Slice 2 §9). Row polymorphism is demonstrated without them via effect-polymorphic top-level functions (§3.5).
- **`async`/concurrency runtime, `Yield`/generators as library effects** — the *mechanism* supports them; the *library* + scheduler are later work.
- **Shallow handlers, first-class effect values, effect aliases** — deep handlers only.

---

## 2. Effects in the language, informally

- An **effect** is a named set of **operations**; each operation has a base-typed signature `fn op(A…) -> B`.
- **Performing** an operation (calling `op(args)` where `op` is in scope from an `effect` decl) is the only impure act; a function that (transitively) performs `op` has `op`'s effect in its row.
- A **handler** `handle e with { Op(x) -> body … return(x) -> body }` runs `e`, intercepting each performed operation with the matching clause. The clause receives the operation's argument(s) and a continuation **`resume`**; calling `resume(v)` returns `v` as the operation's result and continues `e` from the perform point. The **`return` clause** transforms the final value of `e` (defaults to identity).
- **Deep handlers:** the handler stays installed around the resumed continuation, so later operations in `e` are caught by the same handler. `resume : (B) -> R` where `R` is the whole `handle`'s result type.
- **Built-in effects:** `IO` is tracked in rows but discharged **natively** by the runtime (no user handler needed) — `io.println : (String) / {IO} -> Unit`, and `main` may perform `{IO}`. User effects require a handler or they are unhandled (`E0420`).

`try`/`throw` is an `Exn` effect whose clause never calls `resume`; state is a `State` effect; nondeterminism is a multi-shot handler — one mechanism, many features (design spec §8).

---

## 3. The Effect-Row Type System

### 3.1 Representation

The arrow grows a row (the exact extension the Slice-2 spec anticipated — Slice 2 spec §2.1):

```
Ty      ::= Var(TyVar) | Base(TyCon) | Fn([Ty], EffectRow, Ty) | Tuple([Ty]) | Error
EffectRow ::= { labels: BTreeMap<EffectLabel, Provenance>, tail: RowTail }
RowTail   ::= Closed | Open(RowVar) | ErrorRow
EffectLabel = effect name (Slice 3: a bare name; generic effects would add type args)
```

- **Closed** row = exactly these labels, no more. **Open(ρ)** row = these labels *plus whatever ρ is* (row polymorphism). **Pure** = `{ }` **Closed** (empty, closed).
- Rows are **idempotent simple rows**: each label appears at most once (design spec §8.2). No label ordering is semantically significant.
- **Provenance** on each label records the operation call-site that introduced it and the boundary it crossed — load-bearing for diagnostics (§5), exactly as the design spec §9 requires.
- `Infer` gains a **second union-find** `row_subst: Vec<Option<EffectRow>>` alongside the type `subst`; `RowVar(u32)` indexes it. A fresh open row is `{ } | Open(fresh RowVar)`.

### 3.2 Row unification (row rewriting)

`unify_row(r1, r2, span)` follows the standard simple-row algorithm (Rémy / Leijen; Koka):

1. Resolve both rows' tails through `row_subst`.
2. `common = r1.labels ∩ r2.labels` — matching labels are compatible (idempotent). *(When generic effects land, unify their type args here; a payload conflict is `E0423`.)*
3. `only1 = r1.labels \ r2.labels`, `only2 = r2.labels \ r1.labels`.
4. To reconcile, **rewrite each row's tail to expose the other's extra labels**:
   - `only1` must be absorbed by `r2.tail`. If `r2.tail = Open(σ)`, bind `σ := { only1 } | Open(σ')` (fresh `σ'`). If `r2.tail = Closed` and `only1` is non-empty → **cannot absorb** → `E0421`/`E0423` (§5), *not* a raw unification dump.
   - Symmetrically, `only2` into `r1.tail`.
5. Unify the two residual tails (`Open(σ') ~ Open(τ')` → bind; `Closed ~ Closed` → ok; `Open ~ Closed` → close the open one; `ErrorRow` unifies with anything). **Occurs-check** row variables (a tail may not contain itself) → `E0424`.

Because open rows reconcile by *mutual extension*, two open rows never fail by "neither is a subset" — failures are only *closing conflicts* (a required-closed row meeting an extra label) or *occurs-check* (design spec §9's corrected analysis). Both have purpose-built diagnostics.

### 3.3 Effect-annotated inference (ambient-row threading)

Inference gains an **ambient effect row** — "the effects the current context is allowed to perform" (Koka's approach). `infer_expr(e, env, amb)` infers `e`'s type and unifies every effect `e` performs *into* `amb`:

| Expression | Rule |
|---|---|
| literal / `Var` (value) | type as Slice 2; **no** effect added to `amb`. |
| operation `Op(args)` (an effect op in scope) | infer args (into `amb`); `add_effect(amb, Op, span)`; result = `Op`'s declared return type. |
| builtin `io.println(arg)` | infer arg; `add_effect(amb, IO, span)`; result `Unit`. |
| call `f(args)`, `f : ([A], ε_f, R)` | infer callee + args into `amb`; **unify `ε_f` into `amb`** (`unify_row(amb, {} | Open(fresh)` extended by `ε_f`… see `add_row`); result `R`. |
| `if`/`let`/block | thread the *same* `amb` through sub-expressions (sequencing unions effects); types as Slice 2. |
| `handle e with H` | §3.4. |

`add_effect(amb, Op, span)` = `unify_row(amb, { Op@span } | Open(fresh))` — forces `Op ∈ amb`, rewriting `amb`'s tail to expose `Op`. `add_row(amb, ε)` folds all of `ε`'s labels in the same way (used to pour a callee's row into the ambient).

A **function definition** `fn f(params) / <decl?> -> ret { body }`: create the function's row `ε_f` (the annotation if present, else a fresh **open** row); infer `body` with `amb = ε_f`; unify body type with `ret`. If annotated, `ε_f` is what was written — **exact match, no sub-effecting** (declaring an effect you never perform is `E0423`; §3.6). If unannotated, `ε_f` is inferred and **generalized** (§3.5), typically open → row-polymorphic.

### 3.4 Typing `handle`

For `handle e with H` where `H` handles effect `E` (the effect whose operations `H`'s clauses cover):

- Infer `e` with a **fresh inner ambient** `amb_in = { E } | Open(ρ)` — `e` may perform `E` and a polymorphic remainder `ρ`.
- Each **operation clause** `Op(x) -> body` (for `Op : (A) -> B ∈ E`): bind `x : A` and `resume : ([B], amb_out, R)` — resuming yields the operation's result `B` and continues to the handler's result `R`, performing the handler's *outer* effects `amb_out`. Infer `body` with `amb = amb_out`; unify `body : R`.
- The **return clause** `return(x) -> body`: `x : τ_e` (type of `e`); infer `body : R` with `amb = amb_out`. Absent ⇒ identity, `R = τ_e`.
- **Discharge:** `E` is removed. Unify `amb_out ⊇ Open(ρ)` (the residual `ρ` of `e` passes through) and the whole `handle : R / amb_out`, unified into the enclosing ambient.

So `handle` **subtracts** the handled effect and passes the rest through — the composable, row-polymorphic story. `resume`'s type falls out of the clause typing above; it is an ordinary function value whose latent effect is `amb_out` (deep handler).

### 3.5 Row polymorphism without lambdas

Row polymorphism is observable in Slice 3 even without higher-order lambdas, because top-level functions are first-class values (`Value::Fn(name)`, Slice 2) and can be **parameters**:

```elya
fn run_it(g) { g() }        // inferred:  fn(fn() / e -> a) / e -> a   — polymorphic in row `e`
```

`run_it` is generalized over both the type var `a` and the **row var** `e`: its effect equals its argument's effect. `run_it(some_logging_fn)` performs `{Log}`; `run_it(pure_fn)` is pure. This is parametric row polymorphism — no subtyping (§3.6). **Generalization** quantifies free row variables (not free in the environment) exactly as Slice 2 does for type variables; `display_scheme` prints them (§5).

### 3.6 "No subtyping" preserved

A function that "does less" is handled by **row-variable instantiation**, not sub-effecting: a caller expecting `fn() / {IO} -> a` accepts `run_it` by instantiating `run_it`'s `e := {IO}`. There is no rule "`{}` ≤ `{IO}`". Sub-effecting was rejected (design spec §8.2) precisely because it is subtyping; row polymorphism gives composition without it. Consequence, stated honestly (§11): an **annotated** function must perform *exactly* its declared row — declaring an unused effect is `E0423`, not silently widened. In practice this bites mostly on explicit *public* signatures (unannotated functions have their rows inferred).

**`E0423` severity is revisitable (strict-by-default, not welded shut).** The error-vs-warning severity for the declared-more-than-performed case is a deliberate strict default. If real use shows exact-match is too strict, relaxing it to *warn, don't error* is **additive and stays subtyping-free** — tolerating an over-declared annotated row is not sub-effecting (it introduces no `{} ≤ {IO}` rule and no coercion; it merely accepts a row the programmer wrote but did not use). Chosen strict now; revisit on evidence.

### 3.7 Honest deferrals in the type system

- **Monomorphic operations:** operation signatures use base types only; generic effects deferred (§11).
- **No effect subtyping / masking / injection** beyond row-variable polymorphism.
- **`main`'s discharge set** is `{IO}` (natively discharged); any user effect surviving to `main` is `E0420`.

---

## 4. Handlers & `resume` on the CEK machine

This section shows how `resume` **captures the persistent `Rc`-frame `Kont` Slice 2 built** — the reason that representation was chosen (Slice 2 spec §3.2).

### 4.1 One prerequisite: `Rc`-share the AST (mechanical)

Slice 2's machine frames borrow `&'a` AST nodes. A `resume` value is a **first-class `Value`** bound in a handler-clause environment and called later, so a captured continuation must not be tied to a borrow. Slice 3 makes the machine's AST references **owned and cheaply clonable** by changing the recursive AST positions from `Box` to `Rc` (`Expr` children, `Block` tail, `FnDecl` body). The machine then holds `Rc<Spanned<Expr>>`; frames own `Rc` clones; `Value::Resume` is lifetime-free and `'static`-capable, and **`Value`/`Env` stay lifetime-free and shared with the tree-walker**. This is exactly the "mechanical Slice-3 step" the Slice 2 spec flagged (Slice 2 spec §3.8). It touches `ast.rs` + `parse.rs` (`Box::new` → `Rc::new`) only; all deref sites are unchanged.

### 4.2 New frame, new value

```
Frame  ::= … (Slice-2 frames, now holding Rc AST) …
         | HandleK { handler: Rc<Handler>, ret_env: Env }   // installed by `handle`
         | OpArgs  { effect, op, done, pending, env, span }  // evaluating an operation's args

Value  ::= … | Resume(Rc<ResumeData>)
ResumeData { captured: Vec<Frame>, handler: Rc<Handler>, one_shot: Cell<bool>? }
```

`Kont` is unchanged: `Option<Rc<KontNode { frame, rest }>>` — the persistent, immutable, `Rc`-linked stack from Slice 2. `Frame` is already `Clone` (Slice 2 derived it for the `Rc::try_unwrap` fallback); that `Clone` is what makes multi-shot capture sound.

### 4.3 `handle`, perform, capture

- **`handle e with H`**: push `HandleK{H}` onto `Kont`, evaluate `e`. (The handler `H` is `Rc`-shared from the AST.)
- **Perform `Op(v)`**: evaluate args (via `OpArgs`), then search the `Kont` **from the top** for the nearest `HandleK{H}` whose `H` handles `Op`. This walk splits the persistent stack:
  - **`k_cap`** = the frames *above* that `HandleK` (from the perform point up to it). Because `Kont` links **downward** (each node's `rest` is below), these "above" frames are a *prefix* and are **materialized into a `Vec<Frame>` by cloning** during the walk — cheap: it is only the frames between the perform and the handler (for a `get()` in a loop, a handful).
  - **`k_rest`** = the `HandleK` node's `rest` — a *suffix*, so it **is** a `Kont` already (no copy, just the `Rc` pointer).
  - Run `H`'s clause for `Op` with `x := v`, `resume := Value::Resume(ResumeData { captured: k_cap, handler: H, one_shot })`, and **continuation `k_rest`** (deep handler: the clause result flows to the handler's consumer).

Materializing `k_cap` as an owned, cloned `Vec<Frame>` is the crux: it is an independent, immutable snapshot of a slice of the persistent `Kont`, so it can be re-entered **any number of times**.

### 4.4 `resume`

Calling `Value::Resume(rd)` with a value `u`:

1. **One-shot check:** if `rd.one_shot` is present and already consumed → runtime `E0425` ("continuation resumed twice; handler is one-shot — use `with multi`"), with both spans. Otherwise mark consumed.
2. **Rebuild the `Kont`:** deep-handler semantics re-install the handler beneath the captured frames. Fold `rd.captured` (the `Vec<Frame>`, immutable) back on top of `HandleK{rd.handler}` on top of the resume call's current continuation `k_now`:
   `Kont' = k_cap ++ [ HandleK{H} ] ++ k_now`
   (built by pushing frames onto `Kont`; each push is one `Rc::new(KontNode…)`).
3. Continue: `State::Return(u, Kont')`.

Multi-shot (`with multi`, no `one_shot` flag) simply performs steps 2–3 again on a later call — the same immutable `rd.captured` re-pushed independently. Nothing is mutated; each resumption is an independent run. **This is the payoff of the persistent, `Clone`-able `Kont`/`Frame` design from Slice 2.**

### 4.5 Worked traces

**Exception (never resumes):**
```elya
effect Exn { fn throw() -> Unit }
fn safe_div(a: Int, b: Int) / {Exn} -> Int { if b == 0 { throw()  0 } else { a / b } }
handle safe_div(10, 0) with { Exn.throw() -> 0-1 ; return(x) -> x }   // => -1
```
`throw()` performs; the clause returns `-1` to `k_rest` and never calls `resume` — the captured `k_cap` is dropped. This is `try`/`catch` as a handler.

**State (tail-resumptive, one-shot):**
```elya
effect State { fn get() -> Int ; fn put(v: Int) -> Unit }
// interpreted by threading the state through resume (parameter-passing handler)
```
Each `get`/`put` performs; the clause `resume(...)` in **tail position** continues the loop. §6 shows this runs in bounded `Kont` depth.

**Multi-shot (base types only — no lists needed):**
```elya
effect Flip { fn flip() -> Bool }
fn choose_greeting() / {Flip} -> String { if flip() { "hello" } else { "bye" } }
handle choose_greeting() with multi {
  Flip.flip() -> resume(True) <> "/" <> resume(False)   // resume called TWICE
  return(x)   -> x
}   // => "hello/bye"
```
`resume(True)` re-enters `k_cap` producing `"hello"`; `resume(False)` re-enters the **same** immutable `k_cap` producing `"bye"`; they are concatenated. Two independent runs of one captured continuation — impossible without the persistent `Kont`.

### 4.6 The tree-walker oracle and effects

The Slice-1 tree-walker recurses on the *host* stack and **cannot** capture continuations, so it cannot run multi-shot (or general) handlers. Therefore:

- The tree-walker remains the oracle **only for the pure (effect-free) subset**; the Slice-2 cross-check narrows to effect-free programs.
- **Effect programs are tested by golden outputs + targeted semantic assertions** (§8), since no host-stack walker can serve as their oracle. This is an honest consequence of adding first-class continuations, flagged in §11.

### 4.7 Honest deferrals in the machine

- **Deep handlers only** (no shallow).
- **`k_cap` materialization clones frames** per perform — bounded by the perform-to-handler distance; acceptable, and noted as the one non-`O(1)` step per operation.
- **No Core IR** still (design spec deferral); the machine steps the `Rc`-AST directly.

---

## 5. Effect-Row Diagnostics (`E042x`)

Same discipline as Slice 2's type errors and the design spec §9: **discharge is a separate pass from unification; zonk before printing; never emit a raw row variable.** An `EffectRow` renders as its **named labels** plus, if the tail is an open variable that is genuinely polymorphic and relevant, a readable **`| …` residual** — never `%r7`.

- **`E0420` — effect never handled.** A dedicated **discharge pass** over the typed program (not the unifier): at each `handle` and at `main`, compute the residual row; any user label not discharged (and, at `main`, anything outside `{IO}`) is reported, naming the effect and the operation call-site (from provenance) and offering a fix-it (`handle … with { Op(..) -> … }` or widen the row).
- **`E0421` — must be pure here.** A closed empty row meeting a performed effect (e.g. an argument required pure). Rendered "this must be pure but performs `Log`", not a row-unification dump.
- **`E0423` — effect-row mismatch.** Two rows that cannot reconcile (annotation says `{Log}` but body also performs `Net`; or a closed row meets an extra label). Reports **the differing labels as a set** ("differs by `{Net}`"), never a tail variable. (Generic-effect payload conflicts reuse this code, printing the effect name once + the two zonked payload types.)
- **`E0424` — cyclic effect row** (row occurs-check): "an effect row would contain itself", naming the offending label's origin.
- **`E0425` — resumed more than once** in a one-shot handler: **static** where syntactically detectable (two `resume` calls in a non-`multi` clause), else a **runtime** structured error; both name the clause and point at `with multi`.
- **`E0426` — cleanup may run more than once** (double-free lint) under a `multi` handler (design spec §8.6). Best-effort lint; full guarantee awaits linear types (§11).

Illustrative fixtures (these become UI tests; each asserts named labels appear and **no `%r`/`%e`/`%row` token** does — the same invariant Slice 2 enforced for `%t`):

```
error[E0420]: effect `Log` is never handled
  ┌─ app.elya:8:3
4 │   log("hi " <> name)         ─── `Log` is performed here (inside `greet`)
8 │   greet("ada")               ^^^^^^^^^^^^ escapes here; `main` may perform only {IO}
  = help: handle it → handle greet("ada") with { Log.log(m) -> ... }
```
```
error[E0423]: effect row mismatch: `sync` performs more than it declares
5 │ fn sync(url) / {Log} -> Unit {    ───── declared to perform only {Log}
6 │   let body = fetch(url)           ^^^^^^^^^^ this performs `Net`, not in the declared row
  = the rows differ by exactly:  {Net}
```

---

## 6. TCE Through Effects (testable)

Slice 2 proved bounded-depth TCE for ordinary tail calls (`peak_kont_depth ≤ K_MAX = 3`). Slice 3 delivers the design-spec §11.4 cases it deferred: **tail-resumptive handlers** and **tail calls in handler clauses**.

### 6.1 Machine rules

- **Tail call in a handler clause** — as in any body, an application in tail position swaps `control` without pushing a frame (Slice 2 §3.5); the clause's continuation `k_rest` is reused. `O(1)`.
- **Tail-resumptive `resume`** — when a clause's tail is `resume(x)`, the machine detects the tail position and, instead of re-pushing `k_cap` onto a *growing* stack, **splices `k_cap` onto the clause's own continuation `k_rest` (with the handler re-installed) in place** — no net growth per operation. This is the "tail-resume splices into the current slot" rule (design spec §8.5). A non-tail-resumptive clause (work after `resume`) legitimately grows and is out of scope for the guarantee.

### 6.2 Testable assertions (pin `K_MAX` from first measurement, per Slice 2's rule)

Using a tail-resumptive `State` handler that threads the counter through `resume`:

1. **Tail-resumptive State loop is bounded.** A `count_down(1_000_000)` driven entirely through `get`/`put` under a tail-resumptive handler runs at `peak_kont_depth ≤ K_MAX_EFF` (a pinned single/low-double-digit constant). A per-operation frame leak drives the peak toward 1,000,000 and fails; **do not raise the constant to hide a regression — fix the machine** (Slice 2's discipline).
2. **Tail call in a handler clause is bounded.** A handler whose clause ends in an ordinary tail call, iterated 1,000,000 times, is bounded.
3. **Existence + grow control.** The deep run completes (a host-stack walker would overflow — and the tree-walker is correctly not asked); a non-tail-resumptive counterpart's peak **grows** with the operation count, proving the machine distinguishes the tail case.

**Deferred TCE case (honest):** the design-spec §11.4 *cross-module* mutual-recursion case still needs multi-file modules (not built — §11); Slice 3's TCE tests are single-file, exercising the same tail-resume/tail-call machinery.

---

## 7. Pipeline & Module Changes

### 7.1 AST (`ast.rs`, `parse.rs`) — mechanical
- **`Box` → `Rc`** in `Expr`'s recursive positions, `Block.tail`, `FnDecl.body` (§4.1). Parser `Box::new` → `Rc::new`. `Spanned`, pretty-printer, resolver, types unaffected (deref-only).
- New nodes: `EffectDecl { name, ops: Vec<OpSig> }`, `OpSig { name, params, ret }`; `Expr::Handle { body, handler }`, `Handler { clauses: Vec<OpClause>, ret: Option<ReturnClause>, multi: bool }`, `OpClause { effect, op, params, resume_binder, body }`; `Expr::Resume { arg }` (or resume as a callable binding — decided at plan time). The effect-row on `FnDecl` (currently `Vec<String>` of names) is elaborated to a real `EffectRow` reference during type-checking.

### 7.2 Resolver (`resolve.rs`)
- Register effect declarations; resolve operation names to `(effect, op)`; resolve `resume` only inside handler clauses (else `E020x`). Handler `Op` clauses must name operations of a single declared effect.

### 7.3 Types (`types.rs`)
- Add the **row union-find** (`row_subst`, `RowVar`), `EffectRow`, `unify_row`, `add_effect`/`add_row`, row generalization/instantiation, the **discharge pass** (`E0420`/`E0421`), and effect-row rendering (zonk + named labels). `Ty::Fn` gains the row. Inference threads the **ambient row** (§3.3). Builtins re-typed with effects (`io.println : (String) / {IO} -> Unit`).

### 7.4 Machine (`eval.rs`)
- Machine frames hold `Rc` AST. Add `HandleK`, `OpArgs`; add `Value::Resume`; implement perform/capture/`resume` (§4). One-shot enforcement. The **tree-walker is restricted to effect-free programs** (§4.6).

### 7.5 Pipeline / layering
- `run_source`/`check_source` unchanged in shape (`parse → resolve → infer → cek`); inference now includes the discharge pass. **Layer map unchanged** (`types = 4`, `eval = 6`); `tests/arch/layering.rs` still guards it.

---

## 8. Testing Strategy

- **Row-inference unit tests:** `run_it : fn(fn() / e -> a) / e -> a` (row-poly); `greet : fn(String) / {Log} -> Unit`; effect-row snapshots (`insta`) of inferred schemes including rows.
- **Effect-error UI fixtures** (`//~ ERROR[E0420|E0421|E0423|E0424|E0425]`): unhandled effect, purity violation, declared-vs-actual mismatch, cyclic row, double-resume — each asserting named labels and **no `%r`/`%row`** token.
- **Handler-semantics golden corpus — the *only* backstop, so coverage is deliberate, not happy-path.** §4.6 removed the tree-walker oracle for effect programs, which *raises the bar*: golden outputs are the sole safety net, so the corpus is designed to exercise **every distinct handler behavior**, each an output-verified program, not just demos:
  - **(a) non-resuming / `Exn`-style** — a clause that never calls `resume` (captured continuation dropped);
  - **(b) one-shot resume** — resume exactly once (the common case);
  - **(c) multi-shot re-invocation** — `resume` called **more than once**, results combined (over `String`);
  - **(d) nested handlers** — two handlers in scope; the inner discharges its effect, the outer catches the residual; correct **innermost-matching**;
  - **(e) tail-resumptive** — `resume` in tail position (the §6 loop).
  Happy-path examples alone are explicitly insufficient; each row (a)–(e) is a required corpus entry.
- **Cross-check narrowed** to effect-free programs (`cek == tree`), still green.
- **TCE-through-effects** regressions (§6): bounded tail-resumptive state loop + tail-in-handler, pinned `K_MAX_EFF`, plus a grow control.
- **One-shot enforcement negative tests:** static `E0425` fixture + a runtime double-resume test.
- **Gate unchanged:** `scripts/check.sh` (fmt + clippy `-D warnings` + all tests) green; push to `origin/main` per the standing workflow.

---

## 9. Build Order (sub-slices — likely a plan per sub-slice)

1. **3a — AST `Box`→`Rc` + effect syntax** (parse `effect`, rows, `handle`/`with`/`multi`, `resume`; resolver). Pure plumbing, **behavior-preserving**. **Explicit gate: the entire existing test suite stays green after 3a — before any effect *semantics* land.** The `Box`→`Rc` change is deref-only downstream; the new syntax is parsed + resolved but not yet type-checked or evaluated. This is the isolated, low-risk change to already-shipped code, landed first behind the full suite.
2. **3b — effect-row types + inference + discharge** (row union-find, `unify_row`, ambient threading, `E0420`–`E0424`, row-poly `run_it`, snapshots). No machine changes; `check_source` type-checks effects.
3. **3c — handlers & one-shot `resume` on the CEK machine** (`HandleK`, perform/capture, deep-handler `resume`, exception + state golden tests; one-shot `E0425`).
4. **3d — multi-shot** (`with multi`; `E0426` cleanup lint). **Gate: a working, output-verified multi-shot program** — `resume` re-invoked and results combined (over `String`), producing the expected output. This is the actual cash-out of the "multi-shot needs no design change" claim from the effect-design review (design spec §8.4) — *demonstrated running*, not merely "the representation supports it."
5. **3e — TCE through effects** (tail-resume splice rule; §6 bounded assertions; grow control).

### Exit criterion
The §8 handler-semantics golden corpus — non-resuming, one-shot, **multi-shot re-invocation (output-verified)**, nested handlers, tail-resumptive — all pass; row-poly inference + `E042x` fixtures green; tail-resumptive state loop bounded at a pinned constant; effect-free cross-check still green; full gate green and pushed to `origin/main`.

---

## 10. Risks & Mitigations

- **Row-unification bugs / gibberish errors** → separate discharge pass + zonked, named-label rendering + the no-`%r` UI invariant, specified from 3b.
- **Continuation-capture correctness (multi-shot)** → immutable `Vec<Frame>` capture + `Clone` frames (already in place); golden semantic tests (flip, state); the persistent `Kont` was built for this.
- **TCE regressing under resume** → the tail-resume splice rule + pinned-constant assertion (don't-raise-the-constant discipline).
- **AST `Rc` change rippling** → mechanical, deref-only downstream; land it first (3a) behind the full existing suite.
- **Scope (largest slice)** → sub-sliced (§9); generic effects / ADTs / async explicitly out (§11).
- **Deep-vs-shallow / resume typing subtlety** → deep handlers only, one composable typing rule (§3.4).

---

## 11. Deferred / Honestly-Flagged

| Deferred | Where it lands | Why safe |
|---|---|---|
| Generic/parametric effects (`State(s)`, `Choice(List(a))`) | with ADTs/generics | Monomorphic base-typed ops suffice to demonstrate row polymorphism + multi-shot (§4.5). `EffectLabel` is ready to carry type args. |
| ADTs / generics / traits / lambdas | later slices | Row polymorphism shown via effect-polymorphic top-level functions; multi-shot shown via `<>` over base types (no lists). |
| Async runtime, generators-as-library, scheduler | later | The *mechanism* (capture/resume) is here; the library + event loop are separate. |
| Shallow handlers, first-class effect values, effect aliases/masking | later | Deep handlers are the composable core. |
| Tree-walker as oracle for effect programs | n/a (permanent) | A host-stack walker cannot capture continuations; effect programs use golden tests (§4.6). |
| Cross-module TCE case (design-spec §11.4) | with multi-file modules | Slice 3 TCE is single-file; same tail-resume machinery. |
| Full cleanup-safety guarantee (`E0426` is a lint) | with linear/affine types | Design spec §8.6, §13; lint now, static guarantee later. |
| Effect-annotation *widening* (declare more than you perform) | not planned | Rejected: it is sub-effecting = subtyping (§3.6). Annotations match exactly. |

**Manifesto check (design spec §1):** this slice *is* the radical bet — effects as first-class, inferred, handler-resolved values. It keeps purity-by-default (only operations are impure), keeps "no subtyping" (row polymorphism, §3.6), and turns `try`/state/nondeterminism into library handlers over one mechanism. Nothing here competes with the manifesto; it delivers it.

---

## 12. Milestone Checklist (Slice 3)

- [ ] AST `Rc`-shared; `effect`/row/`handle`/`with multi`/`resume` parsed + resolved (existing suite green).
- [ ] Row union-find + `unify_row` (rewriting) + occurs-check (`E0424`).
- [ ] Ambient-row inference; builtins re-typed (`io.println / {IO}`); `run_it` row-poly; scheme+row snapshots.
- [ ] Discharge pass: `E0420` unhandled, `E0421` purity, `E0423` mismatch — zonked, named-label, no-`%r` UI fixtures.
- [ ] `HandleK` + perform/capture + deep-handler one-shot `resume`; exception + state golden tests; `E0425`.
- [ ] `with multi` + flip multi-shot demo; `E0426` cleanup lint.
- [ ] Tail-resume splice rule; bounded tail-resumptive state loop + tail-in-handler (`K_MAX_EFF` pinned) + grow control.
- [ ] Effect-free cross-check still green; pipeline unchanged in shape; full gate green; pushed.

---

*Slice 3 is Elya's soul: effects are typed, inferred without annotations, and handled by ordinary code — and `resume` is nothing more than a re-entry into the persistent continuation the CEK machine has carried since Slice 2. Review gate: this is the signature feature; the design is for your approval before any implementation plan is written.*
