# Modules, part 1: a program in more than one file

**Status:** in progress (2026-10-23, branch `auto/elya`, a scheduled run). HANDOFF
step 1. The design choices are Claude's under the maintainer's delegation (rule 2),
recorded here. ROADMAP priority 2 (capabilities per dependency) needs this first;
the effect allow-list at the import boundary is part 2.

## 0. Measured first (at 0f09f35, gate 1144 / 110)

What the front end does with imports today. `util.elya` holds
`pub fn twice(x: Int) -> Int { x * 2 }` next to each program:

| program | `elya run --value` |
|---|---|
| m1 `import util` then `util.twice(21)` | E0201 "unknown builtin `util.twice`" |
| m2 `import util as u` then `u.twice(21)` | E0100 at `as` ("expected `import`, `fn`, ..."), then E0201 |
| m3 `import nothing/here`, main `1` | 1 (the import is ignored) |
| m4 `import util` then `twice(21)` | E0200 "unresolved name `twice`" |

How names work after the parser (read from the code):

- `ast::Module` is one file: `imports` (path segments, `alias` always `None`) and
  `decls`. Nothing after the parser reads `imports` or `FnDecl::is_pub`.
- Every name space is flat and keyed by the bare name: functions, types,
  constructors, effects and operations (resolve's E0209 rejects a second
  declaration of any of them; operation names are unique because a perform is an
  unqualified call, `roll(6)`).
- `m.f` is `Expr::Qualified`, and only the builtin modules exist: `io`, `int`,
  `time`, `op`. A builtin cannot be used as a value (E0208).
- Spans are byte offsets into ONE text; the typed table, the evaluator's errors and
  Core all key on them; `SourceMap` and `render` know one file.
- The data-literal syntax (`op.show`, `op.encode`, `op.decode`) and the record /
  replay trace name constructors and `Effect.op` unqualified.
- Native: Core functions become LLVM symbols `elya.<name>`.
- The corpus (836 programs) has one import, `import elya/io` in
  `examples/01_hello.elya`, and no other use of `import` or a user `pub` beyond
  `pub fn main`.

## 1. Decisions

- **D1. A module is a file; `import a/b` is the file `a/b.elya` next to the
  program.** Paths resolve against the directory of the root program (the file
  given to `elya`), never against the importing file, so one path names one module
  everywhere. Each module is loaded once, however many files import it. A missing
  file is **E0600** "no module `a/b`" on the import; an import cycle is **E0601**
  "an import cycle: `a` -> `b` -> `a`" on the import that closes it.
  - `elya/...` is the standard library's prefix. `import elya/io`, `elya/int`,
    `elya/time` and `elya/op` name the builtin modules: accepted, and they change
    nothing (builtins stay in scope without an import, as today). Any other
    `elya/x` is E0600. Whether builtins need their import is a part-2 question
    (it is a capability question).
  - *Rejected:* paths relative to the importing file. Two files would name one
    module differently, and the module's identity (its mangled names, D5) would
    depend on who imported it first.
- **D2. Functions cross files only qualified.** `import a/b` binds the name `b`;
  `import a/b as c` binds `c`. In the importing file, `b.f(...)` calls the
  module's `pub fn f`; `b.f` is also a value (a module function is an ordinary
  function, unlike a builtin). A module's functions never enter another file
  unqualified (m4 stays E0200).
  - `b.g` for a function without `pub` is **E0602** "`g` is private to module
    `a/b`"; for a name the module does not declare, **E0603** "module `a/b` has no
    function `g`".
  - An import name equal to a builtin module (`io`, `int`, `time`, `op`) or to
    another import's name in the same file is **E0604** on the second import:
    `m.f` must mean one thing.
  - *Rejected:* unqualified imports (`import util` bringing `twice` in). Two
    dependencies could then clash on any helper, and a reader cannot tell where a
    call goes; the qualified form is already the builtins' form.
