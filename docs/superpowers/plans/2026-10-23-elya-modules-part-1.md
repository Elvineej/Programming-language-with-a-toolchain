# Plan: modules, part 1

Spec: `docs/superpowers/specs/2026-10-23-elya-modules-part-1-design.md`.
Baseline: 1144 passed, 110 suites (0f09f35).

## Task 1: red tests

`tests/modules.rs` (counted twice), through a test reader over a map from module
path to text (`elya::modules::load` with a closure), thirteen tests:

1. `qualified_calls_run`: m1 42, m2 (`as u`) 42, `let g = util.twice` then `g(5)`.
2. `imports_resolve_by_path_once`: `a/b` nested; the root and `lib` both import
   `util` (loaded once: its one effect declared once, no E0209); `lib` calls `util`.
3. `a_missing_module_is_e0600`: `nothing/here`, `elya/net`; `elya/io` accepted.
4. `an_import_cycle_is_e0601`: `a` imports `b` imports `a`; `a` importing the
   root's own path.
5. `private_and_unknown_functions_are_e0602_and_e0603`.
6. `functions_do_not_cross_files_unqualified`: m4 E0200; root and `util` each
   declare `helper` (1 and 2) and `util.f` calls its own: `helper() * 10 +
   util.f()` is 12; a local named `helper` inside `util` shadows the rename.
7. `an_import_name_is_unique_e0604`: `import x/io`, and `x/util` with `y/util`.
8. `types_and_effects_cross_an_import`: `util` declares `type Opt(a)`, `effect
   Log`, and `pub fn find(...) / {Log} -> Opt(Int)`; the root matches `Yes(n)`,
   handles `Log.say` and prints `op.show` of a `with any` operation.
9. `a_name_from_a_module_not_imported_is_e0605`: root imports `a`, `a` imports `b`;
   the root uses `b`'s constructor, type, effect in a row, and operation.
10. `a_type_declared_in_two_files_is_e0209`.
11. `a_diagnostic_names_its_file`: a type error inside `util` renders
    `util.elya:1:` and not the root's name.
12. `an_allow_list_is_e0606`: `import util / {Log}`.
13. `a_single_file_program_is_unchanged`: for every example, the module from
    `load` with no reader equals `parse_module`'s (`ast::pretty`), and the
    diagnostics are the same.

Predicted red: the whole file fails to compile (`elya::modules` does not exist).
To get per-test red values first, the module and its `load` signature are added
returning the root parsed alone (no imports read), then: 1, 2, 6, 8 fail with
E0201 "unknown builtin"; 3, 4, 7, 9, 12 check clean where an error is expected;
5 E0201 instead of E0602; 10 checks clean; 11 the rendered file is the root;
13 passes (it is the stub's behaviour, kept).

`crates/cli/tests/run_cli.rs` (counted twice): `elya_run_reads_imported_files` (a
two-file program on disk: stdout, and `elya check` on a broken module names its
file). `crates/cli/tests/build_cli.rs` (once): `elya_build_reads_imported_files`.
`crates/codegen/tests/native_modules.rs` (once): `modules_agree` (programs of 1,
2, 6 and 8 natively and in the evaluator).

Prediction: **1174 passed, 113 suites** (modules.rs +26, run_cli +2, build_cli +1,
native_modules +1; two new root suites, one native).

## Task 2: spans and sources

`span.rs`: `SourceMap` holds files (name, base, text); `add_file`, `file_of`.
`diag.rs`: `render` reports each label in its own file. `lex.rs`/`parse.rs`:
`parse_module_at(text, base)`, every span shifted by `base`.

## Task 3: the loader and the merge (`src/modules.rs`)

Parse the root, then each import depth-first with cycle detection; E0600-E0606;
rename functions of imported modules and rewrite qualified references (a
scope-aware walk); the visibility check of D3 (a walk over type annotations,
rows, clauses, patterns and constructor and operation uses).

## Task 4: entry points

`lib.rs`: `*_with` entry points taking a reader; the old ones call them with a
reader that finds nothing. `main.rs`: a file reader relative to the program, for
every command; `build` through the same front end.

## Task 5: gate, controls, review, sweep

`sh scripts/check.sh` at 1174 / 113; the four controls of spec §2; an
independent review; the sweep unchanged (no corpus program imports a user module).
