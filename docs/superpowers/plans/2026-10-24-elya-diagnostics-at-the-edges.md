# Plan: diagnostics at the edges

Spec: `docs/superpowers/specs/2026-10-24-elya-diagnostics-at-the-edges-design.md`.
Baseline: 1241 passed, 125 suites (0635bb1).

## Task 1: red tests

- `tests/diagnostics_edges.rs` (root, counted twice; +2 suites):
  1. `a_label_after_non_ascii_text_is_under_its_bytes` -- red: the header says `3:9`.
  2. `a_type_declared_in_two_files_names_both` -- red: one label.
  3. `an_operation_declared_twice_names_both` -- red: one label.
  4. `an_if_cut_off_by_the_end_of_the_file_is_one_error` -- red: two diagnostics.
  5. `every_deep_shape_is_one_e0102` -- red: the test process aborts (stack
     overflow on the 256 MiB thread at 50 000 levels).
  6. `the_limit_is_1024_levels` -- red: 1025 levels run (no E0102).
  7. `after_e0102_the_next_declaration_is_still_checked` -- red: no E0102 (1100
     levels, which survive the thread), only the E0200.
- `crates/cli/tests/run_cli.rs` (twice):
  8. `elya_check_on_deep_brackets_is_e0102` -- red: exit by signal (abort).
  9. `elya_run_on_1024_nested_blocks_prints_its_value` -- red: abort.
- `crates/cli/tests/build_cli.rs` (codegen, once):
  10. `elya_build_of_a_2000_line_main_runs` -- red: abort.

Prediction: 7 x 2 + 2 x 2 + 1 = **19 new tests: 1260 passed, 127 suites**.

## Task 2: D1 (renderer), D2 (E0209/E0202), D3 (one per position)
## Task 3: D4 (the limit in the parser)
## Task 4: D5 (the CLI's thread)
## Task 5: gate, controls (K1 D1, K2 D3, K3 D4's height, K4 D5), review

## Outcome

Red as predicted; green at 1260 / 127 as predicted. The review's F1-F3 added
two tests (one root, one CLI): 1264 / 127.
