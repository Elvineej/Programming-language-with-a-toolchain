# Self-hosting, part 1: files, standard input and arguments

**Status:** designed (2026-11-14, on `main`, an interactive session). HANDOFF
step 1, part 1 (spec 2026-11-14-elya-self-hosting R3). Design choices are
Claude's (rule 2). It answers the old step 3's questions ("Reading text from
outside").

## 0. Measured first

- No Elya program reads a file, standard input or its command line, or writes
  a file. The host's builtins are `io.println` and `time.now_ms` /
  `time.sleep_ms`; each performs `IO`.
- Host answers are recorded and replayed (spec 2026-10-16-elya-cli-record-replay):
  a trace line `time.now_ms() = 27`, the question in the program's own literal
  syntax and the answer as a literal of the builtin's result type; the
  evaluator (`eval.rs`, `host_builtin`) and the runtime (`runtime.c`,
  `trace_answer`) read and write the same text.
- The runtime reaches files through the platform layer already (the trace:
  `elya_platform_file_open/read/close`, `trace_create/append`); a board's
  defaults have no files.
- `elya run <file>` refuses extra arguments; a native binary's `main` is
  generated as `int main(void)` and ignores its arguments.
- `elya/console` and `elya/clock` show the capability pattern: an effect in a
  standard module, handlers that give a dependency less than the host, and the
  host's handler in `elya/host`.

## 1. Decisions

**D1. Six builtins, each performing `IO`:**

| builtin | type | what |
|---|---|---|
| `io.read_file(path)` | `fn(String) -> String` | the whole file |
| `io.file_exists(path)` | `fn(String) -> Bool` | whether `path` is a regular file |
| `io.write_file(path, text)` | `fn(String, String) -> Unit` | the file made `text`, created or replaced |
| `io.read_stdin()` | `fn() -> String` | standard input to its end |
| `io.arg_count()` | `fn() -> Int` | the program's arguments, not counting itself |
| `io.arg(i)` | `fn(Int) -> String` | argument `i`, from 0 |

**D2. Failures by name**, status 1, as every runtime trap: `elya: cannot read
`<path>`: <reason>`, `elya: cannot write `<path>`: <reason>`, `elya: `<path>`
is not UTF-8` (an Elya `String` is UTF-8 on both back ends), `elya: standard
input is not UTF-8`, `elya: no argument <i> (the program has <n>)`. A
missing file is a failure, not an answer: a builtin cannot answer a program's
own `Option` type, and `io.file_exists` asks first. The reason is the
operating system's words, which differ by platform; tests compare up to the
colon.

**D3. Recorded and replayed** like `time.*`: `io.read_file("a.txt") =
"..."`, `io.file_exists("a.txt") = True`, `io.write_file("b.txt", "...") =
Unit`, `io.read_stdin() = "..."`, `io.arg_count() = 2`, `io.arg(0) = "x"`.
A replay answers from the trace and touches nothing: it reads no file and
writes none. A failure ends the run, so the trace holds what came before it
(a replay of that run diverges where the failure was; recording failures as
answers is not done here).

**D4. Arguments.** `elya run <file.elya> [-- <args>...]` gives the program what
follows `--`; a native binary gives it its own arguments (the generated `main`
takes `argc` and `argv`). A board has none: `io.arg_count()` is 0.

**D5. A board has no files and no input.** The platform layer gains three
hooks with weak defaults: `elya_platform_read_file` (on a board, the failure
`no files on this machine`), `elya_platform_write_file` (the same),
`elya_platform_file_exists` (false), and standard input reads as empty.
Hosted, they are the C library's (`fopen`, `fread`, `fwrite`, `stat`).

**D6. Capabilities.** `elya/files` (effect `Files`: `read_file`,
`write_file`, `file_exists`; handlers in memory, read-only, under a directory,
logged; ChatGPT's first draft, checked here) and `elya/host`'s `files(body)`
over the builtins. Arguments and standard input stay builtins until a program
needs to give a dependency less of them.

**D7. Not here:** running a program (`io.run`), needed by part 5's back end
and not before; reading by line (`io.read_stdin` reads all; a compiler reads
whole files); directories.

## 2. Tests (planned)

Both back ends, the same bytes: a file written, checked, read back; a missing
file and a non-UTF-8 file failing by name; standard input read; arguments
through `elya run -- ...` and a native binary's command line; record and
replay of each, a replay touching no file (the file removed before it); the
`elya/files` handlers; a board answering `no files on this machine` and no
arguments.

## 3. Predictions

Written with the plan.
