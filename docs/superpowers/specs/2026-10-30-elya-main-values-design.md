# What `main` is worth, for every data type

**Status:** in progress (2026-10-30, branch `auto/elya`, an interactive session).
HANDOFF step 1 (the sweep's last native refusals). The design choices are Claude's
under the maintainer's delegation (rule 2), recorded here.

## 0. Measured first (at the trace-text slice, gate 1308 / 135)

The differential sweep's last native refusals, at the 2026-10-23 sweep and since:
- **two "non-Int value"** (a `Bool` main, a `Float` main);
- **two "too many type instances"**. Both decode at the non-regular
  `Nest(a) { Z(a), S(Nest(Opt(a))) }` (`any_handler.rs`, `native_any.rs`).

A `main` of each type, on each side:

| `main`'s value | `elya run` | `elya run --value` | native build |
|---|---|---|---|
| `7` (Int) | nothing | `7` | `7` |
| `Unit` | nothing | `0` | `0` |
| `True`, `1.5`, `-0.0`, `"hi\n\"x\""` | nothing | refused: "main's value is not an `Int` or `Unit`, so it has no native value line" | refused, "non-Int value" |
| `Some(3)`, `Cons(1, Cons(2, Nil))`, `Box(1.0 /. 0.0)` | nothing | refused, the same | refused, the same |
| `None` (a generic main: `Option(a)`) | nothing | refused, the same | **"no `main`"** |
| `fn(x) { x }` (a generic function) | nothing | refused, the same | **"no `main`"** |
| `fn(x: Int) { x }` | nothing | refused, the same | "non-Int value" |

- "no `main`" is wrong words: there is a `main`. Type monomorphization clones a generic
  function only at its uses, and nothing uses `main`.
- The evaluator already writes every data value as a literal (`literal::show`,
  `op.encode`'s text). Natively, `elya_any_show` writes the same text through the
  reflection table, in the same words when it reaches a function.
- The instance budget is hit only by the non-regular type. Its instances are
  `Nest(Int)`, `Nest(Opt(Int))`, ... without end, and the 2026-10-17 spec's D5
  refuses that by name on purpose.

## 1. Decisions

- **D1. The value line is the value's literal, for every data type.**
  - `elya run --value` and a native binary print `main`'s value after its output as
    `op.encode` writes it: `True`, `1.5`, `-0.0`, `"hi\n\"x\""`, `Some(3)`,
    `Cons(1, Cons(2, Nil))`, `Box(inf)`.
  - `Int` is unchanged (its literal is its digits).
  - `Unit` stays `0`, as the shim has always printed it. Every native test reads a
    `Unit` main's value line as `0`, and `--value` was made to match it
    (2026-10-16). Changing it to `Unit` would change dozens of expectations for no
    program's benefit.
  - Natively, the shim prints through `elya_any_show` and `main`'s type joins the
    reflection table.
  - *Rejected:* a separate printer per type in the shim. The reflection table
    already writes these literals in the evaluator's words, including the failure.
- **D2. A value that is or holds a function fails by name, after the run.**
  - Both sides say "main's value is a function, which is not data" and exit 1, with
    the program's output kept. That is `op.encode`'s rule and words, with `main`'s
    value in place of an answer; a continuation is "a continuation".
  - It is a run-time failure because a field's function is found only when the
    walk reaches it. `elya run` without `--value` prints no value, so it stays as it
    is.
  - *Rejected:* a check-time error for a function-typed `main`. A `main` that
    returns a closure is a fine program to `elya run`; only the value line cannot
    write it.
- **D3. A generic `main` is instantiated at its defaults.** Type monomorphization
  treats `main` as used once, at its own type with every unconstrained variable
  `Unit` (the ambiguity default, N7 part 2 §2). So `pub fn main() { None }` builds and
  prints `None`.
- **D4. The instance budget stays.** Both programs that reach it decode at a
  non-regular type, which has no finite set of instances. The named refusal is the
  2026-10-17 spec's D5. Recorded as measured; no change.

## 2. Evidence

- `crates/cli/tests/run_cli.rs`: `--value` prints each data type's literal; a
  function-valued `main` fails by name and keeps the output. This changes the test
  that pinned the refusal ("refuses a Bool main by name", 2026-10-16), a decision
  (D1).
- `crates/codegen/tests/native_main_values.rs`: every row of §0 natively against
  `--value`'s text, the generic `main`s included (D3), and the function-valued ones
  failing in the same words (D2).
- The sweep: no "non-Int value" refusal left.

Measured (2026-10-30):
- Red first as planned: the native suite did not compile (no `RunOutcome::line`),
  and the CLI test refused `True`.
- Written while making the rows: `io.println("out")  -0.0` is a SUBTRACTION. A `-`
  continues the line it is on, and the layout rule only ends a statement at a
  line's start. The row writes `0.0 *. -1.0`.
- **Changed expectations (D1):** the CLI `--value` test, and two codegen unit tests
  renamed `a_bool_literal_main_compiles` and
  `a_bool_returning_helper_and_a_bool_main_compile`.
- Controls K1-K3 (the native suite's header): each failed distinctly.

## 3. Review (an independent agent, 2026-10-30), each finding fixed test-first

- **F1, high: the evaluator wrote a constructor used as a function as data.**
  `pub fn main() { Some }` printed `Some`, a field-less value of the same name;
  natively it is a closure and fails "a function". `op.encode` and `op.show` had the
  same fault (a `resume(Yes)` answer encoded as `Yes`), older than this slice.
  `literal::show` now takes the module's constructor table (`Interp::types`) and a
  constructor with fewer arguments than fields is "a function". Rows: `Some` and
  `Cons(Some, Nil)` in `native_main_values.rs`, `showctor` and `encctor` in
  `native_any.rs`. The message "main's value is not an `Int` or `Unit`, so it has
  no native value line" lost its last clause, which is no longer true (L2).
- **F2, medium: natively, the value line came before a replay's unread-entry
  check.** `elya_entry` wrote `True` and `elya_start` then failed; the evaluator
  fails first and writes nothing. A function-valued `main` failed on its value
  instead of on the unread entry. `@elya_entry` now calls `elya_trace_finish`
  (no longer `static`) before it shows the value; `elya_start` repeats it and finds
  the same, since showing reads no entry. Test: `a_value_line_comes_after_the_replay_check`.
- **L1, low: `elya_any_show`'s failure did not flush stdout first**, so on one
  stream the failure came before the output. Fixed; the `op.show` and `op.encode`
  failures share the path. Test: `the_output_comes_before_a_value_failure_on_one_stream`.
- **L2, low: `RunOutcome::value`'s doc was stale.** Rewritten (`line` is what is
  printed); the three native helpers that built the expected line from `value` read
  `line`.
- **L3, low: a recursive generic `main` was cloned twice** (`main` and
  `main.mono.1`): the default instance was not registered. Registered in `top`.
  Test: mono's `a_recursive_generic_main_calls_itself`.
