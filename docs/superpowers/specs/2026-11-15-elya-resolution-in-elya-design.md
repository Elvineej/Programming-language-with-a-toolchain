# Modules and name resolution in Elya (self-hosting part 4c)

2026-11-15. Status: the Rust side (D1) built; the Elya program (D2-D5) next. Part of R3 item 4 of
`2026-11-14-elya-self-hosting-design.md`, after the parser in Elya (4a, 4b).

## What is reproduced

The two passes the front end runs after parsing and before the checker
(`elya::front_end_with`):

- `modules::load` (src/modules.rs, 1,087 lines): reads and parses the root
  and every module it imports (each file's parse diagnostics are its, as part
  4b prints them), checks the imports (E0600 to E0605, E0607, E0608, and
  E0200 and E0205 for names across modules), and merges the files into one module. An imported
  module `a/b`'s function `f` is renamed `a.b.f`, its unshadowed references
  with it, and `b.f` in an importer becomes a reference to `a.b.f` (modules
  part 1, D5).
- `resolve::check` (src/resolve.rs, 822 lines): every reference resolves to a
  parameter, a local, a top-level function or a builtin (E0200 to E0211, and
  E0432), with the modules review's R3 filter: a diagnostic on a span a module
  diagnostic already refused is dropped.

Not here: `modules::check_grants` (E0606) runs after inference, so it belongs
to the checker's part.

## Decisions

**D1. The print form.** `elya resolve --dump <file>` prints the merged module's
tree in `elya::ast_dump::dump`'s form, then the diagnostics of the two passes
in `front_end_with`'s order in `dump_diagnostics`'s form. Exit 1 when there
is an error. The tree's spans are the merged program's offsets: each file at
its `SourceMap` base, each base one past the previous file's end. So the text
`dump` reads a `Float`'s digits from is the files' texts joined with one
`"\n"`, in the order the files were first reached (the root first, then
its imports depth first, each once). That is not the order their
declarations appear in the tree: each module is merged after the ones it
imports, the root last (found by the review). Nothing new is printed: the tree form is the one the
parser in Elya already matches. Built: `elya::resolved` (the two passes,
which `front_end_with` now calls, so the order is the front end's by
construction), `elya::merged_text`, and the command; pinned by
`resolve_dump_prints_the_merged_program` in crates/cli/tests/run_cli.rs.

**D2. The Elya program.** `selfhost/resolve.elya` imports the parser in Elya.
It takes the root file as `io.arg(0)` and the standard modules' directory as
`io.arg(1)` (src/std in the repository: Rust embeds those sources, an Elya
program reads them), and resolves a user import against the root's directory
as `load`'s reader does. It prints exactly what D1 prints. It uses `elya/map`
for the name tables, and buffers for the output.

**D3. The builtin names** (`resolve::builtins`, the `*_BUILTINS` tables, the
builtin modules list) are copied as tables in the Elya program. A test
compares each table with its Rust twin, so a builtin added in Rust without its
Elya twin fails (R4).

**D4. Tests.** tests/selfhost_resolve.rs runs the Elya program in the evaluator
and crates/codegen/tests/selfhost_resolve.rs runs it natively, over:

- every program the parser's corpus holds (tests/support/selfhost_parse.rs);
- every multi-file program in the Rust tests, with its files written to a
  temporary directory (the modules tests' readers);
- every program in the tests with a module or resolution error;
- a focused set of at least 60, one or more for each code and each renaming
  rule: shadowing, a qualified call, an alias, an import of an import, a std
  module, and a cycle.

Each must equal `elya resolve --dump` byte for byte. Negative controls: K1, a
shadowed reference renamed anyway; K2, the R3 filter dropped; K3, the
diagnostics of the two passes printed in the other order.

**D5. Measured.** The native time of resolving the largest program against
Rust, written in section 3 here, as the parser's spec does.

## 3. Measured after

(To be filled in.)
