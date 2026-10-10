# Native multi-shot handlers: plan

Spec: `docs/superpowers/specs/2026-10-10-elya-native-multi-shot-design.md`.
Branch `auto/elya` at `f65937a`. Baseline Linux gate: 836 passed, 79 suites.
Root-crate tests count twice; codegen-crate tests (unit and native) once.

## Task 1 — Core carries `with multi` (red first)

- `tests/core_lowering.rs`: `with_multi_alone_stamps_the_handler_multi_bit` -- `with multi`
  -> `multi = true`; plain `with` over a `multi` effect -> `multi = false` with
  `is_multi_declared = true`. [red: no field] Counts twice: +2.

## Task 2 — the copy, the rule, and the corpus (red first)

Native (`crates/codegen/tests/native_codegen.rs`, today's result in brackets):
- `the_multi_shot_corpus_prints_the_evaluators_values` -- m1 12, m2 12012000, m3 40100,
  m4 5, m7 12, m8 10199898, m10 10012, m11 3612120, m12 1024000.
  [refused "multi-shot handler"]
- `native_output_matches_the_evaluator_across_the_multi_shot_corpus` [refused]
- `a_plain_handler_over_a_multi_effect_is_one_shot_natively` -- m5 prints 1; m6 exits
  non-zero with "resumed twice" on stderr, and the evaluator says E0425. [refused]
- `a_one_shot_continuation_saved_in_a_multi_shot_segment_fails_on_both_sides` -- m9.
  [refused]
- `a_multi_shot_loops_live_set_settles` -- live equal at N = 250, 500, 1000, 2000,
  collections > 0. [refused]
- `copies_survive_a_collection_mid_copy` -- m11 with ELY_GC_STATS: collections > 0 and
  3612120. [refused]

Unit (`crates/codegen/src/lib.rs`), expectation changes the step requires (each replaced
red-first, named in the commit):
- `a_multi_shot_handler_is_refused_by_its_own_name` -> `a_multi_shot_handler_compiles`.
- `the_multi_refusal_fires_before_any_clause_body_is_lowered` ->
  `a_multi_shot_handlers_clause_bodies_are_lowered` (the poisonous clause now refuses with
  ITS OWN message, "unbound").
- `a_multi_shot_handler_written_in_source_reaches_the_codegen_refusal` ->
  `a_multi_shot_handler_written_in_source_compiles`.
- New: `handler_tags_sit_contiguously_below_the_continuation_tag`. +1.

Predicted final gate: 836 + 2 (Task 1) + 6 native + 1 unit = **845 passed, 79 suites**.

## Task 3 — negative controls, each reverted (`git diff --quiet`)

K1 no copy -> m3 prints 40200. K2 no parent remap -> m10 prints 10024, m3 still 40100.
K3 the one-shot flag on multi resumes -> m1 traps "resumed twice". K4 `innermost` not
remapped -> m10 wrong.

## Task 4 — independent review, close-out (spec §3, HANDOFF, log).
