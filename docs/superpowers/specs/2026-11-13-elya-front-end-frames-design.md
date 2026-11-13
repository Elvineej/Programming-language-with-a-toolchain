# The front end's frames

**Status:** done (2026-11-13, on `main`, an interactive session). Found checking
continuous integration after the platform layer: the Windows gate had been red
since cf3f028 (the affine guarantee), on four commits, and no one had looked.

## 0. Measured first

The Windows gate failed `infer_complexity::nested_performs_cost_in_proportion_to_their_types`
with `STATUS_STACK_OVERFLOW`: 60 nested `op(fn() { .. })`, checked on a test
thread's 2 MiB in a debug build. On Linux the same test needed between 1.5 and 2
MiB. Frame by frame (gdb, the stack pointer of each frame):

| function | debug frame | per nesting level |
|---|---|---|
| `types::Infer::infer_call` | 16.5 KiB | 1 |
| `types::Infer::infer_expr_inner` | 7.4 KiB | 2 |
| the parser's `atom` / `block_inner` | 3.9 KiB each | 2-4 / 1-2 |
| `resolve::Cx::check_expr` | 4.9 KiB | 3-4 |

A debug build gives every local of every arm its own slot: `infer_call` typed
seven builtins, constructors, performs and ordinary calls in one body, and the
affine guarantee's additions (`hold_point`, row bookkeeping) took it past 2 MiB at
60 levels.

## 1. Decisions

**D1. A recursive function holds only what every arm needs.** Each of the four
dispatches what it reads and types each kind in a helper of its own, marked
`#[inline(never)]` where the helper does not recurse (an optimised build would
otherwise fold it back): `infer_call` infers the arguments (a loop: an iterator
chain was ten frames more) and calls `type_builtin_call`, `type_ctor_call`,
`type_perform`, `type_ordinary_call`; `infer_expr_inner`'s `resume`, empty
`match`, arm binding and lambda entry and exit are helpers; the parser's atoms are
read by `leaf_atom`, `minus_atom`, `unary_atom`, `paren_atom`, `resume_atom`, a
block's recovery and E0103 by helpers; the resolver's names, builtins, handlers
and E0204/E0210 by helpers. The order in which things are inferred, fresh
variables made and diagnostics reported is unchanged.

**D2. A test of the stack, not of a platform.** `tests/checker_stack.rs`: three
nests 60 deep (performs in lambdas; calls, matches, `if`s and lambdas; handlers
and resumes) parse and check on a thread of 1.25 MiB in a debug build. All three
overflowed before (the first in the checker, the others in the parser and the
resolver); the first needs 832 KiB after.

## 2. Measured after

`nested_performs` at 60 levels: 1.5-2 MiB before, under 832 KiB after (Linux,
debug). `infer_call`'s frame 1.0 KiB. No diagnostic, type or test expectation
changed.

Gate (Linux): 1516 passed, 161 suites (the three new tests, counted twice: the
gate runs the root crate's suites twice).

## 3. Not done

The other passes (the affine pass, exhaustiveness, lowering, the evaluator) were
not measured frame by frame; the CLI runs the compiler on a 1 GiB thread, so this
matters to the library's other callers (tests, an editor). A `stacker`-style guard
on the recursive entry points would bound it for every pass at once.
