# Failure as an effect, and scope cancellation

**Status:** done (2026-10-16). HANDOFF step 1. Design choices are Claude's under
the maintainer's delegation (rule 2), recorded here.

## 0. Measured first

At `fcd9cd4`:

| Program | check |
|---|---|
| `effect Fail { fn fail(reason: String) -> a }` (an op-local type variable) | E0432 unknown type `a` |
| `type Never { }` and `effect Fail { fn fail(reason: String) -> Never }` | ok |
| `fn raise(r) { match fail(r) { } }` (an empty match on `Never`) | E0430 "`_` is not covered" |

An empty type is already declarable; what is missing is eliminating it. The
exhaustiveness checker is type-free (constructor sets come from declarations, found
through the PATTERNS), so with no arms it sees no constructor and reports `_`.

## 1. Options for "an operation that never returns"

- (a) **Operation-local type variables** (`fn fail(r: String) -> a`, quantified per
  perform). A clause would see `a` as a rigid variable, so it could not resume -- the
  same guarantee as (b) -- but it needs per-operation quantification in the op table, in
  instantiation, in clause typing, and in every back-end path that reads an op's type.
- (b) **An empty type and an empty match (chosen).** `type Never { }` has no values, so a
  clause for `fail` has nothing to resume with: abandoning the computation is forced by
  the type, with no new rule. `match e { }` on a value of an empty type is exhaustive and
  has ANY type (its result is a fresh variable: no arm constrains it), which gives
  `fn raise(r) / {Fail} -> a { match fail(r) { } }` as an ordinary library function.
  This is Haskell's `Void`/`absurd` and Rust's empty enum match: no special bottom type
  in the type system, one exhaustiveness fact.
- (c) A built-in bottom type `!` that unifies with everything: a new subtyping-like rule
  in a checker that has none. Rejected.

(b) is the stricter, more principled option: the empty match is checked against the
scrutinee's TYPE -- `match e { }` is accepted only when `e`'s type is a declared type with
no constructors (E0430 otherwise, naming the type) -- and nothing else in the language
changes. The type checker owns this check (it has the type); `exhaust.rs` leaves zero-arm
matches to it.

## 2. Cancellation

The scheduler of `examples/06_structured.elya` gains `Failed(reason)`: a task's `Fail` is
handled inside `step` (no resume, by the type), and `run` answers `Cancelled` at the first
failed task, DROPPING the tasks still queued -- they never run. `scope` then fails with
the same reason, so a failure climbs nested scopes until a `catch`. Example:
`examples/10_failure.elya`.

## 3. Natively

A zero-arm match is a trap (`elya_match_fail`), unreachable since no value of an empty
type exists.

## 4. Plan and evidence

Plan: `docs/superpowers/plans/2026-10-16-elya-failure-and-cancellation.md`.

- `tests/failure.rs`: `raise` at `Int` and at `Bool` (-3); E0430 naming "no constructors"
  for `Some(1)`, `1` and `"s"` scrutinees (red first: the first failed with exhaust's
  "`_` is not covered", the second passed on the old message and was tightened to name
  the new one).
- `examples/10_failure.elya`: snapshot, `crosscheck`'s effectful list, and
  `the_failure_example_runs_natively` (same bytes, -123).
- Conservative by design: a scrutinee whose type is still a variable at the match
  (`fn absurd(x) { match x { } }`, unannotated) is E0430; annotate it (`x: Never`).

## 5. The independent review

About 25 probes (evaluator and natively). Fixed, red first
(`a_name_declared_twice_is_e0209`):

- **F1 (medium).** `type Never { A }` beside `type Never { }`: the name counted as empty,
  so `match e { }` accepted `A` and the evaluator found no arm. The root is older: a
  name declared twice was a silent overwrite for every kind (functions, types,
  constructors, effects all checked clean at `fcd9cd4`). Now **E0209**, "`X` is declared
  more than once", per kind -- a new diagnostic, Claude's call: the stricter option, and
  the one that removes the whole class.
- **F2 (low, conservative).** Whether an empty match on an unannotated parameter is
  accepted depends on statement order (the scrutinee's type is resolved at the match).
  It only ever rejects; annotate the parameter. Left as documented.
- **F3 (info).** A `with any` clause can `resume(op.decode(o, "5"))` on `fail`; the
  evaluator stops by name ("not a value of type `Never`"), as any failed decode does.

Final gate: 1010 passed, 95 suites (1008 predicted before the review, +2).
