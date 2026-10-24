# Record and replay as an Elya library: `import elya/replay`

**Status:** done (2026-10-24, branch `auto/elya`, a scheduled run). HANDOFF
step 1, ROADMAP priority 3. The design choices are Claude's under the
maintainer's delegation (rule 2), recorded here.

## 0. Measured first (at 730c301, gate 1229 / 121)

- `import elya/replay` is E0600 "no module `elya/replay`": the standard library
  is only the builtin modules (`io`, `int`, `float`, `time`, `op`).
- `examples/09_record_any.elya` writes a generic recorder and replayer with
  `with any`, 50 lines, which every program that wants them must copy.
- Its cost, record then replay of a loop performing `Tick.tick(n)` (debug
  build of the compiler, release-free native code):

  | performs | evaluator | native |
  |---|---|---|
  | 10 000 | 1.18 s | 0.036 s |
  | 100 000 | 10.7 s | 0.33 s |

  About 3.3 µs per operation natively for both passes, including `op.show` and
  `op.encode`; linear.
- The host trace (`elya run --record`, `ELYA_RECORD`) is the text `elya trace 1`
  then one `question = answer` line per answer, the question as `op.show`
  writes it (`time.now_ms() = 1729`).
- There is no string function beyond `<>`, `==` and the `to_string`s, and no
  file input, so an Elya program cannot read a trace's text back.

## 1. Decisions

- **D1. A standard module written in Elya.** `import elya/replay` loads an Elya
  source shipped inside the compiler (`std/replay.elya`, `include_str!`), as if
  it were the file `elya/replay.elya`: an ordinary module (part 1), bound as
  `replay`, its functions `replay.record` etc. (merged `elya.replay.record`).
  Its types and constructors are program-unique like any module's (part 1 D3):
  a program declaring one of their names and importing it is E0209.
  - Capabilities (part 2) apply unchanged: its interface performs nothing of
    its own (`record`'s row relays its body's), so it needs no list.
  - The builtin modules keep their meaning (`elya/io` etc.; a list on them is
    E0608).
  - *Rejected:* builtins in Rust (`replay.record` as a primitive). The point of
    the ROADMAP's third problem is that recording is an ordinary handler; the
    library must be the proof, and it runs natively with no new back-end work.
- **D2. The API.**
  - `record(body)`: runs `body`, asking the handlers outside, and returns
    `Recorded(value, trace)`.
  - `replay(body, trace)`: runs `body` answering every operation from `trace`;
    returns `Replayed(value)`, `Diverged(asked, held)` (the next question
    differs, or the trace ended: `held` is "the end of the trace"), or
    `Unconsumed(value, rest)`.
  - `text(trace)`: the trace as the host trace's text (`elya trace 1` and a
    `question = answer` line each), so a recording can be printed, saved and
    compared with a host trace.
  - `size(trace)`: the number of entries.
  - The trace is data: `type Trace { TraceEnd, TraceEntry(String, String, Trace) }`.
    Names are prefixed (`Trace…`, `Recorded`, `Replayed`, `Diverged`,
    `Unconsumed`) to keep clashes with a program's own names unlikely.
  - *Rejected for now:* reading a trace's text in Elya (needs string functions
    and file input, neither of which exists); recorded as the next step for
    this module.
- **D3. The same answers as the host replayer's words** where they meet: a
  divergence names the question asked and the one held.

## 2. Evidence (planned)

- `tests/std_replay.rs` (root, counted twice): the dice game and the shop of
  example 09 recorded and replayed through `import elya/replay` (the example's
  output, value unchanged); divergence (other dice), a trace ending early,
  leftover entries; `text` matches the host trace's shape; no list needed;
  a clash with a program's own `Trace` is E0209; `elya/replay` with a list
  granting nothing extra checks.
- `crates/codegen/tests/native_std_replay.rs`: the same program natively, the
  same bytes.
- Negative control: the embedded source's replay without its question check
  (`q == op.show(o)` always true): the divergence test fails.

## 3. Outcome and review

Red as predicted (E0600); green at 1238 / 124 as predicted; control K1 in
`tests/std_replay.rs`'s header. The `elya/` prefix is reserved: a user file
`elya/replay.elya` next to a program is not read (the embedded module is).

The independent review found no defect: nested recordings over two effects,
Float, Unit and string answers (escaped), empty bodies, programs that print,
aliases, a module and the root both importing it (loaded once), E0209 on
clashing names, no grant needed, the CLI's run/build/record/replay on disk, all
examples unchanged; the evaluator and native agree everywhere. Limitations it
recorded (each the existing `with any` behaviour, not the library's), now
HANDOFF material:

- inside a `with multi` handler of an `effect multi` that resumes twice,
  `record` fails at run time with "a one-shot continuation was resumed twice";
- `replay` cannot decode an answer whose type is the effect's own parameter
  (`effect Box(a) { fn get(x: a) -> a }`): "op.decode: ... not known at run time";
- `replay`'s body cannot perform `IO` (E0434: `with any` does not handle `IO`),
  where `record`'s can (it relays it);
- `text` writes the host's line shape, but a string containing ` = ` makes a
  line the host's `rsplit_once(" = ")` reader splits wrongly; the host records
  only `time.*` today, so nothing reads such a line yet. A reader must
  understand quotes before the two formats are one.
