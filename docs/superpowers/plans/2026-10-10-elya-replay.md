# Exact replay -- plan

Spec: `docs/superpowers/specs/2026-10-10-elya-replay-design.md`. Branch `auto/elya` at
`c684b0b`. Baseline Linux gate: 873 passed, 83 suites.

- `examples/05_replay.elya` (written first, as the measurement; its crosscheck row was red
  -- the tree-walker list did not name it -- until listed).
- `tests/replay.rs`: seeds 1..20 replay exactly (score in 6..72); divergence 806 (an
  8-sided roll after 6 answers); leftovers: predicted 2, observed **3** (my count missed
  the clock reading `game` takes between rounds -- three answers per round; the test
  expectation was set from the corrected count and the comment says why); the test double
  84 and 120. Counts twice: +8.
- `tests/examples.rs::replay` snapshot: +2. Native `the_replay_example_runs_natively`: +1.

Predicted gate: 873 + 8 + 2 + 1 = **884 passed, 85 suites**.

Review added one test (R3's): final gate predicted 884 + 2 = **886 passed, 85 suites**.
