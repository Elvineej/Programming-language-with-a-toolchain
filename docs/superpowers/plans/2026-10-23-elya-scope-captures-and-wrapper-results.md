# Captures by scope, and a wrapped function's result coerced -- plan

Spec: `docs/superpowers/specs/2026-10-23-elya-scope-captures-and-wrapper-results-design.md`.
Branch `auto/elya` at 2d8f62c. Baseline Linux gate: 1100 passed, 106 suites.

1. Red tests first (native and codegen unit tests count once):
   - `crates/codegen/tests/native_codegen.rs`:
     - the `SHADOWED_TOP_LEVEL` corpus (spec §0: h1-h10, q1, p0), run natively and
       against the evaluator, two tests. Predicted red: h1 and h4 give 2004, h8 and h9
       are refused, and q1 and p0 panic the compiler. +2.
     - the `WRAPPER_RESULTS` corpus (u1-u6), two tests. Predicted red: u1-u3 give 5,
       u4 a pointer, u5 3. +2.
   - `closure.rs`: `a_shadowing_local_is_captured_where_the_top_level_name_is_also_used`
     (h1's lambda captures `inc`). Predicted red. +1.
   - `anf.rs`: `a_wrappers_inner_call_has_the_functions_own_result` (u1: the inner
     call is typed at `mk`'s own result and bound by a `let`). Predicted red. +1.
2. `closure.rs`: `collect_in` threads the scope (D1).
3. `anf.rs`: `inner_type` instantiates the own result; `wrap` binds it when its
   convention differs (D2).
4. Controls, each reverted with `cmp`:
   - K1, captures by name again: h1 2004;
   - K2, the own result computed but not bound: u1 5;
   - K3, the own result bound but kept at the use's type: the same 5.
5. The sweep, and the review's fuzzers: no new disagreement. The build failures they
   listed at seeds 1074, 1151 and 1392 (inkwell panics) are re-run.

Predicted gate: 1100 + 6 = **1106 passed, 106 suites**.

Measured: **1106 passed, 106 suites**, as predicted. The review's F1 rows (u7, u8) were
added to an existing corpus (+0).
