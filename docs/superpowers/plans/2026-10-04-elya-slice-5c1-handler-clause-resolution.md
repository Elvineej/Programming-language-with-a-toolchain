# Slice 5c-1 — handler clause resolution: plan

Spec: `docs/superpowers/specs/2026-10-04-elya-slice-5c1-handler-clause-resolution-design.md`.
Branch: `claude/slice-5c1-clause-resolution`, stacked on PR #1's branch (main does not have
5b-8 yet). Every task: predictions written first, red tests first, full gate
(`sh scripts/check.sh`), commit with explicit paths. Windows CI runs on push.

Test file: a new `tests/clause_resolution.rs` (front-end codes via `parse_module` +
`resolve::check`/`check_source`, values via `eval::run_module_value`). The native row goes
in `crates/codegen/tests/native_codegen.rs`'s Task 8 handler corpus.

Baseline: Linux gate 594 passed, 59 suites (at `23c719f`).

## Task 1 — E0202, unique op names (B1)

Red tests, predictions measured at baseline:
- `an_op_declared_by_two_effects_is_e0202` (m2): today `[E0420]`, wants exactly `[E0202]`,
  message naming `ping`, label at the second declaration, help naming `A`.
- `an_op_declared_twice_in_one_effect_is_e0202` (m7): today `[E0400, E0400]`, wants `[E0202]`.

Implement in `resolve::check`'s declaration pass: a `HashMap<op, (effect, span)>`; a second
insert pushes E0202. Predict: 2 tests green, suite count +1 (new file), passed +2.

## Task 2 — E0203, clause resolution (B2)

Red tests:
- `a_clause_naming_the_wrong_effect_is_e0203` (m3): today `[E0420]`; wants `[E0203]`, text
  "`ask` is an operation of `Ask`, not `Other`".
- `a_clause_naming_an_undeclared_effect_is_e0203` (m4): today `[E0420]`; wants `[E0203]`, text
  "`Nope` is not a declared effect".
- `a_clause_for_an_undeclared_op_is_e0203` (m6): today accepted; wants `[E0203]`, text "no effect
  declares an operation `nope`".

Implement in `resolve`'s `Handle` arm (needs effect names and op -> effect in `Cx`).
Predict: 3 green, passed +3.

## Task 3 — the evaluator runs unqualified clauses (B3)

Red tests:
- `an_unqualified_clause_runs_in_the_evaluator` (m1): today E0300 at run; wants `Int(2)`.
- `a_handler_mixing_qualified_and_unqualified_clauses_runs`: `effect Two { fn a() -> Int  fn
  b() -> Int }`, body `a() + b()`, clauses `Two.a() -> resume(10)` and `b() -> resume(4)`:
  wants `Int(14)` (today E0300 on `b`).
- `an_unqualified_handler_over_two_effects_is_still_e0423`: guard, green before and after.

Implement: `handler_handles` and `run_clause` match `c.op == op && c.effect.map_or(true,
|e| e == effect)`. Predict: 2 red -> green, guard stays green, passed +3.

## Task 4 — Core lowering and native (B4)

Red tests:
- `core_lowering.rs`: m1 lowers, its clause's effect is `Ask` (today
  `Unsupported("unqualified handler clause")`). Replaces nothing: the existing refusal test,
  if any, is the one place an expectation changes — reported, and changed only because the
  spec lifts that refusal.
- native Task 8 corpus row `unqualified-clause` (m1), expected `2`, through both corpus
  tests (value and differential).

Implement: the refusal becomes `c.effect.clone().or_else(|| cx.op_effects.get(&c.op).cloned())`,
with the refusal kept (renamed "clause names no declared operation") as a defensive
unreachable after E0203. Predict: native needs no codegen change.

## Task 5 — negative controls (B6), reverted, each failing differently

- C1 E0202 removed: B1 tests fail with `[E0420]` (m2) and `[E0400, E0400]` (m7).
- C2 E0203 case (c) removed: m3 test fails with `[E0420]`; m4/m6 tests unaffected.
- C3 evaluator match restored to strict `Some(effect)`: B3 value tests fail with E0300;
  native corpus row fails in the differential only (evaluator side errors).
- C4 lowering fallback removed: core test and native row fail with the refusal.

## Task 6 — close-out

PARKED: the unqualified-clause entry resolved; m8 (duplicate clause) added; the op-index
note updated (three indexes, now consistent by rule 1). Spec §4 status table. Independent
review. Windows CI green (B7). Prediction for the final Linux gate: 594 + 2 + 3 + 3 + 2 =
604 passed (the core and native additions counted as one test each; the corpus row adds no
test).
