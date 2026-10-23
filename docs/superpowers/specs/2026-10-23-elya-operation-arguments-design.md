# Operation values passed to functions, natively

**Status:** done (2026-10-23, branch `auto/elya`). HANDOFF step 1 (operation values
as values natively), first part. The design choices are Claude's under the maintainer's
delegation (rule 2), recorded here. Builds on
`2026-10-17-elya-native-any-handlers-design.md` (D1: expansion per operation; D2: no
operation value at run time).

## 0. Measured first (at the one-shot-words slice)

A `with any` clause's binder `o` natively may only be the first argument of an `op.*`
builtin, an operand of `o == o`, captured by a lambda that does the same, or (since
2026-10-22) aliased. Every other use is refused, "operation value used other than by
op.* or ==".

What a recorder library writes, measured on both sides with one effect
`W { roll(Int) -> Int, put(String) -> Unit, none() -> Bool }`:

| program | the clause | evaluator | native |
|---|---|---|---|
| p1 | `io.println(describe(o))`, `describe(o) = op.name(o) <> " = " <> op.show(o)` | the three lines, 80 | refused |
| q1 | a helper calling a helper (`outer(o, 7)` calls `inner(o)`) | `W.put#7`.., 81 | refused |
| q2 | a recursive helper (`rep(o, n)`) | `W.putW.put`.., 81 | refused |
| q3 | `resume(logged(o))`, the helper performs, shows and encodes | the four lines, 81 | refused |
| q4 | `same(o) = o == o` | `same` x4, 81 | refused |
| q6 | a helper generic in another parameter (`tagged(o, True)`) | 81 | refused |
| q7 | nested `with any`: `both(o, p)` with both binders | `W.put/W.put`.., 81 | refused |
| q8 | a helper that decodes (`op.decode(o, op.encode(o, v))`) | 81 | refused |
| p2 | `op.perform(id(o))`: a helper that RETURNS the operation | 80 | refused |
| p5 | `E(o, "x")`: the operation stored in data | 80 | refused |
| q5 | a local lambda taking the operation, `d(o)` | the lines, 81 | refused |
| p3 | a lambda capturing `o` and calling `op.show(o)` | the lines, 80 | **runs, agrees** |

Every helper above takes the operation as a parameter and uses it only as a clause
would: `op.*`, `==`, or passing it on to another such helper.

## 1. Decisions

