# Record and replay from the command line

**Status:** done (2026-10-16). HANDOFF step 1 (ROADMAP priority 3's last step).
Design choices are Claude's under the maintainer's delegation (rule 2), recorded here.

## 0. Measured first

At `6e5794f`:

- `examples/05_replay.elya` and `examples/09_record_any.elya` record and replay USER
  effects with handlers written in Elya, inside one program. `elya run` has no flags.
- `main` may perform only `IO` (E0420 for any user effect reaching it), and `IO` is
  never handled -- not even by `with any` (spec 2026-10-16-elya-any-handlers §1). So a
  recorder wrapped around `main` from outside sees nothing a program does to the world.
- What the world answers today, in the evaluator: `time.now_ms()` (a number that differs
  run to run) and `time.sleep_ms(ms)` (Unit, after a wait). `io.println` answers nothing;
  its OUTPUT is the program's effect on the world. Everything else an Elya program does
  is a function of these answers: user effects are handled inside the program.

## 1. What is recorded: the world's answers (chosen)

- (a) Record every user-effect operation `main`'s callees perform, by wrapping `main`
  in a `with any` recorder. It cannot see `IO`, and a program's own handlers already
  answer its user effects deterministically -- recording them adds nothing that a
  replay could not recompute, and misses the one source of difference.
- (b) **Record the host builtins' answers (chosen).** The trace is the list of questions
  the program asked the host and the answers it got: `time.now_ms() = 1734` and
  `time.sleep_ms(5) = Unit`. Given the same source, those answers determine the whole
  run -- its output, its value, every handler's behaviour -- so replaying them reproduces
  it exactly, which is the property ROADMAP 3 promises ("bit for bit, anywhere").
  Output is not recorded: it is a consequence, and a replay prints it again (a test
  compares the two).

(b) is the stricter option: it records exactly the nondeterminism there is, and every
future host builtin (input, randomness, files) joins the trace by the same rule.

## 2. The trace and its failures

- **Format.** Text. First line `elya trace 1`; then one line per answer,
  `<question> = <answer>`, both in Elya's literal syntax (`literal::show`, the syntax
  `op.show` and `op.encode` use), in the order asked.
- **Replay** answers each host question from the next line and performs nothing:
  `time.sleep_ms` does not wait (the wait is part of the recorded world, not of the
  program), `time.now_ms` reads the trace. Each answer is decoded at the builtin's own
  result type, so a trace cannot inject a value of the wrong type.
- **Divergence stops by name** (E0300): "replay diverged at entry N: the program asked
  `Q`, the trace holds `Q'`" -- or "... the trace has ended". A trace with entries left
  when `main` returns is also an error ("the trace has K unread entries"): a replay that
  did not ask everything did not reproduce the run.
- **CLI:** `elya run --record trace.txt prog.elya` runs live and writes the trace;
  `elya run --replay trace.txt prog.elya` replays it. Evaluator only; natively the
  builtins call the runtime directly (a native trace is a later step).

## 3. Evidence

- `tests/trace.rs` (record then replay reproduces the output; a hand-written trace is the
  world; divergence at its entry; the trace ending; unread entries named; a wrong-typed
  answer refused; a failing run's trace; whitespace, CRLF and a byte-order mark) and
  `crates/cli/tests/run_cli.rs` (the binary records and replays; misplaced arguments are
  usage errors, exit 2, and the program is never overwritten).
- Gate: 1027 passed, 99 suites (predicted 1027) before the review; 1033 after it.

## 4. The independent review

Every recorded run replayed to the same stdout, stderr and exit code
(`examples/08_timers.elya`'s 22 answers, a deadline loop, time inside `with any` and
multi-shot handlers, `sleep_ms(i64::MAX)` replayed instantly). Fixed (each new test red
against the first version, by the review's measurements or the changed API):
- **F1 (medium).** A failing run wrote no trace; it is now written whether `main` returns
  or fails (`eval::run_module_keeping`), and the replay fails the same way.
- **F2-F3 (low).** `elya run prog --record t` ran live and recorded nothing; a flag or the
  program itself was accepted as the trace path (overwriting the source). Now exit 2 with
  the usage; plain `run` with extra arguments is a usage error too (it ignored them).
- **F4-F6 (low).** The unread-entries error names its count with the right noun and the
  first unread entry; lines are trimmed (and a byte-order mark skipped); an unreadable or
  unwritable trace exits 2 like an unreadable program.
- **F7 (latent).** The last ` = ` splits a line, which suits today's Int and Unit answers;
  the first String-answering builtin must quote-aware split (noted in the code).
