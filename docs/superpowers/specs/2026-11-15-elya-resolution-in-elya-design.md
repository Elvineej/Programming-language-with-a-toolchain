# Modules and name resolution in Elya (self-hosting part 4c)

2026-11-15. Status: built (D1-D5); 2026-11-27 the Elya program and its tests. Part of R3 item 4 of
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

`selfhost/resolve.elya` (1,169 lines) imports the parser in Elya, which gains
two public functions for it (`parse_full`, the tree and every diagnostic;
`dump_tree`). Each imported file is parsed at offset 0 and its tree and
diagnostics shifted to its base; the merged text is the files' texts joined
with `"\n"` in the order reached. The tables of `decls` and of `resolve::walk`
are one `elya/map` each, keyed by kind and name (and unit); a name's being a
function, an operation or a constructor is one entry, a mask. No compiler
change was needed, and nothing in Rust was found refused or crashing.

Compared byte for byte, in the evaluator (tests/selfhost_resolve.rs) and
natively (crates/codegen/tests/selfhost_resolve.rs), 4,108 programs: the
parser's whole corpus (3,931, each a root alone, its imports resolved against
an otherwise empty directory and src/std), 106 multi-file programs found in
the Rust tests (each test's roots over each set of modules it gives:
tests/modules.rs 65, native_modules.rs 10, name_clashes.rs 8, std_async.rs 5,
std_capabilities.rs 5, run_cli.rs 5, files_and_arguments.rs 4,
string_bytes.rs 2, build_cli.rs 1, diagnostics_edges.rs 1) and 71 focused
programs. 1,045 have a module or resolution error; by code (programs): E0200
584, E0201 35, E0202 10, E0203 93, E0204 6, E0205 6, E0206 13, E0207 6, E0208
14, E0209 21, E0210 5, E0211 7, E0432 41, E0600 301, E0601 19, E0602 3, E0603
11, E0604 21, E0605 4, E0607 54, E0608 8; the focused set has each. The
builtin tables (D3) are compared with their Rust twins by
`the_builtin_tables_are_their_rust_twins` (the private `BUILTIN_MODULES` and
`STD_SOURCES` read off E0600's help and E0604).

Controls, each run in both tests and reverted (`cmp`): K1 (a shadowed
reference renamed) fails on examples/06_structured.elya natively, on a
token-deleted examples/08_timers.elya in the evaluator (`step`, a parameter
in `elya/async`); K2 (no R3 filter) and K3 (the passes' diagnostics in the
other order) on selfhost/check.elya (its token-deleted variant in the
evaluator).

The largest program is selfhost/check.elya in place, with the parser and
lexer it imports (168,098 bytes of root, a dump of 1,071,082 bytes):

| Measurement | Median | Runs |
| --- | ---: | ---: |
| Elya native (O2): process + read + lex + parse + load + resolve + dump | 230 ms | 7 |
| Rust (release): read + lex + parse + load + resolve | 16.0 ms | 101 |
| Rust (release): the same and the dump | 20.0 ms | 101 |
| Rust (test profile): the same and the dump | 99.9 ms | 101 |

Command: `RUST_TEST_THREADS=1 taskset -c 1 cargo test --release -p
elya-codegen --test selfhost_resolve -- --nocapture` (the test profile's row
from the same test without `--release`). Natively the Elya program is about
11 times the release Rust's time, the parser's ratio. The evaluator test
takes about 2.5 times the parser's in the gate (235 s against 110 s in
configuration B).

Found in Rust, left as it is (smallest example): a standard path that is not
a module is E0600 but its name is not marked failed, so R3 does not refuse a
qualified use of it: `import elya/nope\npub fn main() -> Int { nope.f() }`
prints E0600 and then E0201 ``unknown builtin `nope.f` `` (a user module's
`import nope` prints E0600 alone).
