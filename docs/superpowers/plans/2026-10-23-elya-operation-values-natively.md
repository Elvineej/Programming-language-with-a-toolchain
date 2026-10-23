# Plan: operation values as values, natively

Spec: `docs/superpowers/specs/2026-10-23-elya-operation-values-natively-design.md`.
Baseline: 1106 passed, 106 suites (95e1e08).

## Task 1: red tests

`crates/codegen/tests/native_op_values.rs`, the `native_any.rs` harness (compile, link,
run, compare with the evaluator), six tests:

1. `helpers_take_the_operation`: p1 and p2. Predicted red: "operation value used other
   than by op.* or ==" (a panic in `run_native`).
2. `operations_are_stored_captured_and_returned`: p3, p5, and `o` in a declared
   `Box(a)` matched out. Red: the same refusal.
3. `two_operations_with_one_answer_type_get_their_own_helpers`: `describe(o)` over
   `W.roll(s) -> Int` and `W.now() -> Int` (D2). Red: the same refusal.
4. `operations_compare_through_helpers`: p4 and p7 (the evaluator's failure words).
   Red: the same refusal.
5. `a_replayer_built_from_helpers`: a state-passing replayer whose clause calls
   `question(o)` and `answer(o, s)` (`op.decode` in a helper) over two effects.
   Red: the same refusal.
6. `nested_handlers_pass_both_operations`: an outer and an inner `with any`, the inner
   clause calling `pair(outer_o_name, o)`-style helpers on its own operation and
   `op.name` of the outer. Red: the same refusal.

Changed in place (spec D3): `any.rs`'s unit test (both programs expand), and the
`describe(o)` half of `what_is_not_compiled_is_refused_by_name` removed (it is test 1's
first row).

## Task 2: the pass

`crates/codegen/src/any.rs`:
- an operation-type table: per `(E.op, label instance)` a synthesized `CoreType`
  `$Op.E.op.N` (one constructor, the parameters at the instance), added to
  `CoreModule::types`;
- the clause copy maps `$Op(rigid, _)` to that type before `rigid` to the answer, and
  opens with `let o = $Op.E.op.N(args)`;
- clone keys `(function, rigid, operation type)`; a clone's types are mapped the same
  way;
- a final type-directed rewrite over every function: `op.*` on an operand of an
  operation type is a `match` binding its fields, then the existing expansion; `==`/`!=`
  likewise, both operands matched;
- `OP_VALUE_REFUSAL` removed.

## Predictions

- Gate: **1112 passed, 107 suites** (six native tests, counted once; one new suite).
- The sweep (`scripts/sweep.py`): no new `differ`; no program changes class except ones
  refused for the operation value (none known in the corpus: the 2026-10-17 spec found
  no corpus program using `o` that way).
- Controls K1 (clone key by answer type: test 3 prints `W.roll` twice), K2 (fields
  bound in reverse: test 1's `op.show` of a two-argument operation differs), K3 (no
  rewrite in clones: tests 1-6 fail at build, "op builtin" on a non-operation) each fail
  a distinct test.

## Outcome

Gate 1115 / 107: the six planned tests (1112 as predicted), then three from the
review (spec §3).
