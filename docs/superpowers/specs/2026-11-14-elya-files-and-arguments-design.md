# Self-hosting, part 1: files, standard input and arguments

**Status:** built (2026-11-14, on `main`, an interactive session; the plan
`docs/superpowers/plans/2026-11-14-elya-files-and-arguments.md`). HANDOFF
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
is not UTF-8` (an Elya `String` is UTF-8 on both back ends), `elya: cannot
read standard input: <reason>`, `elya: standard input is not UTF-8`, `elya: no
argument <i> (the program has <n>)`, `elya: argument <i> is not UTF-8`. A
missing file is a failure, not an answer: a builtin cannot answer a program's
own `Option` type, and `io.file_exists` asks first. The reason is the same
words on both back ends and every platform where it can be: `it is a
directory` (asked of the file system after the failure, so a directory reads
the same on Linux, where opening it succeeds, and on Windows, where it does
not), `no such file or directory`, `permission denied`, `a path cannot hold a
NUL byte` (checked first: the C library would cut the path there); any other
failure is the operating system's words (the C library's `strerror`, and
Rust's `io::Error` without its ` (os error N)`, the same text on Linux).

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

**D5. The platform layer gains six hooks, each with a weak default, so a
board has no files and no input.** A program's paths are UTF-8, apart from
the trace's (which keep the C library's `fopen` of the environment's bytes):
`elya_platform_path_open` (read with the existing `file_read`/`file_close`),
`elya_platform_path_exists`, `elya_platform_path_write` (the whole file at
once), `elya_platform_path_error` (D2's reason for the last failure on a
path), `elya_platform_stdin_read`, `elya_platform_args` (the program's
arguments as UTF-8). Hosted: the C library's `fopen`, `fread`, `fwrite`,
`stat`; on Windows the wide functions over the UTF-8 path (`_wfopen`,
`_wstat64`) and the arguments from `CommandLineToArgvW`, so a path or an
argument outside the ANSI code page means what it does to Rust's standard
library, and standard input in binary mode, as Rust reads it. The defaults: no
file opens, none exists, none is written, the reason is `this platform has no
files` (the words a board's trace failure already has), standard input is
empty. A board's start-up calls `main(0, 0)`: no arguments.

**D6. Capabilities.** `elya/files` (effect `Files`: `read_file`,
`write_file`, `file_exists`; handlers in memory, read-only, under a directory;
ChatGPT's first draft, checked here: its read of a missing path in memory
answered `""`, here it stops the body as the host stops a run,
`FilesMissing(path, files)`; its `logged` performed `Console`, which made every
importer grant `Console`, so it is left out) and `elya/host`'s `files(body)`
over the builtins. In the evaluator the live host is `eval::Host` (the
arguments, and standard input when a caller gives it rather than the
process's own), so a test gives a program its world without a process. Arguments and standard input stay builtins until a program
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

In the plan, `docs/superpowers/plans/2026-11-14-elya-files-and-arguments.md`.
