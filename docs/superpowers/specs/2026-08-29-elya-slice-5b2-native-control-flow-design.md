# Slice 5b-2 — Native Control Flow: branches, Bool, and comparisons

**Goal.** Turn the back end from a straight-line fold into a real basic-block
emitter. An Elya source file containing `if`, comparisons, and boolean operators
becomes a native executable that runs and prints the right integer.

**Arc.** 5b is the native-compilation arc (the brainstorm's `N1`..`N8`). This
slice is arc node `N3`. The slice numbering and the arc numbering diverge here
and will keep diverging — 5b-1 was `N1`, 5b-2 is `N3` — because this slice is
deliberately taken before `N2` (calls). §1.2 argues that ordering.

**Non-negotiable boundary.** The slice is not done when codegen emits a `br`. It
is done when a process the compiler produced has exited `0` after writing the
result of a *branch it actually took* to stdout, and a test asserted that.

---

## §0 How to read this document

The rules from 5b-1 §0 carry over verbatim and are not restated in full:

- **Proof is execution.** No `insta` snapshot of LLVM IR, anywhere. A test that
  inspects IR instead of running a binary does not count. No test may skip.
- **Semantic fidelity.** Native codegen must never be *more*-undefined than the
  tree evaluator. This slice extends the rule in a direction 5b-1 did not have to
  face: native must also never be *less*-undefined in a way that changes an
  observable result. §4.1 is where that bites, and it costs the slice a scope
  item the user asked for.
- **Rejections are errors, not silent skips.** Every construct outside the subset
  returns `CodegenError::Unsupported` naming the construct.

### The decoy we are NOT building

The decoy is "add `If` to the codegen match, emit a `select`". `select` is not
control flow — it evaluates both arms. It would pass every test in §5 that has no
side effects and no divergence, and it would be a lie the moment `N2` lands calls
and a base case has to *not* recurse. This slice emits real basic blocks with a
real `phi`, because the whole point is the structural change the later slices sit
on.

---

## §1 Scope

### In

| Construct | Detail |
|---|---|
| `If` | mandatory two-branch expression; both branches yield a value |
| `Bool` | literals, and `Bool` as a *second* codegen value type |
| Comparisons | `Eq`, `Ne`, `Lt`, `Le`, `Gt`, `Ge` |
| Boolean operators | `And`, `Or` — **strict**, not short-circuiting. See §4.1 |

Everything from 5b-1 stays in: `Int` literals, `Var`, `Let`, `Add`, `Sub`, `Mul`,
the `printf` shim, object emission, the `clang` link step, `elya build`.

### Out (and why, briefly)

| Deferred | Why |
|---|---|
| `Div`, `Rem` | Not merely "needs a branch". §1.1 — they drag the runtime-error path. |
| Calls, multi-function modules | `N2`. Still one function named `main`, still zero parameters. |
| `Match`, ADTs, heap, GC | `N4`. Core lowers `Match` today; codegen still refuses it. |
| `Lambda`, closures | `N5`. |
| `Str`, real `io.println` | `N6`. This is what constrains `Eq` — §3.3. |
| `Float`, `AddF`..`DivF`, `Concat` | No float story yet; `Concat` is `N6`. |
| `Unary` (`Neg`, `Not`) | Core refuses `Expr::Unary` today. Not needed to prove branching. |
| Short-circuit `&&` / `||` | A **language-surface** change, not a codegen one. §4.1. |

### §1.1 Why `Div`/`Rem` are cut, precisely

The shallow reason is that a zero guard needs a branch, and this slice is the one
that adds branches. That reason is wrong, or at least not the binding one.

The real reason is what the guard has to *do*. The evaluator does not leave
division by zero undefined — it produces a diagnostic:

```rust
(Div, Int(_), Int(0)) => Err(rt(span, "division by zero")),    // src/eval.rs:228
(Rem, Int(_), Int(0)) => Err(rt(span, "remainder by zero")),   // src/eval.rs:230
```

Under semantic fidelity the native binary must fail the same way: a message on
stderr and a non-zero exit. That requires a string constant in the module, a
second external symbol (`abort` or `exit`), and a decision about whether the span
reaches the runtime at all. That is the **first breach of the one-external-symbol
convention** (5b-1 §3.5), and it is the seed of the runtime-error machinery that
`N4` needs anyway for pattern-match failure and allocation failure.

The alternative — emit a bare `sdiv` — is strictly *more*-undefined than the
evaluator. LLVM `sdiv` by zero is UB, and `INT64_MIN / -1` is UB on top of that.
That is exactly the trade 5b-1 §3.4 refused. So `Div`/`Rem` wait for the slice
that takes on the runtime-error path, and they arrive *with* it rather than
forcing a half-version of it into a control-flow slice.

### §1.2 Why `N3` before `N2`

Recorded so the ordering is auditable later:

- **`N2` alone cannot terminate a recursion.** Calls without branches means no
  base case: `If` does not reach Core, and codegen refuses `Match`. `N2` by
  itself buys straight-line calls and nothing that loops.
- **`N2`'s central proof is unwritable until branches exist.** The evaluator's
  tail-call guarantee is *measured*, not asserted in prose: `tests/tce.rs:19` and
  `tests/tce.rs:30` bound self- and **mutual** tail recursion at 1,000,000 frames
  by peak memory. Native codegen has to match that. No such program can be
  written without a base case.
- **`N3` is self-contained.** Branching lives inside one function. It needs no
  calling convention, no symbol mangling, no linkage decisions, no new external
  symbol.
- **`N3` is the structural change everything else sits on.** `N2`, `N4`, and `N5`
  all emit branches. Doing it first means none of them has to invent basic-block
  emission halfway through its own hard problem.
- **`N3` retires the largest batch of `Unsupported` arms per unit of risk.**
  `If`, `Bool`, six comparisons, two boolean operators — one Core arm and one
  back-end shape change.

### Honesty line

After this slice Elya still cannot call a function it defined. The subset is
wider but the module shape is unchanged: one `main`, no arguments, an `Int`
result. What changes is that the printed value can now *depend on a decision the
program made at runtime* — the first time the native binary computes something a
constant folder could not have computed for it.

---

## §2 The front-end reach

5b-1 §6.2 promised the front end and Core would not be modified. This slice
breaks that promise once, minimally. The size of the break was measured before
this spec was written, not estimated.

### §2.1 The reach is exactly one Core arm

`src/core.rs:177` currently reads:

```rust
Expr::If { .. } => return Err(LowerError::Unsupported("If")),
```

It becomes a lowering that mirrors the existing `Expr::Lambda` arm: recurse into
the condition with `lower_expr`, and into each branch with `lower_block`.

**Nothing else in the front end is touched.** Verified by reading the code, not
inferred from a grep:

| Construct | Status | Evidence |
|---|---|---|
| `Bool` literals | already lower | `Expr::Bool(b) => CoreKind::Lit(CoreLit::Bool(*b))`, `src/core.rs` |
| All six comparisons | already lower | one `Expr::Binary` arm to `CoreKind::Prim(op, [l, r])` |
| `And`, `Or` | already lower | same arm — they are ordinary infix `BinOp`s, `src/parse.rs:1119-1120` |
| `If` branches | do **not** hit `Unsupported("Block")` | branches are `Block` nodes reached through `lower_block`, exactly as `Expr::Lambda` already reaches its body |
| `else` | always present | `else_block: Rc<Spanned<Block>>` — not `Option`, `src/ast.rs:183` |

That last row is worth more than it looks. Because `else` is mandatory in the
AST, **every `If` is a two-branch expression with a value type**. There is no
dangling-else case, no `Unit`-typed one-armed `if`, and therefore no place where
codegen has to invent a value for a missing branch. The diamond in §3.2 is the
only shape that exists.

An `If` whose branch block has no tail expression is already refused upstream:
`lower_block` returns `Unsupported("block without tail expression")`. Codegen
never sees it.

### §2.2 This is not a recorder-totality gap

The tracked recorder-totality class is: a node that lowering reaches but that
inference never recorded, surfacing as `LowerError::Untyped(span)`. `If` is
**not** in that class, and the reason is structural rather than incidental:

```rust
pub fn infer_expr(&mut self, e: &Spanned<Expr>, env: &mut TyEnv, amb: RowVar) -> Ty {
    let ty = self.infer_expr_inner(e, env, amb);
    self.node_types.insert(e.span, ty.clone());     // src/types.rs:882
    ty
}
```

`infer_expr` is the single record point, and `infer_expr_inner` handles `Expr::If`
(`src/types.rs:930-941`: `unify_cond` on the condition, unify the two branch
types, return the unified type). So `table[if_span]` is populated for every `If`
**by construction**.

Contrast the genuine latent candidate in that class: `Qualified` callees are
recorded through a *separate* insertion at `src/types.rs:1152` carrying
`Ty::Error`. Different mechanism, different fix, still `N6`'s problem. This slice
does not touch inference at all.

**Consequence:** the front-end reach costs one match arm and zero inference
changes. Task 1 (§9) *measures* that rather than trusting this paragraph.

---

## §3 The Core→LLVM mapping

### §3.1 Type mapping — the first widening past one value type

5b-1 §3.2 mapped exactly one type. This slice makes it two:

| `Ty` | LLVM | Note |
|---|---|---|
| `Ty::Base(TyCon::Int)` | `i64` | unchanged |
| `Ty::Base(TyCon::Bool)` | `i1` | **new** |
| everything else | `CodegenError::Unsupported` | unchanged |

This is the first crack in "one value type", and naming it as such is half the
point of this section. Concretely, the fold's return type changes from
`IntValue<'ctx>` to a small two-variant value that carries which LLVM type it is,
and the `HashMap<String, _>` environment threaded through `Let` changes with it.
That refactor is most of the mechanical work in the slice, which is why §9 gives
it a task of its own.

It is a **toe-in, not the value-representation decision**. `i1`/`i64` happens to
work only because both are unboxed scalars of statically known width, and because
the type is known at every node from `CoreExpr.ty`. It generalises to nothing.
The real decision — uniform boxed representation versus monomorphization —
arrives at `N4`'s ADT layout table and is flagged in §11.

`main` must still return `Int`: the `printf` shim prints an integer. A `main`
whose body has type `Bool` is `Unsupported("non-Int value")`, exactly as today.
`Bool` is an *internal* value type this slice, never a program result.

### §3.2 `If` lowering

The standard three-block diamond with a `phi` join:

```
  <current>   ; lower cond to i1
              br i1 %c, label %then, label %else
  then:       ; lower then-branch
              br label %merge
  else:       ; lower else-branch
              br label %merge
  merge:      %v = phi <t> [ %tv, %<then-exit> ], [ %ev, %<else-exit> ]
```

Two implementation obligations, both easy to get wrong and both cheap to get
right:

1. **The `phi` incoming blocks must be the blocks each branch *ended* in, not the
   blocks it started in.** A nested `If` inside a branch leaves the builder
   positioned in that nested `merge`, not in the outer `then`. Read the current
   block back from the builder *after* lowering each branch and use that. Corpus
   case 4 in §5 exists solely to make this fail loudly if it is wrong.
2. **Both branches have the same LLVM type by construction.** Inference already
   unified them (`src/types.rs:939`). Codegen asserts this rather than
   re-deriving it; a mismatch is an internal-invariant failure, not a user error.

### §3.3 Comparison lowering, and the `Eq` guard

`Lt`, `Le`, `Gt`, `Ge` are monomorphic in the front end — `binop_type` gives
`(Int, Int) -> Bool` (`src/types.rs:795`). They lower to `icmp slt / sle / sgt /
sge` on `i64`. **Signed**, because Elya `Int` is `i64` and the evaluator compares
Rust `i64`.

`Eq` and `Ne` are **not** monomorphic. Inference handles them specially:

```rust
if matches!(op, BinOp::Eq | BinOp::Ne) {
    self.unify(&lt, &rt, span);      // src/types.rs:920-922
    Ty::bool()
}
```

Both operands are unified with each other and otherwise unconstrained. So `==` is
legal *today* on `Str`, on `Unit`, on ADT values, and on closures — the evaluator
compares `Value`s structurally: `(Eq, a, b) => Ok(Bool(a == b))`,
`src/eval.rs:236`.

Codegen therefore **must dispatch on the operand type**, never accept `Eq`
blanket:

| Operand type | Emit |
|---|---|
| `Int` | `icmp eq` / `icmp ne` on `i64` |
| `Bool` | `icmp eq` / `icmp ne` on `i1` |
| anything else | `Unsupported("Eq on <ty>")` / `Unsupported("Ne on <ty>")` |

The message names the operand type so the boundary is legible from the CLI rather
than from a stack trace. This is the second place the polymorphism boundary shows
through the back end. The first was 5b-1 §3.2's note that a `Ty::Var` reaching
codegen is `N7` knocking; here it is not a variable but a *concrete* type the
back end has no representation for, which is `N4`/`N6` knocking instead. Two
different doors, same wall.

### §3.4 `And` / `Or` lowering

`and i1` and `or i1`. **No branches, no diamond.** See §4.1 — this is a finding,
not an oversight, and it is the one place this spec deviates from the requested
scope.

---

## §4 Semantic-fidelity findings

### §4.1 Elya's `&&` and `||` do not short-circuit

The requested scope for this slice included "`&&`/`||` short-circuit". That
scope item was written on an assumption the code does not support — an assumption
this project's own 5b-1 spec also carries, in its deferral list ("`And`/`Or` with
short-circuit ... need basic blocks (`N3`)"). Both were written about a language
feature Elya does not currently have.

Both evaluators are **strict**. Verified independently, in each:

**Tree evaluator** (`src/eval.rs:335-338`) — both operands are evaluated
unconditionally before the operator is applied:

```rust
Expr::Binary { op, lhs, rhs } => {
    let l = eval_expr(interp, lhs, env, fns)?;
    let r = eval_expr(interp, rhs, env, fns)?;
    apply_binop(*op, l, r, span)
}
```

**CEK machine** — `src/eval.rs:726` pushes a `Frame::BinRight` for *every* `BinOp`
with no special case for `And`/`Or`, and the handler at `src/eval.rs:885-887`
unconditionally schedules the right operand:

```rust
Frame::BinRight { op, rhs, env, span } => {
    State::Eval(rhs, env, push(Frame::BinApply { op, lval: v, span }, rest))
}
```

`apply_binop` then receives two already-computed `Value`s:
`(And, Bool(a), Bool(b)) => Ok(Bool(a && b))`, `src/eval.rs:242`. The `&&` on
that line is Rust's, operating on two `bool`s that both already exist. It is not
Elya's `&&`.

**Decision: `N3` emits strict `and i1` / `or i1`.**

Emitting a short-circuit diamond would make the native binary *less*-undefined
than the evaluator — a divergence in the same family as the one §1.1 refuses,
pointing the other way. It is not hypothetical: it becomes observable the moment
`Div` lands. Under `elya run`, `false && (1 / 0)` aborts with "division by zero";
under a short-circuiting native binary it returns `false`. One source file, two
answers, and the codegen slice would be the thing that caused it.

**Making `&&`/`||` short-circuit is a language-surface change and belongs in a
front-end slice**, where it changes both evaluators, the language design doc, and
the test corpus in one commit. That is precisely the move 5b-1 §4 refused when it
declined to smuggle an `io.print_int` builtin into a codegen slice. Filed in §11
as a carried question, not silently dropped.

Cost of the decision: `And`/`Or` contribute nothing to the basic-block work in
this slice. `If` carries it alone. That is fine — `If` is the construct `N2`,
`N4`, and `N5` actually build on, and a boolean operator that lowers to one
instruction is a smaller diff to review.

### §4.2 The tail-call obligation is deferred to `N2`, not dropped

The evaluator guarantees constant-depth tail recursion, and it is *measured*: 1e6
frames for self-recursion (`tests/tce.rs:19`), 1e6 for **mutual** recursion
(`tests/tce.rs:30`), each with a non-tail negative control, and separately 1e6
through an effect handler (`tests/state_effect.rs:77`). Native codegen must not
stack-overflow where the evaluator loops.

No program expressible in this slice can call anything, so the obligation is
**untestable here** — which is exactly the asymmetry §1.2 uses to argue the
ordering. It is not silently dropped: it becomes `N2`'s primary acceptance
criterion, and `N2` inherits a back end that can already branch, so the test is
writable on `N2`'s first day rather than blocked behind a second structural
change. Recorded in §11 as a carried obligation with a named owner slice.

---

## §5 The deliverable

A corpus of programs that each compile, link, run, exit `0`, and print an exact
integer — where the printed value depends on a branch actually taken at runtime.

| # | Program shape | Proves |
|---|---|---|
| 1 | `if 1 < 2 { 10 } else { 20 }` → `10` | the true edge, and the `phi` |
| 2 | `if 2 < 1 { 10 } else { 20 }` → `20` | the false edge |
| 3 | `let x = 5` then `if x > 3 { x * 2 } else { 0 }` → `10` | condition reads a binding; branch computes |
| 4 | an `if` nested inside a branch of an `if` | the `phi`-incoming-block trap (§3.2) |
| 5 | each of the six predicates, each in both directions | every `icmp` predicate, positive and negative |
| 6 | `if True == False { a } else { b }` | `Eq` on `Bool` — the `i1` path |
| 7 | `And` / `Or` over two comparisons | strict boolean operators |

Every case is asserted by **running the produced binary** and comparing stdout.
No IR is inspected anywhere. Refusal cases (`Div`, `Rem`, `Match`, `Lambda`, `Eq`
on `Str`, a `Bool`-typed `main`) are asserted as named `Unsupported` payloads —
as errors, not as skips.

**Differential check.** Each corpus program is *also* run through `elya run` and
the two outputs compared. This is cheap while the corpus is small, and it is the
mechanism that would have caught §4.1 automatically instead of by reading
`eval.rs`. Establishing it now, at seven programs, is much easier than
retrofitting it at seventy.

---

## §6 Non-interference guarantees

### §6.1 The feature gate still holds

Without `--features codegen`, nothing in the back end is compiled. The `elya` lib
and the front end still have no path to `inkwell` or `llvm_sys` — structural
since Task 5 Step 0 of 5b-1 split the crates, not a matter of default features.
The layering test in `tests/arch/` continues to enforce it.

### §6.2 The Core change is unconditional, and additive

The §2.1 change *is* compiled in every configuration, because `src/core.rs` lives
in the `elya` lib. It is additive in the only sense that matters: an arm that
returned `Err(Unsupported)` now returns `Ok`. No existing successful lowering
changes shape.

The one place this can bite is a test that *asserts* the old refusal. Task 1
finds and updates any test asserting `Unsupported("If")`; that is part of the
task, not a surprise during it.

### §6.3 The evaluator is untouched

Neither `eval.rs` nor `types.rs` is modified. `elya run` behaviour is bit-for-bit
unchanged, which is what makes the §5 differential check meaningful rather than
circular.

---

## §7 Pipeline and module changes

| File | Change |
|---|---|
| `src/core.rs` | one arm: `Expr::If` lowers via `lower_expr` + two `lower_block`s |
| `crates/codegen/src/lib.rs` | value type widens to `Int`/`Bool`; `If`, six comparisons, strict `And`/`Or` |
| `crates/codegen/tests/native_codegen.rs` | the §5 corpus and the refusal tests |
| `README.md` | the supported-subset paragraph |

No new crate. No new dependency. No CLI change — `elya build` gains the wider
subset for free, because the subset lives entirely inside `compile_module`.

---

## §8 Testing strategy

1. **Execution corpus** (§5) — the deliverable. Produces binaries, runs them,
   asserts stdout and exit status.
2. **Differential against `elya run`** (§5) — same source, same output, both paths.
3. **Refusal tests** — `Div`, `Rem`, `Match`, `Lambda`, `Eq` on `Str`, `Bool`
   `main`; each asserts the specific `Unsupported` payload string, so a
   refusal that silently changes shape fails the suite.
4. **Core lowering tests** — `If` now lowers; the typed S-expression renderer
   shows it with its inline type.
5. **No skips, no IR snapshots.** Enforced at review, per §0.

The gate is unchanged: five stages, both configurations, default parallelism (the
`-j 2` cap is obsolete since the crate split — only two test binaries link LLVM
now).

---

## §9 Build order

### Task 1 — Measure the Core reach *(front end only, no codegen)*

Lower `Expr::If`. Run the existing Core test suite. Find and update any test
asserting `Unsupported("If")`. Confirm on a real corpus program that no
`LowerError::Untyped` arises for an `If` span (§2.2 predicts none; this measures
it). Commit standalone — it is a front-end change and should bisect as one.

### Task 2 — Widen the codegen value type

Refactor the fold from `IntValue<'ctx>` to the two-variant value, and the `Let`
environment with it. `Bool` literals and the `i1` type mapping land here. **No
new control flow yet**, and the entire 5b-1 corpus must still pass unchanged —
it is the regression suite for this refactor. Kept as its own task so that the
mechanical diff and the interesting diff are reviewable separately.

### Task 3 — `If`, comparisons, `And`/`Or`

The diamond, the `phi` with correctly-read exit blocks, the six `icmp`
predicates, the typed `Eq`/`Ne` guard, strict `and`/`or`. Named refusals for
everything else.

### Task 4 — The execution corpus and the differential check

§5 cases 1-7, the refusal tests, and the `elya run` comparison harness.

### Task 5 — Docs, gate, commit

README subset paragraph, plan and spec checklists, full five-stage gate in both
configurations, commit with explicit paths, push.

---

## §10 Risks and mitigations

| # | Risk | Mitigation |
|---|---|---|
| 1 | `phi` wired to branch *entry* blocks instead of *exit* blocks (§3.2) | corpus case 4 is a nested `if` chosen specifically to expose it; it fails loudly, not subtly |
| 2 | The Task 2 value-type refactor balloons past its task | it is isolated, and the untouched 5b-1 corpus is its regression suite |
| 3 | `Eq` accepted blanket, silently mis-compiling `Str` equality | typed dispatch (§3.3) plus an explicit refusal test naming the operand type |
| 4 | Short-circuit emitted out of habit from other languages | §4.1 is in the spec rather than a code comment precisely so review catches it |
| 5 | The `i1`/`i64` special case hardens into an implicit value-rep decision | §3.1 and §11 name it a toe-in; `N4` must decide explicitly |
| 6 | `select` used as a shortcut for a branch | §0's decoy paragraph; corpus case 4 does not distinguish them, so this is a review obligation, not a test one |

---

## §11 Deferred and honestly flagged

**Deferred to named slices:**

- `Div`/`Rem` with a faithful runtime error — to the slice that takes on the
  runtime-error path (message, `abort`/`exit`, string constant, span question).
  §1.1.
- Calls and multi-function modules — `N2`.
- `Match`, ADTs, heap, GC — `N4`. Core lowers `Match` today; codegen refuses it.
- `Lambda`/closures `N5`; `Str` and a real `io.println` `N6`; runtime
  polymorphism `N7`; effects, handlers, and the `Handle`/`Resume` crux `N8`.
- Remaining Core `Unsupported` arms untouched here: `Float`, `Qualified`,
  `Unary`, `Block`, `Handle`, `Resume`.

**Carried obligations — tracked, not closed:**

- **Tail calls.** Native must not stack-overflow where the evaluator loops
  (`tests/tce.rs`, 1e6 depth, self *and mutual*). Untestable in this slice; it is
  `N2`'s primary acceptance criterion. §4.2.
- **Short-circuit `&&`/`||`** is a *language* question, not a codegen one. If
  Elya wants it, a front-end slice changes both evaluators, the design doc, and
  the corpus together. Until then, native matching strict evaluation is the
  correct behaviour, not a limitation. §4.1.
- **Integer overflow.** Still unreconciled between the evaluator and codegen
  (`add`/`sub`/`mul` emitted without `nsw`/`nuw`). Carried unchanged from 5b-1
  §11; this slice neither worsens nor addresses it.

**Forward flag — do not act on it in this slice.** `N4` picks the ADT layout, and
that choice silently pre-commits `N7`'s monomorphize-versus-uniform-boxing
decision. The two are one decision made at one table, and it will get made
whether or not it is made deliberately. When `N4` opens, that decision must be
stated and argued in its spec — not defaulted into by whichever layout is
convenient for the first ADT that needs one.

---

## §12 Milestone checklist

- [x] `Expr::If` lowers to Core; no test asserts `Unsupported("If")` any more
      (reached through five Core sites, not the one §2.1 assumed — see the plan's
      "Deliberate deviations" #1)
- [x] No `Untyped` arises for an `If` span (recorder totality holds — §2.2)
- [x] Codegen value type carries `Int` and `Bool`; `Bool` maps to `i1`
- [x] The full 5b-1 corpus still passes after the Task 2 refactor, unchanged
- [x] `If` emits a three-block diamond with a correctly wired `phi` (not `select`)
- [x] A nested `if` inside a branch compiles and runs correctly (the §3.2 trap)
- [x] All six comparison predicates emit, each proven by execution in both directions
- [x] `Eq`/`Ne` dispatch on operand type; `Str` operands refused by name
- [x] `And`/`Or` are strict, matching both evaluators (§4.1)
- [x] `Div`/`Rem` still refused by name, with the §1.1 rationale recorded in the spec
- [x] Every corpus program produces identical output under `elya run` and native
      (measured via `run_module_value`, because `elya run` discards main's value —
      see the plan's "Deliberate deviations" #2)
- [x] No IR snapshots anywhere; no test skips
- [x] Full five-stage gate green, both configurations, default parallelism
