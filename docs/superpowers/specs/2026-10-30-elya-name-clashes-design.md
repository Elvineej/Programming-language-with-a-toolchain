# A name clash is reported once, where it was made

**Status:** in progress (2026-10-30, branch `auto/elya`, an interactive session). HANDOFF
step 1 (name clashes across files). The design choices are Claude's under the
maintainer's delegation (rule 2), recorded here.

## 0. Measured first (at the long-blocks slice)

`elya check` of a root program against the standard modules and two user modules
(`lib/a` imports `lib/b`; `lib/b` declares `type Counted { Counted(Int) }` and
`pub fn count`):

| root program | diagnostics | labelled in |
|---|---|---|
| `import elya/console` + `fn console_line(..)` (an operation's name) | E0200 x3, E0205 | the three E0200s in `elya/console.elya` ("unresolved name `console_line`"), E0205 in root |
| the same through `import elya/host` | the same | the same |
| `import elya/console` + `effect Console { fn speak() -> Unit }` | E0209, E0207 x5 | E0209 in both files; the five E0207s ("this handler does not cover every operation of `Console`") in `elya/console.elya` |
| `import elya/console` + `type ConsoleCounted { CC }` | E0209 | both files |
| `import elya/replay` + `type T { TraceEnd }` | E0209 | both files |
| `import lib/a` + its own `type Counted`, used | E0605 x2, E0209 x2 | E0605s in root ("`Counted` is declared in module `lib/b`, which this file does not import", help "add `import lib/b`"), E0209s in both |
| the same with `import lib/b` too | E0209 x2 | both |
| `import elya/console` + `fn silenced(..)` (a pub function's name) | E0606 | root: the import's missing `/ {Console}` grant, not the name (found writing the tests; with the grant, none) |
| `import elya/replay` + `fn lines(..)` (a private function's name) | none | |
| `import lib/a` + its own `fn count` | none | |

- Functions are per file (a module's are renamed `path.f`, modules part 1 D5);
  types, constructors, effects and operations are one name space for the whole
  program (E0209's help says so).
- The modules are merged into one module before `resolve` and the checker run, so
  a clash changes what a module's own code means: its `console_line(..)` no longer
  reaches its operation, its handlers no longer cover the root's `Console`.
- E0605's owner of a name is the first file that declares it, and modules come
  before the root: the root's own `Counted` is "declared in `lib/b`".
- The causes, read in the code:
  - the three E0200s: `modules::Rewriter::expr` reports a module's reference to a
    name the ROOT declares as a function ("the root's functions keep their names in
    the merged module; they are not this file's") before asking whether the name is
    an operation the module sees. `console_line` is both;
  - the five E0207s: `resolve` collects an effect's operations by name, so the two
    `Console`s pool theirs, and the module's handlers no longer cover "`Console`";
  - the E0605s: `Decls::owner` keeps the first declaring file.

## 1. Decisions

- **D1. A module's reference to an operation means the operation.** The module
  rewriter asks first whether the name is an operation some file of the program
  declares, then whether it is the root's function. So the root's `fn console_line`
  is the root's one error (E0205), and `elya/console`'s own `console_line(..)` stays
  its operation. If the module cannot see that operation (no file it imports
  declares it), it gets E0605, as it would with the root's function renamed. Only a
  lowercase name can be both: constructors are capitalised, functions and
  operations are not.
- **D2. An effect declared twice is E0209 and nothing else.** No handler of an
  effect with two declarations is checked for coverage (E0207): which declaration's
  operations it should cover is the question E0209 asks. The same rule `resolve`
  already has for a duplicated operation (E0202).
- **D3. E0605 is never about a name the file can see.** A type, constructor,
  effect or operation the file declares, or a file it imports declares, is visible
  there, whichever file declared it first. The clash itself is E0209 (E0202 for an
  operation).
- **D4. Types, constructors, effects and operations stay one name space.**
  - *Rejected:* making them per file, as functions are (modules part 1 D5). Their
    names are data: a trace and `op.show` write `Console.console_line(..)`,
    `op.encode` and `--value` write constructors by name. A per-file name would
    change those texts or need a second name to print.
  - E0209 keeps its two labels, one in each file, and its help ("unique across the
    program, imported files included"); it is reported once, in the file whose
    declaration came second (the root is always last).

## 2. Evidence

- `tests/name_clashes.rs`: each measured row, by code and by the file each
  diagnostic is labelled in, standard and user modules, direct and transitive; each
  kind of name for D3; and the E0605 that stays (a name used from a module the file
  does not import).

Measured (2026-10-30):
- Red first as planned: three tests red with the measured cascades, the fourth
  (what stays) green.
- Found writing the tests: the measurement's E0606 for `fn silenced` was the
  import's missing `/ {Console}` grant, not the name (§0's table says so now); every
  fixture grants what its module performs.
- No existing expectation changed (the root crate: 499 tests, 59 suites).
- Controls K1-K3 (the test's header): each made its own test fail with the
  measured cascade.
- **Review (an independent agent), each finding fixed test-first:**
  - F1, medium: D3 counted only the file's own declarations. A file importing
    `lib/y`, which declares `T`, while `lib/x` (reached through another import)
    declared it first, still got E0605 "add `import lib/x`". Now a name any
    imported file declares is visible too. Test
    `a_name_an_imported_file_declares_is_visible`; control K4.
  - F2, medium: the tests reached D3 only for constructors. Test
    `every_kind_of_own_name_is_visible` (a type, an effect, an operation); under
    K3 each kind gets its E0605 back.
  - F3, F4, low: D1 described "visible to the file" where the code asks the
    program; and its constructor half cannot happen. D1 above says what the code
    does.
  - F5, low: two measured rows were untested (a standard module's constructor
    clash; the own type with `lib/b` imported directly), and the E0605 that stays
    was checked by code only. All three are checked now.
