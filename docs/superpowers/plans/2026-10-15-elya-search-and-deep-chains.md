# A backtracking example and deep handler chains -- plan

Spec: `docs/superpowers/specs/2026-10-15-elya-search-and-deep-chains-design.md`. Branch
`auto/elya` at `511d6ed`. Baseline Linux gate: 916 passed, 87 suites.

- `tests/examples.rs::search` (red: no snapshot): +2 (counts twice).
- `native_codegen.rs`: `the_search_example_runs_natively` +1;
  `a_multi_shot_copy_of_100000_handler_frames` +1; the `MULTI_SHOT` row (no count change).

Predicted gate: 916 + 2 + 1 + 1 = **920 passed, 87 suites**.
