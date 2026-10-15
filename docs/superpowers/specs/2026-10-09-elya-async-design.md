# Async without function colouring — and what it took

**Status:** done (2026-10-10), after an independent review (review6) and its fixes.
ROADMAP priority 1, HANDOFF step 2 (taken with step 1, N7 part 1, because the native
half needed it). Design choices are Claude's (rule 2).

## 0. The goal, measured

`examples/04_async.elya`: an `Async` effect (`yield_`, `fork`), a round-robin scheduler
written as an ordinary Elya handler that keeps the continuations of suspended tasks in a
queue, and ONE `each` used by synchronous code and by tasks that yield. Measured on the
annotations tip, the program was refused at every layer:

| Layer | Refusal |
|---|---|
| check | a function type in a type or effect declaration ("not supported yet"): the queue holds functions, `fork` takes one |
| check | after that, `fn(q2) { (resume(Unit))(q2) }` closed its row before `resume`'s result type was known: a closed-row E0423 |
| build | "computed callee" (`(run(child))(q2)`) |
| build | "effect-polymorphic function used at a user effect" (`each` at `Async`) |
| build | "unrepresentable type" (`each`'s callback result is a type variable) |

Now it checks, runs in the evaluator (snapshot `tests/snapshots/examples__04_async.snap`),
and runs natively with the evaluator's output, line for line
(`the_async_example_runs_natively_like_the_evaluator`).

## 1. Front end

- **Function types in declarations** have exactly the row written; unwritten means pure.
  A declaration has no row parameters, so a field cannot be effect-polymorphic; writing
  the effects (`Next(fn(Queue) / {IO} -> Unit, Queue)`) is how a queue of effectful tasks
  is declared, and a pure value still fits it by sub-effecting. A generic effect in such a
  row (`{State}`) is a named error for now.
- **Deferred lambda closing.** A lambda whose ambient still receives an inclusion from an
  open source (a call of a value whose type is not known yet -- `resume(..)`'s result in a
  clause) is not closed at its end. Its ambient stays open, is never generalized
  meanwhile, and closes at the group's end, keeping a tail shared with a member's
  parameter open (a relay).
- **The deferred lambda's VALUE row includes its ambient; it is not the ambient**
  (review6). The first version used the ambient itself as the value's row. Then every
  later use joined it by equality: a context's effects flowed into the lambda's row and
  on to its other uses, and a let-bound one lost its generalization. Six programs base
  accepted became E0420. Now the value row is `{ambient's labels so far | φ}`, where φ is
  a fresh phantom (upcast) tail, and the pair is recorded in `deferred_incl`.
  - The flush carries the ambient's later labels into it, labels only. What remains of φ
    stays phantom (`add_label_keeping_phantom`), so callers include the row rather than
    unify with it.
  - φ may be generalized. `instantiate` copies the inclusion to every instance, so a
    label the ambient gains later still reaches each use, including a closed field one
    use was unified with (a conflict, never a performing closure in a pure field).
    This is a constrained type scheme, `∀φ. (amb ⊆ φ) ⇒ τ`, kept as a side table.
  - If the ambient is still open at the group's end (it relays a member's parameter), each
    value row's tail grows into the relayed one.

## 2. Native

- **Every computed callee is A-normalized** in Core lowering (`let $k = e in $k(args)`),
  not only `resume(..)`.
- **Convention specialization (N7 part 1).** `crates/codegen/src/specialize.rs`: a use that
  instantiates a function's signature row variables at a user effect gets a clone in which
  they are a marker label, `$poly`, that every later pass treats as one more user effect.
- **Monomorphization (N7 part 2, first half).** The same pass: a use at ground types gets a
  clone with the signature type variables replaced, and a generic original nothing reachable
  from `main` uses is not emitted -- so `Ty::Var` never reaches the back end for a function
  only used at concrete types. Clones are keyed by their type arguments without label
  spans. A backstop of 4096 clones turns a non-converging worklist into a named refusal
  ("specialization did not converge") instead of a build that never finishes.
- **Parametric ADTs are monomorphized** (`TypeMono`). Each ground instantiation becomes
  its own type (`List$t0`, `Cons$t0`), because a GC row records which fields are heap
  values and `List(Int)` and `List(String)` disagree on that.
  - **An instantiation is its arguments up to effect rows** (`layout_key`, review6). The
    first version keyed instances on the `Debug` text of their arguments. That text
    carries an open row variable's id, a clone's `$poly` against the caller's real
    effect, and a label's span, so one value got several types with different
    constructor tags. A wildcard arm ran (0 where the evaluator printed 42), or no arm
    matched.
  - The key keeps each function row's calling convention and nothing else. The
    convention is the same at every use of one value, because `$poly` stands for a user
    effect, and a pattern binder's field type decides how a closure taken out of it is
    called.
  - Plain types' fields are rewritten too (`type Q { Q(List(Int)) }`).
