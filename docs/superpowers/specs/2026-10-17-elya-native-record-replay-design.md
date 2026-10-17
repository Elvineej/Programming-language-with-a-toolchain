# Record and replay natively

**Status:** in progress (2026-10-17, branch `auto/elya`). HANDOFF step 1. The design
choices are Claude's under the maintainer's delegation (rule 2), recorded here.
Builds on `2026-10-16-elya-cli-record-replay-design.md` (the trace, its format and its
failures, in the evaluator).

## 0. Measured first (at 005b0d2)

- `elya run --record T prog` / `--replay T prog` work in the evaluator. A native
  binary calls `elya_time_now_ms` / `elya_time_sleep_ms` in the runtime directly: it
  has no way to write or read a trace, so a native run of a timer program
  (`examples/08_timers.elya`, `tests/trace.rs`'s clock programs) cannot be reproduced,
  and the sweep files such programs as `differ-timing` (2 of 700).
- The runtime is the only place every native time question passes through (both
  emitters call the same two functions), so a trace there covers every program.

## 1. Decisions

- **D1. Environment variables, read by the runtime: `ELYA_RECORD=<file>` and
  `ELYA_REPLAY=<file>`.** A native binary has no CLI of its own (its arguments are not
  Elya's), so its configuration is the environment, as `ELYA_STACK_MIB` already is.
  Both set is refused before the program runs ("ELYA_RECORD and ELYA_REPLAY cannot
  both be set"). *Rejected:* a flag compiled in by `elya build`: a recording is a
  property of a run, not of a build.
- **D2. The same trace, byte for byte.** `elya trace 1`, then `<question> = <answer>`
  per answer in the order asked (`time.now_ms() = 17`, `time.sleep_ms(5) = Unit`); read
  with the evaluator's rules (a byte-order mark skipped, lines trimmed, blank lines
  ignored, the LAST ` = ` splits, an `Int` answer read as Rust's `i64::from_str`
  reads it). A trace recorded by `elya run --record` replays natively and the reverse.
- **D3. The same failures, in the same words** (`elya: ` then the evaluator's E0300
  message, exit 1): a divergence (`replay diverged at entry N: the program asked `Q`,
  the trace holds `Q'``, or `..., the trace has ended`), an answer of the wrong type
  (`replay entry N: `a` is not an answer of type Int`), and entries left when `main`
  returns (`replay: `main` returned with K unread trace entries, from entry N (`q =
  a`): the run asked less than the recording`), checked after `main` returns and
  before its value is printed. A trace whose first line is not the header is
  `elya: not a trace: the first line must be `elya trace 1``.
  - A malformed trace (no header, a line without ` = `) exits 1 with the CLI's
    words (`not a trace: ...`, `trace line N: expected `question = answer``), as
    `elya run --replay` does. An unreadable or unwritable trace file exits 2
    (`elya: cannot read the trace `T`: ...`), as the CLI does for one.
- **D4. Written as it goes.** Recording appends and flushes each entry, so a run that
  stops by a trap (overflow, a failed match, a shift out of range) leaves the trace of
  what it asked, which replays to the same failure (the CLI's F1).
- **D5. Replay does not wait:** `time.sleep_ms` answers from the trace at once, as in
  the evaluator.

## 2. Evidence (planned)

`crates/codegen/tests/native_trace.rs`: a trace recorded by the evaluator replays
natively to the same output, and the reverse; a native trace replays natively; every
failure of D3 in the evaluator's words; a failing native run's trace; both variables
refused; an unreadable trace exits 2. Negative controls recorded in its header.

## 3. Results

Negative controls K1-K3 recorded in `native_trace.rs`'s header; each failed a distinct
test, the runtime restored byte for byte.

Gate: 1084 passed, 106 suites, as predicted; 1086 after the review's fixes (two native
tests, both red first).

### The review (2026-10-17)

About 70 cases; every D3 message, round trips both ways (the timers example, a
`with multi` handler between two resumes, 100 000 entries) held. Fixed test-first:
- **M1.** A write of the trace that fails mid-run (a full disk) was ignored, exit 0.
  Now `elya: cannot write the trace `T`: ...`, exit 2, as the CLI.
- **M2.** Lines were trimmed of ASCII whitespace only; Rust's `trim` also strips
  Unicode whitespace (U+00A0, U+2003, ...). Now the same set.
- **M3.** A trace that is not UTF-8 was read as bytes; the CLI refuses it
  (`stream did not contain valid UTF-8`, exit 2). Now natively too.
- **L1.** A NUL byte cut a line natively (C strings). Entries now carry lengths.
- Older than this slice, recorded: `elya run --replay` drops the output printed
  before a failure and `--record` writes it to stderr, where plain `run` and a
  native binary keep it on stdout (HANDOFF's open items).
