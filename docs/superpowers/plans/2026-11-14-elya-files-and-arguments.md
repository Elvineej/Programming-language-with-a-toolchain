# Plan: files, standard input and arguments (self-hosting, part 1)

Spec: `docs/superpowers/specs/2026-11-14-elya-files-and-arguments-design.md` (D1-D7).
Base: `main` at 0d4be84.

## Steps

1. Red tests first, each failing for the reason it names (below).
2. The front end: the six names in `resolve::builtins` with their arities;
   their types in `types.rs`, the three host-builtin branches (`io.println`,
   `time.*`, `io.*`) made one table of signatures, each performing `IO`.
3. The evaluator: `eval::Host`; `host_builtin` answering every host builtin
   live, recorded or replayed, a replay's answer read at the builtin's own
   result type (`literal::read_string` for a `String`); both machines (CEK and
   the tree walker) through it. `lib.rs`: `run_source_outcome_host`,
   `record_source_host`.
4. The CLI: `elya run [--record <trace> | --value] <file.elya> [-- <args>...]`;
   a replay takes no arguments (they are in its trace).
5. Native: the runtime's six functions, record and replay over the trace's
   reader and writer (a line built whole in working memory, any length); the
   platform hooks of D5, hosted and weak; `main(argc, argv)` and
   `elya_args_init`; both emitters through one `io_builtin`; the boards' and
   the test board's start-up calling `main(0, 0)`.
6. `elya/files` (ChatGPT's draft: checked, its `true`/`false` made `True`/
   `False`), `elya/host`'s `files`.
7. Controls (each reverted, compared byte for byte, each failing what it names):
   K1 a replay that writes the file anyway, in both back ends (the
   replay-touches-nothing tests find the file); K2 the reason for a directory left
   to the operating system (Linux natively reads `Is a directory`, the
   evaluator `it is a directory`: the words test fails); K3 `arg_count`
   counting the program itself (the arguments tests see one more); K4 a
   recorded `String` answer not escaped (a file holding a newline breaks the
   trace's line and the replay fails).
8. Independent review; the gate.

## Tests (red first)

- `tests/files_and_arguments.rs` (new, 8): a file written, checked, read
  back; reading failing by name (missing, a directory, a NUL in the path, not
  UTF-8, no such argument, input not UTF-8); arguments and input from
  `eval::Host`; every answer recorded and replayed, the replay touching no
  file; a replay's answer checked against its type; the builtins typed and
  performing `IO` (a module granted `Console` alone cannot call them); the
  `elya/files` handlers; `elya/host`'s `files`.
- `crates/cli/tests/run_cli.rs` (+3): `elya run <file> -- <args>`; standard
  input reaching `elya run`; a recording keeping the arguments, a replay
  refusing them.
- `crates/codegen/tests/native_files.rs` (new, 5): the same programs natively
  writing what the evaluator writes, the files they make included (direct and
  CPS code); the failures in the evaluator's words; a binary's own arguments
  (one outside ASCII) and input; traces both ways; a native replay touching no
  file.
- `crates/codegen/tests/native_platform.rs` (+1): the test board has no files,
  no arguments, empty input.
- `crates/codegen/tests/native_boards.rs` (+1, configuration C): the same on
  both QEMU boards.

## Predictions

- Gate, configurations A and B: 1536 + 2 x (8 + 3) + 5 + 1 = **1564 passed,
  169 suites** (166 + 2 + 1). Configuration C: 7 + 1 = **8 passed, 2
  suites**.
- No existing expectation changes. The usage line of `elya run` gains
  `[-- <args>...]`; no test reads it whole.
