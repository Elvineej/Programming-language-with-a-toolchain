# Plan: `&&` and `||` short-circuit

Spec: `docs/superpowers/specs/2026-11-15-elya-short-circuit-design.md`.

## Steps

- [x] Red: tests/short_circuit.rs (6 tests), tests/programs/short_circuit.rs,
  crates/codegen/tests/native_short_circuit.rs (1 test). Before the change
  the two runtime tests fail, the skipped division and the effect order.
  The tail test runs a strict million-deep `||` and fails or runs very long.
- [x] The tree evaluator and the CEK machine (src/eval.rs): the right only
  when the left does not decide; in the CEK machine the right takes the
  frame's continuation, so it is in tail position.
- [x] Core lowering (src/core.rs): `If`, so native code needs no change.
- [x] The affine checker: built (the right operand as one branch in the use
  counts, the give-away sets and the liveness walk), then reverted, because
  K5 to K7 below showed it changes no verdict (spec D3).
- [x] Controls, each reverted and compared byte for byte (results below).
- [x] Gate `sh scripts/check.sh`. Predicted 1640 passed, 190 suites: wrong.
  The prediction counted tests/short_circuit.rs once, but configurations A
  and B both run the `elya` suites. Measured: 1654 passed, 193 suites over
  every configuration, against 1641 and 190 for the last landing counted the
  same way: +13 tests (6 twice, and the native one) and +3 suites. Without
  configuration C (boards: 8 tests, 2 suites), as HANDOFF counts: 1646 and
  191, against 1633 and 188.
- [ ] Windows CI on a `claude/*` branch; fast-forward main; HANDOFF; E:.

## Controls

Each applied, run, reverted with `cmp` against the saved file:

- K1, the tree evaluator strict again (its short-circuit arm never taken):
  `the_right_operand_runs_only_when_it_decides` and
  `effects_on_the_right_happen_only_when_it_runs` fail.
- K2, the CEK machine never letting the left decide: those two and
  `an_index_guard_keeps_the_read_in_bounds` fail, and
  `the_right_operand_is_in_tail_position` never ends (`reaches(0)` goes on to
  `reaches(-1)`).
- K3, the CEK machine leaving two frames under the right operand:
  `the_right_operand_is_in_tail_position` fails (the peak continuation depth
  grows with the calls).
- K4, Core lowering `&&` to the strict `Prim` again:
  `short_circuits_natively_as_the_evaluator_does` fails.
- K5 to K7, the affine checker's right operand on every path again (in the
  give-away sets, the use counts and the liveness walk, one at a time): not
  caught, and cannot be (spec D3). The change was reverted.
- K8, after the review: `may_allocate` and `allocates_here` without their `If`
  arms: `a_short_circuit_argument_roots_nothing` fails.

## Review

An independent review found no correctness defect, and could not break D3 (ten
more pairs, all equal). It found: a native rooting regression, since `If` was
"may allocate" (fixed, D4, K8); the CEK machine not checking the right's type
(kept, D5); and three gaps, now tested: an effect on the right
(`an_effect_on_the_right_is_performed_only_when_it_runs`, natively too), E0429
and E0439 (`the_affine_checker_sees_the_if_with_points_and_borrows`), and the
rooting. Gate after it: predicted 1659 passed, 193 suites (the last
landing's 1641 and 190, +16 for the eight tests in two configurations, +1
native, +1 rooting; 3 suites); measured 1659 and 193. Without configuration C,
as HANDOFF counts: 1651 and 191.
