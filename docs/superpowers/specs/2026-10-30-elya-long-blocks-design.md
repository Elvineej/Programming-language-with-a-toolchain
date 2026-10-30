# A block's length is not a stack bound

**Status:** implemented (2026-10-30, branch `auto/elya`, an interactive session).
HANDOFF step 1 (Core passes iterative over a block's statements). The design choices
are Claude's under the maintainer's delegation (rule 2), recorded here.

## 0. Measured first (debug build, at fa24c18)

A `main` of N statements `let a{i} = {i} + 1`:

| N | `elya check` | `elya run` | `elya build` |
|---|---|---|---|
| 10 000 | 0.26 s | 0.49 s | 2.2 s |
| 50 000 | 1.7 s | 1.8 s | 11.9 s |
| 100 000 | 3.9 s | 3.7 s | **stack overflow** on the CLI's 1 GiB thread (8.4 s in) |

- `gdb` at the overflow: `mono::Mono::rewrite_let` and `rewrite`, two frames per
  statement, 134 763 frames deep. In a debug build a frame of these passes is a few
  kilobytes, so a library caller on an 8 MiB thread is bounded at about a thousand
  statements.
- On a thread of its own, through the root crate's API:
  - the parser, resolver, checker and `core::lower_module` take 300 000 statements
    on 8 MiB (15 s) and 100 000 on 1 MiB. They already loop over a block.
  - Dropping the lowered module overflows: Core's derived `Drop` recurses down the
    `Let` spine. On 8 MiB it fails at 60 000 statements.
- In the codegen crate, about 40 traversals in 11 files match a `CoreKind::Let`, and
  every one of them recurses into the `let`'s body: the pipeline `anf`, `mono`,
  `specialize`, `any`, `instantiate_types`, `patterns`, `spill`; the collectors in
  `closure`, `cps`, `reflect`; both emitters.

## 1. Decisions

- **D1. Every pass walks a `let` spine with a loop.** A pass's recursion depth is then
  the program's nesting depth (an argument, a branch, an arm, a lambda's body, a
  handler), never a block's length.
  - The shape: walk down the spine collecting each binding's result, process the
    body that ends it, then rebuild from the last binding out. A scoped pass pushes
    each binding's scope as it goes and pops them all after the body.
  - Read-only visitors with no scope become an explicit work stack over the whole
    tree.
  - Per-binding special cases stay per binding (`mono`'s generic `let`s,
    `specialize`'s upcast bindings, `anf`'s aliases): the loop calls the same code
    the recursive arm did.
  - *Rejected:* a flatter Core block node, `Block([(x, v)], body)`. Every pass would
    change shape, and the passes that insert bindings (`anf`, `specialize`, `mono`'s
    clones) build `Let`s one at a time; two forms of the same thing would need a
    normalization of their own.
  - *Rejected:* growing the stack on demand (`stacker`, as rustc does). Its base,
    `psm`, needs Rust 1.88 in its current release, past the MSRV (1.75); an older
    one pinned is a dependency on a C and assembly build that the Windows gate has
    never run.
  - *Rejected:* a bigger thread. That moves the bound, as the 1 GiB CLI thread
    already did.
- **D2. Core's `Drop` is iterative.** `CoreExpr` drops its children through a heap
  stack, taking only those whose last reference it holds, as the evaluator's
  `CtorArgs` does (`src/eval.rs`).
- **D3. Nesting depth stays bounded by the stack.** A call nested N deep, or a block
  inside a block N deep, recurses in the parser and every pass. That is a bound on
  how a program is written, not on how long it is, and the parser already refuses
  one past 1024 levels by name (§2). Recorded as measured, not changed.
