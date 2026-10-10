# The evaluator's continuation: linear cost -- plan

Spec: `docs/superpowers/specs/2026-10-10-elya-eval-linear-continuations-design.md`.
Branch `auto/elya` at `a583181`. Baseline Linux gate: 845 passed, 79 suites.
Root-crate tests count twice.

## Task 1 -- the counter, and red tests on the old machine

`Interp::cost()` added to the current machine (steps + the probe's walk + perform's walk +
resume's re-push). `tests/eval_complexity.rs` (predicted on the old machine):

- `non_tail_recursion_costs_linearly` -- ratio ~4. [red: 3.99]
- `non_tail_recursion_under_a_handler_costs_linearly` -- ~4. [red: 3.99]
- `a_perform_costs_its_handler_distance_not_its_depth` -- ~4. [red: 3.99]
- `a_perform_past_other_handlers_costs_linearly` -- ~4. [red: 3.98]
- `a_multi_shot_resume_at_depth_costs_linearly` -- ~4. [red: 3.98]
- `the_parked_shape_runs_a_hundred_thousand_deep` -- 200000. [red: does not finish]
- `an_abandoned_continuation_a_hundred_thousand_deep_drops` -- 7. Guard: predicted green
  before (the old capture was a `Vec`). Observed: timed out at 2 minutes on the old
  machine, through the depth probe; it is a guard for D3 (control K4).

## Task 2 -- segmented continuation (D1-D3)

Predicted after: every ratio 2.00 (+-0.01); cost(1000) of `len(deep(1000))` drops from
16 096 054 to under 60 000 (steps only); the 100 000-deep test under 5 s in debug.
Controls K1-K4 each fail as the spec says, reverted.

Predicted final gate: 845 + 7 x 2 = **859 passed, 81 suites** (a new root test binary is a
suite in both configurations).

## Outcome

Gate **859 passed, 81 suites** (as predicted). Ratios 2.00 everywhere; `len(deep(1000))`
costs 32 022. K1, K2, K4 as predicted; K3 failed the two perform tests but not the
multi-shot one (spec §4 explains). Independent review: no findings (19 hand-written and
2 361 generated programs, old and new machines byte-identical, including E0425 and peak
depth).
