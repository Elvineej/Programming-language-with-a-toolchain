# Deep non-tail recursion natively -- plan

Spec: `docs/superpowers/specs/2026-10-16-elya-native-stack-design.md`. Branch `auto/elya`
at `ab791b6`. Baseline Linux gate: 1033 passed, 99 suites.

1. Tests first (`crates/codegen/tests/native_codegen.rs`), each red with the predicted
   failure:
   - `deep_non_tail_recursion_runs_natively_a_million_deep` (m3, m5, m6; red: SIGSEGV,
     rc 139): +1.
   - `deep_non_tail_recursion_matches_the_evaluator` (m2, m5 at 200 000; red: SIGSEGV):
     +1.
   - `a_native_stack_overflow_is_named` (one line printed, then unbounded recursion,
     `ELYA_STACK_MIB=8`; red: SIGSEGV, no message): +1.
   - `elya_stack_mib_sets_the_stack_and_is_checked` (m2 under 8 MiB: named overflow;
     `0` and `lots`: named errors; red: m2 SIGSEGV unnamed, `0` runs normally): +1.
   - Every existing run of a binary in that file goes through one helper that sets
     `ELYA_STACK_MIB=8` (D3; no expected value changes, count unchanged).
2. Runtime `elya_start` (POSIX and Windows), the `@elya_entry` wrapper and the shim
   change, `-pthread` at link.
3. Negative controls K1-K3, recorded in the test file's header.

Predicted gate: 1033 + 4 = **1037 passed, 99 suites** (native tests count once).

Measured before the review: **1037 passed, 99 suites**. The review added two tests
(`elya_functions_may_be_named_like_runtime_symbols`, `foreign_faults_still_kill_by_their_signal`,
both red first): predicted **1039 passed, 99 suites**.
