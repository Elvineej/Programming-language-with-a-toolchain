# Plan: the affine guarantee across functions

Spec: `docs/superpowers/specs/2026-11-06-elya-affine-across-functions-design.md`
(D1-D8). Base: `main` at b08164a.

## Steps

1. Measure (done, spec §0): 17 programs reading a `File` twice check clean,
   3 sound ones refused.
2. Red tests, each failing first with the predicted value:
   - `tests/affine.rs` (+10): parameters (U1, U2, U7, U8: E0428; S5
     clean); arms (S1, S2, S4 clean; a use after a branch that used it:
     E0428); closures by their calls (U3, U5: E0428; S3, S7 clean); closures
     passed on (U16, U19: E0428); containers and binders (U4, U9 and a
     non-generic holder: E0428); clauses (U11, U20: E0428; S9 clean); generic
     functions (U15, U17, U18 and one through two generic calls: E0437; S8
     clean); multi-shot points (U12, U13: E0429; S6 clean); held types on row
     variables (U14, U24: E0437; S10 clean); `with any` over an operation
     taking a `File` (E0437). Before: every program predicted refused checks
     clean (U6, U10, U20 and U23, already refused, are not among them), and
     S1, S2 and S6 are refused.
   - `tests/affine.rs`: `callee_duplication_is_currently_accepted_known_gap`
     becomes `a_callee_duplicating_its_parameter_is_e0428` (an expectation
     changed: the pinned gap closes, as 4d-2 meant it to).
   - `tests/ui/affine_through_generic.elya` (+1, E0437).
   - `src/affine.rs` unit tests (+2): the bindings counted many and the held
     sets of two small programs.
   - `tests/infer_complexity.rs` (+1): affine bindings and the closures that
     consume them down a block, linear.
3. `src/affine.rs`: the syntactic pass (D2, D3, D7): binders resolved once;
   counts by arm, scaled by a lambda's calls and a clause's many; held sets
   by a backward walk per body, a `handle` body continuing into its return
   clause; a let-bound lambda's use standing for its captures.
4. `src/types.rs`: duplicable types (D1, D5): marks on type variables,
   checked by `bind`, copied by `instantiate`; held types on row variables,
   applied or moved by `bind_row`, copied by `instantiate`; imposed at every
   local binding and at every point; diagnostics apart (D6, D7). `lib.rs`
   reports them after exhaustiveness.
5. README "Affine resources": the guarantee as spec §2 states it.
6. Controls, each reverted and compared byte for byte:
   - K34 the arms summed: S1 refused (arms test);
   - K35 a let-bound lambda's uses not scaled by its calls: U3 clean;
   - K36 a lambda passed on scaled once: U16 clean;
   - K37 `instantiate` not copying marks: U15 clean;
   - K38 `bind` not checking marks: U2 clean;
   - K39 held types not copied by `instantiate`: U14 clean;
   - K40 held types not moved down a row tail: U24 clean;
   - K41 a `handle` body's points holding what follows the `handle`: S6
     refused;
   - K42 a let-bound lambda's use not standing for its captures (held sets):
     a closure called after a `multi` perform clean;
   - K43 D5 not checked: the `with any` test.
7. An independent review.

## Predictions

- Gate: 1463 + (10 + 1 + 2 + 1) x 2 = **1491 passed, 157 suites**.
- One existing expectation changes (step 2's renamed test); E0428 and E0429
  keep their messages, so the two ui fixtures and the other affine tests pass
  unchanged.
- Inference's steps unchanged on the complexity shapes (the pass adds no
  unification); `elya check` on the scalability shapes within 10% of
  before.
- `gc_mark` untouched (no runtime change).