- **D4. The proof is a long program built and run on a small thread.** On 8 MiB
  (`main`'s size on Linux), natively: a 20 000-statement `main` with a dependency
  chain through every binding, and blocks of a few thousand statements in each place
  a block can be — a lambda's body, a match arm, a handle's body and clause, an `if`'s
  branch, and a function that performs, so the CPS emitter sees one. In the root
  crate: a 100 000-statement module lowered and dropped on 1 MiB, and run by the
  evaluator there.

## 2. Evidence

- `tests/long_blocks.rs` (root crate): D2 and the evaluator.
- `crates/codegen/tests/native_long_blocks.rs`: D4's programs, each against the
  evaluator's value.
- The nesting bound, measured after the change (D3).

Measured (2026-10-30, debug build):
- Red first as planned: the root test overflowed dropping the module; both native
  tests in `anf`, the first pass.
- Found by the red test, not the measurement: the evaluator's environment is a
  chain of `Scope`s, one per `let`, and its derived `Drop` recursed too (the probe
  of §0 had run the evaluator on 20 000 statements only). Its `Drop` is a loop now,
  the same rule as D2.
- Converted (the long `main` reached them in this order under `gdb`; the places
  test of the review's F2 reaches the rest): `anf` (`expr`, `fresh_binders`,
  `rename`, `binds`), `mono` (`map_expr`, `rewrite`, `visit`, the
  instance pass's `expr`), `specialize` (`subst_expr`, `rewrite`, `walk`), `any`
  (`expr`, `ops`, `map_vars_named`), `patterns` (`expr`, `needs`), `spill`,
  `closure` (`fv_walk`, `collect_in`), `cps` (`performed_ops`, `resume_owners`,
  `region_effects`, `contains_effect`, `walk`), `cps_emit` (`index_nodes`, `check`,
  `check_local_conventions`, `expr`, `tail`, `clause_tail`), the direct emitter
  (`lower_expr`, `lower_tail`, through one `lower_block`), and `reflect`'s
  `targets_in`. Each keeps the order of its effects: `mono`'s instance numbering,
  the lambda sites' pre-order, `cps::walk`'s path steps (`(let, 0, depth)` around a
  value, `(let, 1, depth + 1)` for the rest).
- Left recursive: `core::pretty_typed` (only snapshot tests print Core, on small
  programs), and walks over patterns and types.
- `elya build` after the change: 10 000 statements 1.75 s (was 2.2), 50 000 9.5 s
  (was 11.9), 100 000 18.6 s (was a stack overflow). The time is linear; it fell
  where `cps_emit::expr` asked `contains_effect` of every `let` on a spine, each
  asking it of the rest.
- D3: the parser already refuses nesting past `MAX_NESTING` (1024 levels) by name,
  E0102 "expression nested too deeply" (spec 2026-10-24 D4). At that bound,
  1 000 nested `if`s check and build on the CLI's thread. No change.
- Controls K1a, K1b (`tests/long_blocks.rs`), K2 to K6 (`native_long_blocks.rs`):
  each failed distinctly.

## 3. Review (an independent agent, 2026-10-30), each finding fixed test-first

- **F1, high: `spill` kept a `let`'s type unspilled.** The recursion's epilogue
  spilled every node's type (`self.ty(&e.ty)`); the loop copied the `let`'s. A `let`
  whose value is the `let` naming a computed callee then had a function of four
  where its body had the record form, and `check_local_conventions` refused
  `let g = mk(10)(2)  g(1, 2, 3, 4)` ("direct function used where an effectful one
  is expected"; the evaluator and the old build: 30). Each `let`'s type is spilled
  now, in the recursion's order (values, body, then types from the innermost out).
  Row `l1-a-function-of-four-from-a-computed-callee` in `native_codegen.rs`'s
  argument-record table. Every other converted arm was checked for the same
  omission: each maps its `let` types as its recursion did.
- **F2, medium: the tests did not prove every conversion.** At 3 000 statements on
  8 MiB a small-frame recursion passed (2.8 KB a statement), and no long block
  reached rule 4, `mono`'s or `specialize`'s clones, a `with any` or `return`
  clause, an effectful lambda, or a block of another type. The tests run on 512 KiB
  now (26 bytes a statement for the long `main`, 105 for the places) and the places
  test has each of those, at 5 000 statements. Controls K4 to K6 show a small-frame
  recursion, rule 4 and the `with any` clause are each reached.
- **F3, low: this spec claimed more than was verified** (rule 4's walkers "reached
  under `gdb`"; "each place a block can be" without three of them). Corrected above
  and by F2.
- **F4, low:** `lower_block`'s doc was inserted inside `lower_tail`'s. Moved.
- **F5, low:** `resume_owners` visited a handle's clauses before its body and its
  children right to left. Now the recursion's pre-order exactly.
- **F6, low:** on an error, `mono` and `specialize`'s blocks and `specialize::walk`
  left their scopes unbalanced (the recursion restored them). Restored on every path.

Found while sizing F2's test: `closure::fv_walk` looked a name up from the oldest
binding in scope, so a CPS site's saved set walked the rest of a block with a
lookup as long as the block (quadratic: the places test's build took 58 s). It looks
from the innermost binding now (the same answer: an `any`). What remains is the CPS
emitter's design: each site's continuation emits the rest of its region, so a
block's code grows with its sites times its length. The test performs once per
1 000 to 1 500 statements; the build takes about 30 s.