- **D3. Types, constructors, effects and operations are exported, unique across the
  program, and visible where their module is imported.** A module's type and
  effect declarations are usable in the module itself and in the files that import
  it directly; anywhere else a use is **E0605** "`T` is declared in module `a/b`,
  which this file does not import". Two declarations of one such name anywhere in
  the program are E0209, as in one file.
  - *Why unqualified:* `op.show`, `op.encode`/`op.decode`, the trace format and the
    data-literal syntax write `Cons(1, Nil)` and `World.roll(6)`; a program-unique
    name keeps every one of them self-describing with no format change, and a
    perform is already an unqualified call.
  - *Why visible only through an import:* part 2 puts the effect allow-list on the
    import, so the import must be the gate for an effect's use; a file cannot reach
    a dependency's dependency's effects without importing it.
  - *Rejected:* qualified type and effect names (`json.Value`, `json.Log.say`):
    qualified names in types, patterns, rows, clauses and the literal syntax, for a
    clash no program has yet. Recorded as open, for when two libraries collide.
  - *Rejected:* `pub type` / `pub effect`. A `pub fn` whose signature mentions a
    private type would need a further rule; every type-level name is part of a
    module's interface in part 1.
- **D4. `pub` marks the functions a module exports.** Without it a function is
  private to its file. The root program's functions are not exported anywhere (no
  file imports the root: importing the root is a cycle, E0601).
- **D5. Core and codegen see one merged module.** The root's declarations are
  unchanged. In an imported module `a/b`, the function `f` is renamed `a.b.f` (a
  name no source can write, so it cannot clash), every reference to it in its own
  file that no local shadows is renamed with it, and every `b.f` in an importer
  becomes a reference to `a.b.f`. An imported module's `main` is an ordinary
  function; only the root's `main` runs.
  - Each file gets its own range of span offsets (one past the end of the files
    before it), and the source map holds every file, so a diagnostic in `util.elya`
    is reported in `util.elya` with its own lines.
  - A program with no user import is exactly the module the single-file front end
    builds today (tested), so nothing in the corpus changes.
  - *Rejected:* separate compilation (a Core module and an object per file). It is
    part of a package story, not of a two-file program; the merged module is the
    smallest step and keeps every later pass unchanged.
- **D6. The allow-list has its place now, and is refused.** `import a/b / {Net}`
  (the row syntax of a function signature) parses, and part 1 reports **E0606**
  "an import's effect list is not checked yet (modules part 2)". Refusing rather
  than ignoring keeps any program from relying on an unchecked list.
- **D7. The toolchain reads the files.** `elya check`, `run`, `run --value`,
  `run --record|--replay` and `build` load a program's imports from disk relative
  to the program. The library's text entry points (`check_source`,
  `run_source_outcome`, ...) keep their signatures and see no files: a user import
  there is E0600. New `*_with` entry points take a reader from module path to text.

## 2. Evidence (planned)

- `tests/modules.rs` (front end and evaluator, both configurations), through an
  in-memory reader: qualified calls with and without `as` and as values; nested
  paths, a module imported twice, a module importing another; E0600 (missing,
  `elya/x`), `elya/io` accepted; E0601; E0602 and E0603; names not crossing
  unqualified, and two files with a function of one name; E0604; types,
  constructors, effects and operations across an import (with `op.show`); E0605;
  E0209 across files; a diagnostic rendered in the right file; E0606; a single-file
  program unchanged.
- `crates/cli/tests/run_cli.rs`: `elya run` and `elya check` on files on disk.
- `crates/cli/tests/build_cli.rs` and `crates/codegen/tests/native_modules.rs`:
  multi-file programs compiled, run and compared with the evaluator.
- Negative controls (in `modules.rs`'s header): the rename skipping shadowing (a
  local named like a module function), visibility off (E0605 never), the `pub`
  check off, the span base at 0 for every file (diagnostics in the wrong file).
