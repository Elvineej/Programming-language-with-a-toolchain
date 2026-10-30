# What `main` is worth, for every data type -- plan

Spec: `docs/superpowers/specs/2026-10-30-elya-main-values-design.md`. Branch `auto/elya`
at aabce93 (the trace-text slice). Baseline Linux gate: 1310 passed, 135 suites.

1. Red tests:
   - `crates/codegen/tests/native_main_values.rs` (new suite, native: once each):
     - `every_data_main_prints_its_literal`: 11 mains, including the generic `None`
       and `Nil`, against `RunOutcome::line`;
     - `a_main_that_holds_a_function_fails_by_name`: 4.

     Predicted red: the field does not exist (a compile error), then "non-Int
     value". +2, +1 suite.
   - `crates/cli/tests/run_cli.rs`: the `--value` test prints `True` and a
     `P(String, Float)`, and names a function main's failure. A changed expectation
     (D1): it pinned the `Bool` refusal. +0.
   - Two codegen unit tests pinned "non-Int value" for a `Bool` main
     (`rejects_bool_literal_specifically`,
     `a_bool_returning_helper_compiles_but_a_bool_main_still_refuses`). They become
     compile checks under new names: changed expectations (D1). +0.
2. `RunOutcome::line` (`src/lib.rs`), which the CLI's `--value` prints.
3. Native: `main`'s type in the reflection table; `elya_entry` writes the line
   through `elya_any_show` and the shim prints no `%lld` for it. Monomorphization
   instantiates a generic `main` at its defaults (D3).
4. Controls:
   - K1, main's type left out of the table: an internal error, named;
   - K2, no default instance: "no `main`" again;
   - K3, the shim's `%lld` line printed too: a `0` after the literal.

Predicted gate: 1310 + 2 = **1312 passed, 136 suites**.
Measured: **1312 passed, 136 suites**, as predicted.

5. The review's fixes (spec §3), red first: two native tests (the replay order,
   the shared stream), one mono unit test, and rows added to two existing tests.
   Predicted gate: 1312 + 3 = **1315 passed, 136 suites**.
