# `with any` natively -- plan

Spec: `docs/superpowers/specs/2026-10-17-elya-native-any-handlers-design.md`. Branch
`auto/elya` at `1cf9bb5`. Baseline Linux gate: 1054 passed, 101 suites.

1. `crates/codegen/tests/native_any.rs` (native: counts once), red first. Every test
   but the refusal one is red with `core lowering failed: Unsupported("operation-generic
   handler ...")`; the refusal test is red because the refusals it expects are not
   the ones reached (Core refuses first):
   - `the_record_any_example_runs_natively`
   - `forwarding_resumes_at_every_answer_type`
   - `names_and_questions_are_the_evaluators`
   - `answers_round_trip_through_declared_types`
   - `a_hundred_thousand_cell_list_round_trips`
   - `decode_failures_have_the_evaluators_words`
   - `showing_a_function_fails_by_name`
   - `operations_compare_by_what_they_are`
   - `nested_and_local_any_handlers`
   - `what_is_not_compiled_is_refused_by_name`
   +10 tests, +1 suite.
2. Core (D6): `CoreModule::effects`, `CoreHandle::any`; lowering stops refusing. The
   changed expectation in `tests/core_lowering.rs` (renamed, same count).
3. `any.rs` (D1, D2, D7) with three LLVM-free unit tests: +3.
4. `instantiate_types` demands a decode target's closure (D5).
5. Runtime `elya_any_show`, `elya_any_decode`, `elya_any_fail`; the reflection table
   and the builtins in both emitters (D3, D4).
6. Negative controls K1-K4 (spec §2).

Predicted gate: 1054 + 10 + 3 = **1067 passed, 102 suites**.

Predicted sweep (the 9 programs of §0): 6 build and agree with the evaluator (09,
0395, 0409, 0412 as a failure with the same words, 0419, 0434); 3 refused by name
(0391 and 0399 `Float`, 0410 `too many type instances`).

Measured: **1067 passed, 102 suites**, as predicted. After the review's fixes (two native
tests, both red first): predicted and measured below in the spec's §3.