- **A non-regular parametric type is refused by name** ("non-regular parametric type",
  review6). For example, `Nest(a)` holds a `Nest(Box(a))`; its instances never run out, and
  registering them made `elya build` run forever. Once the module's own uses are registered,
  a field-derived instance may be at most `uses' depth + (parametric types + 1) × (field
  depth + 1)` deep. A regular type's fields only rearrange its arguments, so its instances
  stay within that bound; a non-regular one crosses it. There is also a backstop of 4096
  instances.
- Pattern binders carry their field types into the convention check, so a direct closure
  taken out of an ADT field and upcast is refused by name, not miscompiled.

## 3. Review6 (independent) and what changed

| Finding | Repro | Fix | Pinned by |
|---|---|---|---|
| A: one instantiation, several native types | `call_or(Some(fn(x) { x + 41 }), 0)`: native 0, evaluator 42; e02 (no generics at all): "match failed"; review4's n2 now miscompiled | `layout_key` | `one_adt_instantiation_is_one_native_type` (8 rows) |
| A: a non-regular type hangs `elya build` | `type Nest(a) { Leaf, N(a, Nest(Box(a))) }` | depth bound, named refusal | `a_non_regular_parametric_type_is_refused_by_name` (runs the build on a thread, 30 s limit) |
| C: deferral over-rejects | q04: `let q = fn(y) { p(y) }  with_t(q) + q(1)` was E0420 (base accepted it; the evaluator prints 5); also q01, q03, q07, h01, h03c | the value row includes the ambient | six tests in `tests/async_effect.rs` |

The sweep of 595 programs from earlier reviews was re-run against this tree:
- **Check:** every status is the same as in the reviewed tree. No program goes from
  accepted in base to rejected.
- **Native:** nothing that compiled in base now crashes or prints something different;
  n2 now matches the evaluator.
- **One row returns to base's named refusal: review2's a1.** The program is
  `f = handle { k() + get() } with { S.get() -> fn(s) { (resume(s))(s) } .. }`.
  - **Before the fix:** in the reviewed tree it compiled only through the leak. `f(1)` tied
    the clause lambda's ambient to `h`'s ambient, which happens to relay `k`'s effects.
  - **Now:** it is refused by name again, "resume of an effectful handler in direct code",
    which is what base did.
  - **Underlying cause:** `resume` does not carry a row the handled body only relays (PARKED).

## 4. Known limits (PARKED)

- **`resume` does not carry relayed rows.** Under it, review6's s03 is unsound: an
  unannotated `run(task)` whose task performs an outer `T` checks clean, and the evaluator
  then reports "unhandled effect". Review2's a1 is refused natively by name.
  - The direct fix ties the resume site to the relayed tail by equality, as a call of a
    parameter does. In an unannotated scheduler, that forces the tasks to be `Async`-only:
    the queue's closed `{IO}` closes the relayed row, and `IO` is already counted in the
    lambda's own labels. That would break `a_yield_only_scheduler_checks_and_interleaves`.
  - The fix needs an inclusion with an upper bound, or the clause rows known before the
    clauses are typed. `04_async` writes `task`'s row, so it is not affected.
- **Shadowing a generic top-level name keeps the original alive** (review6's d04). `refs`
  counts names, not bindings, so the build refuses with "unrepresentable type", as base did.
- **A misleading refusal name for an unused non-ground function** (review6's f05). For
  example, `fn unused() -> Int { len(Nil) }` gives "callee is not a top-level function"
  instead of naming the type.

## 5. Negative controls (each reverted, verified with `cmp`)

| Control | Mutation | Fails |
|---|---|---|
| F1 | the deferred value row is the ambient again | the six over-rejection tests and the growing-row guard |
| F2 | `instantiate` does not copy the inclusion | only the pure-field soundness guard (accepted) |
| F3 | a grown value row's tail is made meaningful | only the growing-row guard (E0423) |
| K1 | instances keyed by raw `Debug` again | seven rows of the instance corpus (the plain-type guard holds) |
| K2 | the key drops the convention too | only the two-conventions row, by D16's refusal |
| B1 | no depth bound | the non-regular test, by its 30 s timeout |

The earlier controls (D1, D2 in `tests/async_effect.rs`; S1–S3, M1, M2 above the
`SPECIALIZED` corpus) are unchanged.
