# Plan: `Float` natively

Spec: `docs/superpowers/specs/2026-10-23-elya-float-natively-design.md`.
Baseline: 1184 passed, 113 suites (9233648).

## Task 1: red tests

`tests/float_text.rs` (counted twice):
1. `float_to_string_writes_the_literal_text`: the §0 table through
   `float.to_string` and `io.println`. Predicted red: E0201 "unknown builtin
   `float.to_string`".
2. `float_to_string_is_typed`: pure (`/ {}`), `Float -> String`; `True` and
   two arguments rejected. Red: E0201.

`crates/codegen/tests/native_float.rs` (counted once), each program compiled, run
and compared with the evaluator:
3. `float_arithmetic_agrees`. Red: "core lowering failed: Unsupported(\"Float\")".
4. `float_equality_agrees` (D2). Red: the same.
5. `floats_live_in_fields_captures_and_frames`. Red: the same.
6. `with_any_shows_encodes_and_decodes_floats`. Red: the same.
7. `the_printer_matches_the_evaluator` (D3): the C harness. Red: the harness does
   not link (`elya_float_text` undefined).

Prediction: **1193 passed, 116 suites** (+2 x 2, +5; two new root suites, one
native).

## Task 2: the evaluator and the front end

`float.to_string` in `resolve` (builtins, arity), `types` (`Float -> String`,
pure), `eval` (both machines) through `literal::float_text` (the existing
`write_float`, made public).

## Task 3: native

- Core: `CoreLit::Float(bits)`; every `CoreLit` match in codegen.
- `repr_ty`, `value_to_word`, `word_to_value`: `Float` is an `i64` word.
- Both emitters: the float literal, `+. -. *. /.` (bitcast, op, bitcast), `==`
  and `!=` (`fcmp`), `float.to_string` (a runtime call).
- The float mask table and `elya_eq_init`; `elya_eq` and `elya_any_check`.
- Reflection: `KIND_FLOAT`; `elya_any_show` prints, `any_parse` keeps the bits,
  decode builds the word.
- `runtime.c`: `elya_float_text` (D3), `elya_float_to_string`.

## Task 4: gate, controls, review, sweep

`sh scripts/check.sh` at 1193 / 116; the three controls; a review; the sweep (the
3 `Float` build errors become `same`).
