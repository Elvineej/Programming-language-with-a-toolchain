# Plan: the occurs checks look inside effect rows

Spec `docs/superpowers/specs/2026-10-30-elya-occurs-in-rows-design.md`. Gate before:
1331 passed, 141 suites (6b443af).

1. Red tests first, `tests/occurs_in_rows.rs` (root crate), two tests:
   - the three measured programs: predicted red by abort (stack overflow);
   - a task-typed effect without a cycle still checks and runs (green before and
     after).
2. D1 in `Infer::occurs`, D2 in `Infer::bind_row` (a helper: does a type mention a
   row variable, through rows' labels).
3. Controls, each reverted: K1, D1 off (the type cycle aborts again); K2, D2 off
   (the row cycle aborts again).

Predicted gate: 1331 + 2 x 2 (root tests run twice) = **1335 passed, 143 suites**.

The review's fixes add one test (`a_cycle_found_in_the_inclusion_fixpoint_ends_it`):
predicted gate 1335 + 1 x 2 = **1337 passed, 143 suites**.