- **D1. A top-level function called with the operation is cloned per operation.** In
  a clause copy the operation is known statically (`OpAt`: its name, its arguments
  bound to the clause's parameters, its answer type). A call `f(.., o, ..)` of a
  top-level function becomes a call of `f.op.<k>(.., a1, .., an, ..)`. The clone's
  parameter for `o` is replaced by the operation's arguments `a1..an`, and its body is
  rewritten at the same operation, exactly as a clause body is: `op.*`, `==`, and
  further calls that pass the parameter on.
  - This is the same monomorphization D1 of 2026-10-17 applies to the clause: a
    function over `Op(t)` runs at a different operation each time, so natively it is
    one copy per operation. That was the D2 option the spec deferred.
  - Clones are memoized by (function, the position, the operation, its answer and
    label), so a recursive helper (q2) is one clone that calls itself.
  - They are built in ONE worklist with the clones a rigid answer type needs (R1), in
    request order, so a clone's source always exists.
  - An operation clone inherits its source's rigid answer types: a helper cloned at
    an inner clause's operation can still be at the outer clause's answer type, and
    the outer expansion clones it again there (the review's F1, §3).
  - A function that still takes an operation after the pass is unreachable (every call
    of it was redirected) and is dropped, as R1 drops the rigid originals.
- **D2. What stays refused, by the same name.**
  - A helper that returns the operation, or stores it in data (p2, p5). The operation
    would have to exist at run time.
  - A local lambda called with it (q5). It is a closure, not a name to clone.
  - The operation passed twice to one call (`f(o, o)`). The two parameters would have
    to be renamed into one, under the body's binders.

  These are measured as rarer than a helper (no corpus program does them).
- *Options considered:*
  - (a) A run-time operation value: a descriptor and boxed arguments, and dynamic
    `perform`. Rejected again for the 2026-10-17 D1 reasons (non-uniform native
    values, a second resume convention).
  - (b) Inline the helper into the clause. That fails on recursion (q2) and grows code
    with every call site.
  - (c) **Taken:** clones per operation.

## 2. Evidence

`crates/codegen/tests/native_any.rs`, each red first as the plan predicted (refused by
name):
- `helpers_over_an_operation_agree`: p1, q1, q2, q3, q4, q6 and q8, each with the
  evaluator's text and value;
- `a_nested_clause_passes_both_operations` (q7).

Also:
- `an_operation_returned_stored_or_passed_twice_is_refused` (p2, p5, q5, q9) is D2's
  guard. It was green before the change and stays green.
- `any::tests::a_recursive_helper_is_cloned_once_per_operation` asserts three `rep`
  clones, each calling itself, and no function left that takes an operation. Red
  first: refused.

**Changed test, by D1:** `what_is_not_compiled_is_refused_by_name`'s first program
gave the operation to a function (`describe(o)`), which now compiles and agrees
(p1). It now has a function return the operation (`op.show(id(o))`), which is still
refused.

Controls (each reverted, `cmp` after):
- **K1**, the clone's body not rewritten: "unsupported builtin (op.name)" (the clone
  still applies `op.*` to a parameter that no longer exists).
- **K2**, the operation's arguments not passed (the parameter kept): the clone is
  dropped as one that still takes an operation, so its call is "callee is not a
  top-level function".
- **K3**, no memo: q2's recursive helper requests itself forever, "too many function
  clones".

## 3. Review (2026-10-23)

An independent review ran:
- about 97 differential programs: recursion and mutual recursion; the operation at
  every position, with more than 3 parameters after the clone; several clauses,
  handlers, effects and row specializations; effects with parameters and generic
  answers; CPS performs; nested `with any`; `with multi` around a handler and inside
  a helper; every failure's words; shadowing; the clone budget;
- the full sweep: 801 programs, no `differ`, no crash.

Findings:
- **F1 (high, caused by this slice, fixed).** A nested clause passes its operation to
  a helper that also uses a value of the OUTER operation's answer type
  (`both(o, p)` with `op.encode(o, v)`).
  - The inner expansion's clone kept the outer rigid type, and nothing substituted
    it. At run time this was "internal: op: a value of a type with no instance";
    elsewhere it panicked the compiler in inkwell.
  - Before the slice these programs were refused by name.
  - Fix: an operation clone inherits its source's rigid types (and any in the
    operation's arguments). Both kinds of clone are served from one list in request
    order, so the outer expansion clones it again at the outer answer.
  - Ten programs (m1-m3, m6, d01-d05, d07) are now
    `nested_clauses_pass_operations_to_helpers_over_the_outer_answer`. That test was
    red first: m1's run-time failure.
  - **Control K4**, clones not registered as rigid: that run-time failure again.
- **F2 (low, the same cause, fixed with F1).** The inner clone was dropped while
  still called: "callee is not a top-level function" instead of the D2 refusal.
  Those programs now run (m3, d02, d05, d07 above).
- **F3 (low to medium, open).** A helper that performs the operation inside its own
  handler (`fn guarded(o) { handle { op.perform(o) } with { A.ask() -> resume(3) } }`)
  is refused, "calling convention disagrees with the callee". The evaluator gives 40.
  - The helper's type puts `A` in the `with any` row, so the expansion clones it for
    `A.ask` too.
  - That clone handles its own perform, so it is direct code. Its call keeps the
    source call site's CPS type.
  - The same code written in the clause runs. Recorded in HANDOFF.

On the fixed build, the review's 97 programs give 73 agreements (including every
failure in the same words), 19 refusals by name and 5 front-end rejections, with no
mismatch.
