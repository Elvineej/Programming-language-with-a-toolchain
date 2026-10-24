# Plan: strings to read a trace

Spec: `docs/superpowers/specs/2026-10-24-elya-trace-text-design.md`.
Baseline: 1284 passed, 132 suites (bac4834, measured).

## Task 1: red tests

- `tests/string_bytes.rs`: 6 tests (root, counted twice: +12, +2 suites).
  Red: E0201 "unknown builtin `string.length`" (and E0600 for `elya/string`).
- `tests/std_replay.rs`: +3 tests (+6). Red: E0201 on `replay.parse`.
- `tests/trace.rs`: +1 test (+2). Red: the error names the question
  `time.now_ms() = "a`.
- `crates/codegen/tests/native_string_bytes.rs`: 3 tests (+3, +1 suite). Red:
  the front end's E0201.

Prediction: **1307 passed, 135 suites**, with exactly these 13 new tests (23
counted) red before the code.

## Task 2: the builtins (D1)

`resolve.rs` (names, arity), `modules.rs` (`string`), `types.rs` (types),
`eval.rs` (both evaluators: the recursive one and the machine), codegen
(`lib.rs` and `cps_emit.rs`: `elya_str_length`, `elya_str_byte_at`,
`elya_str_slice` in `runtime.c`; `slice` roots what is live, as `<>`).

## Task 3: the line format (D2) and `replay.parse` (D3)

`lib.rs`'s reader through one `split_entry`; `std/replay.elya`'s `parse`.

## Task 4: gate, controls K1-K3, review
