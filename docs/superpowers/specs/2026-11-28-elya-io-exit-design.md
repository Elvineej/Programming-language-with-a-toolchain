# `io.exit`: a program ends with a status of its choosing

2026-11-28. Status: decided. Step (1) of spec
`2026-11-28-elya-retiring-rust-design.md` T1. Design choices are Claude's
(rule 2).

## 0. Measured first

The driver in Elya (`selfhost/elya.elya`) returns its status as `main`'s
value, which a native program prints as a line (`0` built, `1` not) and then
ends with status 0 either way; a shell or a build script cannot tell a failed
build from a good one. Nothing in Elya ends a program with a status: a
failure by name (`elya: ...`) ends with 1, a normal end with 0.

## 1. Decisions

**D1. `io.exit(status: Int) -> Unit`, performing `IO`.** It ends the program
at once with `status`, after the program's output is written; nothing after
it runs, and `main`'s value line is not printed (there is no value). The
status must be 0 to 255, the range every platform keeps whole; another fails
by name, `elya: io.exit: status <n> is outside 0..255`, status 1. Typed
`Unit` (Elya has no type for "does not return"), so a branch that exits
still answers its type, e.g. `{ io.exit(1)\n 0 }`.

**D2. Not recorded.** It asks the world nothing: a trace holds no entry for
it, and a replay ends where the recording ended, by the same call.

**D3. The evaluator:** a `RuntimeError` carries `exit: Option<i32>`;
`io.exit` returns one, which unwinds the run like a failure but is not one:
`RunOutcome` gains `status`, its `error` stays `None`, and its output is kept.
`elya run` (live, `--value`, record and replay) prints the output and ends with
that status.

**D4. Natively:** `elya_io_exit(int64_t)` in the runtime (both back ends call
it): the range check, the output flushed, `elya_platform_exit(status)`. A board
ends the program as its `exit` hook does.

**D5. The twins:** the resolver's, checker's and back end's tables in Elya;
the driver ends with its status (`io.exit(build(...))`).
