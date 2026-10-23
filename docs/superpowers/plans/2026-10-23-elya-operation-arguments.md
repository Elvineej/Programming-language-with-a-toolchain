# Operation values passed to functions, natively -- plan

Spec: `docs/superpowers/specs/2026-10-23-elya-operation-arguments-design.md`. Branch
`auto/elya`, on the one-shot-words slice. Baseline Linux gate: 1100 passed, 106
suites (1106 by the time it was gated, see the end).

1. Red tests first. Native and codegen unit tests count once.
   - `crates/codegen/tests/native_any.rs`:
     - `helpers_over_an_operation_agree`: p1, q1, q2, q3, q4, q6 and q8 of spec §0,
       each against the evaluator (text and value). Predicted red: refused by name.
       +1.
     - `a_nested_clause_passes_both_operations`: q7. Predicted red the same way. +1.
     - `an_operation_returned_stored_or_passed_twice_is_refused`: p2, p5, q5 and
       `f(o, o)`. D2's guard: predicted GREEN before the change, and it must stay
       green after. +1.
   - `crates/codegen/src/any.rs`: `a_recursive_helper_is_cloned_once_per_operation`.
     q2's `rep` gives one clone per handled operation, each calling itself, and no
     function's signature still takes an operation. Predicted red: refused. +1.
2. `any.rs`:
   - `rewrite` gains a sink for clone requests;
   - a call of a top-level function with `o` in exactly one argument position is
     redirected to `f.op.<k>`, with the operation's arguments in that position;
   - the worklist builds the clones; functions whose signature still mentions `$Op`
     are dropped.
3. Controls, each reverted with `cmp` against the kept files:
   - K1, the clone's body not rewritten: the clone keeps `Var(o)`, so it is refused
     again;
   - K2, the arguments not passed (the clone's parameters kept as the operation): a
     build failure by name;
   - K3, no memo: q2 never finishes cloning and stops at the clone budget ("too many
     function clones").
4. The sweep: no new disagreement.

Predicted gate: 1100 + 4 = **1104 passed, 106 suites**.

The baseline moved before this slice was gated: the slice before it (dfb8a5c) added
six tests, so the prediction is 1106 + 4 = 1110.

Measured: 1110, then **1111 passed, 106 suites** after the review's fix (its nested
test, +1).
