# One wording for the one-shot trap, and the sweep's smaller gaps -- plan

Spec: `docs/superpowers/specs/2026-10-22-elya-one-shot-words-and-sweep-gaps-design.md`.
Branch `auto/elya` at `9327f74` (the lock, on `ebc2635`). Baseline Linux gate: 1090
passed, 106 suites (measured at e78ace6; ebc2635 and 9327f74 touch only documents).

1. Red tests first. Root tests count twice; native and codegen unit tests count once.
   - `crates/codegen/tests/native_codegen.rs`:
     - `the_one_shot_trap_says_the_same_words_on_both_sides` (D1): the four sweep
       programs. Each native line after `elya: ` must be the evaluator diagnostic's
       message. Predicted red on the words. +1.
     - the `CTOR_VALUES` corpus (D2), each row's value measured in the evaluator
       first. Two tests, run natively and against the evaluator. Predicted red: the
       monomorphic rows crash, the parametric rows are refused by name. +2.
     - the `ALIASES` corpus (D3), two tests the same way. Predicted red, refused by
       name. +2.
     - three changed expectations: the native trap's words, in the existing one-shot
       tests (2665, 4269, 4300). A decision (D1).
   - `crates/codegen/src/anf.rs` unit tests:
     - `a_constructor_used_as_a_value_is_wrapped` (D2). +1.
     - `an_alias_whose_target_is_rebound_is_kept` (D3, capture). +1.
   - `tests/effect_syntax.rs`: `a_user_effect_named_io_is_refused` (D4, E0209 and its
     words). +1 root, counted twice: +2.
   - `tests/effects_run.rs`: the one-shot test also asserts the words and the label.
     That is a changed expectation (D1), +0.
2. Evaluator: the E0425 message and label (D1).
3. Runtime: `elya_resume_twice`'s words (D1).
4. `anf`: rule 3 (constructor values) and rule 4 (alias substitution, with the
   capture check).
5. Resolve: E0209 for `effect IO` (D4).
6. Controls, each reverted with `cmp` against the kept files:
   - K1, rule 3 off: the monomorphic constructor rows crash again;
   - K2, rule 4 without the capture check: the capture unit test fails;
   - K3, rule 4 off: the alias rows are refused by name;
   - K4, the label unchanged: the effects_run assertion fails.
7. The sweep (`scripts/sweep.py`): no `both-fail-differently`. The refusals drop by
   two (0155 and 0395) and the build errors by one (0495, now `not-checked`).

Predicted gate: 1090 + 1 + 2 + 2 + 1 + 1 + 2 = **1099 passed, 106 suites**.

Measured: **1100 passed, 106 suites**.
- The first gate run found a consequence of D3. `any.rs`'s refusal test stored the
  operation by a bare alias, which now runs. Its clause changed, and a new
  `native_any.rs` test covers the alias (+1, the one over the prediction).
- The second run found another: `an_alias_of_a_generic_local_used_at_a_user_effect_is_refused_by_name`'s
  program now runs, so the test rebinds the alias's target to keep its guard pinned
  (+0).
