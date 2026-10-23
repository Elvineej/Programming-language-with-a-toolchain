# A failing traced run keeps its output on stdout

**Status:** done (2026-10-23, branch `auto/elya`, a scheduled run). HANDOFF
step 1. The design choices are Claude's under the maintainer's delegation (rule 2),
recorded here.

## 0. Measured first (at 0cf65fd, gate 1139 / 110)

`f.elya` prints `x`, reads the clock, then divides by zero; `g.elya` prints `y`,
then reads the clock, replayed from a trace holding only the header (a divergence).

| run | exit | stdout | stderr |
|---|---|---|---|
| `elya run f.elya` | 1 | `x` | the E0300 diagnostic |
| `elya run --record t f.elya` | 1 | (empty) | `x`, then the diagnostic |
| `elya run --replay t f.elya` | 1 | (empty) | the diagnostic (`x` is lost) |
| `elya run --replay h g.elya` | 1 | (empty) | "replay diverged at entry 1 ..." (`y` lost) |
| native `ELYA_RECORD=t ./f` | 1 | `x` | `elya: division by zero` |
| native `ELYA_REPLAY=t ./f` | 1 | `x` | `elya: division by zero` |
| native `ELYA_REPLAY=h ./g` | 1 | `y` | `elya: replay diverged ...` |
| `--replay` / native with `not a trace` | 1 | (empty) | "not a trace ..." on both |

The cause is the library: `run_source_recording` returns `Err(output + diagnostic)`
and `run_source_replaying` returns `Err(diagnostic)` alone, so the CLI cannot tell
the program's text from the error. Plain `elya run` was fixed the same way on
2026-10-16 (`run_source_outcome`, sweep spec D1).

## 1. Decisions

- **D1. Every evaluator run keeps the program's output on stdout and the
  diagnostic on stderr**, record and replay included, as plain `elya run` and a
  native binary do. The library gains `record_source(name, text) -> (Option<String>,
  RunOutcome)` and `replay_source(name, text, trace) -> RunOutcome`, the
  `RunOutcome` that `run_source_outcome` already returns (output, main's value,
  the rendered error); the CLI prints `output`, then the error, for all three.
  - The old `run_source_recording` / `run_source_replaying` stay as thin wrappers
    with their documented shapes (tests and the native trace tests compare against
    them); their doc comments now point at the outcome functions.
  - *Rejected:* changing the old functions' `Err` to omit the output and leaving
    the CLI to re-run: two runs of a program that reads the clock are two
    different runs.
- **D2. A trace that cannot be read runs nothing** (stdout empty), unchanged, as
  natively.
- **Not in this step:** `--value` together with `--record`/`--replay` (the usage
  stays exact: one flag).

## 2. Evidence (planned)

- `crates/cli/tests/run_cli.rs` (both configurations): a failing `--record` and a
  failing `--replay` (a runtime error, and a divergence) print the program's text
  on stdout and the error on stderr.
- `crates/cli/tests/build_cli.rs` (codegen): the CLI and the native binary print
  the same stdout and exit alike for: a failing record, its replay, a diverging
  replay, and a successful replay.
- Negative control: the CLI's trace path printing the old way (the output inside
  the error) fails both `run_cli.rs` tests.

## 3. Results

Gate: **1144 passed, 110 suites**, as predicted. The three tests were red first
(stdout empty where `x` was expected). The control failed both `run_cli.rs` tests.

The review built the old CLI, the new one and native binaries and compared them
byte for byte: the old wrappers behave exactly as before (`tests/trace.rs`,
`native_trace.rs` unchanged and green); a failing record's old stderr is the new
stdout followed by the new stderr; successful runs, front-end errors, bad and
unreadable traces are unchanged; the CLI and the native binary now agree on stdout
and exit code for every failing record and replay tried (divergence, leftover
entries, a bad answer, overflow). No defect found. Remaining differences are older
and by design: the native value line on success; a non-`Int` main has no native
build; very deep recursion is a native stack overflow while the evaluator runs on.
The traces the CLI and a binary write are not compared byte for byte (their clock
answers differ); the replay step does feed the CLI's trace to the binary.
