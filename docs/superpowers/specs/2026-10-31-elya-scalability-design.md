# Compiler scalability: no step quadratic in a program's size

**Status:** part 1 (inference) and part 2 (the evaluators) done (033ee61);
part 3a (the native passes find bindings without searching, D12), part 3b
(a block's binding is a root until its last reader, D13) and part 4
(generalization by levels, D14-D16) done; the rest of part 3 (CPS joins,
site paths) measured, to be designed (2026-11-05, on `main`, an interactive
session). HANDOFF step 1, the maintainer's priority (2026-10-30: "compiler
scalability is still fragile"). The design choices are Claude's under the
maintainer's delegation (rule 2).

## 0. Measured first

Generators for each shape below, timed at c9405f1 with a release CLI built with
debug information (the HANDOFF's own numbers were a debug build), one run each, 2
CPUs; profiles with callgrind.

| shape | what grows | sizes | time | growth |
|---|---|---|---|---|
| chain | `let a{i+1} = fn() { a{i} }`, checked | 50 / 100 / 200 | 0.09 / 1.37 / 22.8 s | n^4 |
| nested performs | `op(fn() { op(fn() { .. }) })`, `effect E(t)` | 50 / 100 / 200 | 0.14 / 1.60 / 18.5 s | n^3.5 |
| two towers | `let a{i} = if c { fn() { a{i-1} } } else { fn() { a{i-1} } }` | 100 / 200 / 400 | 0.11 / 0.72 / 5.89 s (after D1, D2) | n^3 |
| many functions | `fn f{i}(x: Int) -> Int { f{i-1}(x) + 1 }` | 1000 / 2000 / 4000 | 0.17 / 0.69 / 2.95 s | n^2 (new) |
| a parameter under lets | `let g{i} = fn(y) { y + x }`, `x` a parameter | 1000 / 2000 / 4000 | 0.19 / 0.69 / 2.69 s | n^2 (new) |
| closures calling a value | `apply(fn() { f() })` down a block | 1000 / 2000 / 4000 | 0.07 / 0.26 / 0.84 s | n^2 (new) |
| a name down a block | `let a{i} = (a{i-1} + x) % 1000003`, run | 2500 / 5000 / 10000 | 0.08 / 0.22 / 0.88 s | n^2 |
| a long block | nothing used twice, run | 10000 / 20000 / 40000 | 0.10 / 0.23 / 0.53 s | linear |
| performs down a block | one per 1000, 100 or 10 statements, IR size | 1000 .. 8000 | IR x2 per x2 | linear |
| performs in `if` arms | one per 1000 statements, IR lines | 2000 / 4000 / 8000 | 84K / 280K / 1009K | n^2 |

Where the time went:
- **Generalizing** (many functions, a parameter under lets, the chain): every
  `let` and every top-level group asked what the environment leaves free by
  walking every scheme in scope (each function so far, every constructor),
  resolving each type deeply at every node it visited (`free_vars` called
  `resolve` per node: size times depth), deduplicating with `Vec::contains`.
- **Copying types** (nested performs): `resolve_row` cloned each label, its
  arguments whole, and then resolved the clone: everything under a row was
  copied twice at every level. `open_covariant` resolved its type again at every
  arrow of a result tower.
- **Unifying** (two towers): `unify` resolved both sides whole at every level of
  its recursion.
- **Inclusions** (closures calling a value): every lambda's end re-read every
  sub-effecting inclusion recorded so far in its group (`flush_inclusions`), and
  the ambient row's chain of bindings grew a link per call (each call relays its
  callee's tail), so every read of it walked the chain (`settle_duplicates`,
  `resolve_row`): 3000 links at 3000 statements.
- **Evaluation** (a name down a block): one `Rc<Scope>` per `let`, so a
  parameter's lookup walked every binding above it, hashing the name at each.
- **CPS** (performs in `if` arms): a site's continuation is the rest of its
  region; a site in an arm keeps the rest after the `if` both in its
  continuation and after the other arm, so the code is sites times length.
  Straight-line sites are linear (the direct path stops at the site).
- Also: `elya check` runs the whole front end twice (`check_source_with`, then
  `warnings_with`), `elya build` three times: a constant factor, not this step's.

### The bar

A program's types can themselves grow faster than the program: the chain's link
`i` has a type of `i` arrows, so its types total n^2, and instantiating link `i`
makes `i` fresh row variables. Nothing that writes types out is linear in the
program there. The bar is: **inference costs in proportion to the program plus
the types it writes** (the zonked table, every node's type); the evaluator costs
in proportion to the steps it takes. For programs whose types are bounded (many
functions, long blocks, closures), that is linear in the program.

### The invariant D2 rests on, measured

A quantified variable is never bound to a type or row that holds a variable. A
probe at c9405f1 (every bind of a variable some scheme quantified, printed) over
the whole suite (root, codegen and CLI tests): no quantified type variable is
ever bound; a quantified row variable is bound only to the closed empty row (a
local `let`'s phantom tail closed when its group ends), which holds nothing.

## 1. Decisions, part 1: inference

- **D1. Every walk over a type resolves it once.** `free_vars`, `free_row_vars`
  and `open_covariant` resolve their type once and walk the result (as `occurs`
  has since 2026-10-30), deduplicating with a set; `resolve_row` resolves each
  label's arguments where it reads them instead of cloning the label first.
- **D2. The environment knows which bindings leave something free.** `TyEnv`
  keeps, per scope, each name whose scheme leaves a type or row variable
  unquantified (a parameter, a monomorphic `let`, a member of the group being
  inferred), with those variables, written as a type (`(a, fn() / r -> $Row)`).
  What the environment leaves free is what those resolve to now (less each
  entry's own quantified variables), so `generalize`, `generalize_toplevel` and
  the skolem-escape check (E0435) walk those entries alone. A closed scheme can
  gain nothing free (the invariant above), so skipping it changes no answer.
  - Rejected at first: levels (OCaml's ranks): every variable's creation and
    every binding would carry a level, a larger change resting on the same
    invariant (taken in part 4, D14: what D2 kept below cost cubic time);
    shared, hash-consed types (`Rc`): every consumer of `Ty`, the back end's
    included, would change.
- **D3. `unify` follows heads, not whole types.** Each level follows its two
  sides' head variables through the substitution as it stands when that level is
  reached (a binding made for an earlier argument is seen by a later one), and
  recurses on the parts it owns; nothing is resolved twice. A variable is bound
  to the type as it stands (resolving reads through it later), after the occurs
  check, which still resolves its type once.
- **D4. Inclusions are flushed when something they read changed.** An inclusion
  `src ⊆ dst` can add something only when it is new, or when the row variable its
  `src` ended in at its last flush has been bound or has stopped being phantom
  (labels are never removed, a bound variable is never rebound). Each flushed
  inclusion waits on that variable; binding it, or unmarking it, makes it dirty.
  A flush takes the dirty ones in index order, one dirtied during the pass at a
  later index in the same pass: the order of a full pass, without the ones that
  could add nothing. A conflict or a poisoned ambient settles it for good. The
  member inclusions (recursion through a handle body or a lambda) are still read
  whole each pass: their relay test reads the group's parameters, few by
  construction.
- **D5. A row's chain of bindings is compressed where its duplicates are
  settled.** `settle_duplicates` walks a row's chain (with `&mut self`, before
  every row unification); after unifying the arguments of each effect met twice,
  it binds each variable of the chain straight to what it resolves to: the
  labels from it to the end, the first of each kept, as `resolve_row` keeps them,
  and the chain's end. What every variable resolves to is unchanged, and the
  duplicates it drops have just been unified with the ones it keeps. That
  compresses the chains walked from an ambient, not the variables a chain left
  behind it: a node that recorded one early was resolved, at the zonk, through
  every link added since (closures calling a value, measured with the zonk:
  ratio 3.35 a doubling). So before the zonk every variable bound to a row with
  no labels of its own is bound past such links, to the first variable that
  holds labels or is unbound, or to the chain's end, closed or poisoned (`compress_rows`, linear: a
  memo per variable). Nothing is dropped: those links carry no labels.
- **D6 (part 1's measure). `Infer` counts its steps.** One counter, timing-free:
  every node `resolve` builds, every row read and link it follows, every node a
  free-variable, occurs or substitution walk visits, every `unify` level and row
  unification, every link `settle_duplicates` walks, every inclusion a flush
  takes, every environment entry generalization reads.
  `elya::types::inference_steps(module)`, the zonk of every node's type
  included (the back end's inference: the CLI's `check` does not zonk, and
  measured without it, closures calling a value looked linear). The tests also count
  allocations (a counting allocator in the test binary, per thread): copying
  (D1's `resolve_row`) visits nothing it would not anyway, but allocates.

## 1b. Decisions, part 4: generalization by levels (2026-11-05)

Measured (release CLI, `elya check`, at main af3be7e): closures over a function
parameter (`fn f(k) { let g{i} = fn() { k() + i } .. }`) 1.65 / 18.1 s at 1000
/ 2000, killed at 4000 (memory, 78 s); aliases of one, each called (`let k{i} =
k{i-1}`), 2.1 / 14.4 / 110 s at 1000 / 2000 / 4000: cubic. Non-value `let`s of
function type beside generalized ones (D2's limit, section 4) 0.46 / 2.7 /
15.3 s; one recursive group whose members each bind a local `let` 2.7 / 21.5 s,
over 120 s at 4000. Three causes, each a decision.

- **D14. A `let` quantifies what its value made, by level.** Each type and
  row variable records the depth of `let` values it was made inside (`level`,
  `row_level`, `cur_level`: a `let`'s value, its annotation's check included,
  is inferred one level in). Binding a variable to a type (`bind`, `bind_row`)
  lowers every unbound variable the type mentions to the bound variable's
  level: what it stood for is where it was. A non-value `let`, not
  generalized, lowers its type's variables to the `let`'s own level: they are
  the environment's from then on. A generalized `let` quantifies the
  variables its type mentions above its level, less those kept monomorphic
  for reasons of their own (the function's annotation variables and named
  rows, the enclosing clauses' `resume` types, the rows of handles still
  being filled, the generic sites not yet checked: read as before). What the
  `let` leaves unquantified is then lowered to its level: the environment's
  from then on (a variable kept monomorphic above it, left there, was
  quantified by a later `let`: the review's finding). What the environment
  leaves free sits at the `let`'s level or below, so no binding in scope is
  read. D2's open entries stay, for the skolem-escape check (E0435) and a
  top-level group's outside, and as the check: under
  `elya::types::check_levels` (or `ELYA_CHECK_LEVELS` set) every `let` is
  generalized both ways and a difference panics. Run over the root and
  codegen suites: none (only the new complexity tests failed, by design:
  D2's walk is what they measure). D2's invariant is unchanged (asserted in
  debug builds).
  - Found with it (the review), older than levels: the rows inside an
    annotation variable's type were not kept. `let f: b = fn(x) { x + 1 }`
    makes `b`, one type for the whole function, an arrow with a row, and
    `f`'s scheme quantified that row; `Ask` arriving in it later (`b` met
    the type of a function performing `Ask`, inside a handle) bound a
    quantified row (a debug build stopped at D2's assertion), and a release
    build checked `f(1)` outside the handle clean. Kept now, with the
    annotation variables' type variables: such a program is refused, E0420
    (three of the review's programs; none in the suite or the 1000 programs
    of its generator).
  - Also: the environment's free variables collected through a set, not a
    list (`acc.contains`): every entry's variables were checked against all
    met before them (the cross-check and E0435 still read them).
- **D15. A row variable unified with another is bound to the older.** A call
  unifies its callee's row with the call's fresh one; binding the callee's to
  the fresh one made a parameter's row grow a link at every call (`1 -> 2 ->
  4 -> 6 ..`), and each closure's row, bound to the end then, was read through
  every link after it: entries times links at every `let`, the cubic. Bound
  younger to older (the larger index to the smaller), the old variable stays
  the representative. Nothing else differs: a meaningful variable bound to a
  phantom one hands its meaning on either way (`bind_row`).
- **D16. A flush reads the group's parameters only for a member inclusion.**
  Every flush (every lambda's end) read every parameter of the recursive
  group being inferred, to test member inclusions for a relay; with none
  pending it now reads none. With member inclusions pending (recursion
  through a lambda or a handle body) it still reads them all: flushes times
  the group's parameters, recorded in section 4.

## 2. Decisions, part 2: the evaluators

- **D7. A `let` binds in place in a scope no one else holds.** `Env::bind`
  inserts into the innermost scope when this environment is the only holder of
  it (`Rc::get_mut`: no closure captured it, no continuation frame holds it),
  where no one else can see the binding; otherwise it makes a new scope, as
  before. A block's `let`s share one scope until something captures it, then one
  more, and so on. A shadowing `let` in a scope no one else holds replaces the
  old binding, which nothing can read any more. Both evaluators (the CEK machine
  and the tree-walking oracle).
  - Rejected: names resolved to slots before the run (de Bruijn indices): both
    evaluators, closures, handlers and patterns would change, for the same
    asymptotics on blocks.
- **D8 (part 2's measure).** `Interp::cost()` adds every scope a name's lookup
  visits (`tests/eval_complexity.rs`'s linear test for a name used down a block).

## 3. Part 3: the native passes

Measured natively at main + parts 1 and 2 (the debug test build, front end,
lowering and `emit_ir` timed apart):

| shape | sizes | emit time | IR |
|---|---|---|---|
| a parameter down a block | 4000 / 8000 / 16000 | 0.83 / 2.63 / 9.23 s | linear |
| closures calling a value | 1000 / 2000 / 4000 | 1.02 / 3.56 / 14.2 s | linear |
| a block of strings, `s{i} = s{i-1} <> "b"` | 500 / 1000 / 2000 | 1.3 / 4.6 / 17.6 s | 0.5M / 2.0M / 8.0M lines |
| a site in an `if` arm per 1000 statements | 2000 / 4000 / 8000 | 0.27 / 0.71 / 2.35 s | 84K / 280K / 1009K lines |
| performs down a block, one per 10 statements | 1000 / 2000 / 4000 | 0.14 / 0.46 / 1.33 s | linear |

Profiled (callgrind): a name bound above a long block was searched past every
binding in it at each use, from the innermost binding out, by `mono`,
`specialize` (its rewrite and its upcast adapter), the convention check, `anf`
(a top-level name), the closure and CPS free-variable walks and the CPS saved
sets; each lambda's CPS region kept a copy of the whole scope; and rooting the
environment at a call sorted every name in scope (`gc_root_env`), parking a
shadowed value counted every key.

- **D12. The native passes find a binding without searching.** A `Scope`
  keeps its bindings in order with each name's positions, innermost last: the
  innermost binding of a name, or the innermost below a depth (what a node at
  that depth sees), without a search. Every pass above uses it; a lambda's
  region keeps the binding indices of the names it captures, not the scope.
  The emitter's environment (`Env`) keeps its pointer-valued names in name
  order and counts its hidden keys: rooting pushes the pointers in name order,
  as it pushed them from the sorted names (`gc_root` pushes pointers only), so
  the IR is the same byte for byte.
- **D13. A block's binding is a root until its last reader.** Rooting at an
  allocation or a call pushed every pointer in scope, read again or not: a
  block of strings, each made from the one before, was n^2 IR (table above).
  `Deaths` reads off a block's syntax, for each of its bindings, the last
  statement that reads it: a name free in the statement (`free_vars`, which
  knows every binder and counts a lambda's captures) whose innermost binding
  above it is that one. Once that statement is bound, the binding is retired:
  out of `Env`'s pointers (`Env::retire`) and, in the CPS emitter, out of the
  bindings `root_live` pushes. What the block's end reads, the block never
  retires. A retired binding stays retired when a later binding of its name
  parks it and when it is restored (`bind_local`, `unbind_local`): its value
  may be freed already, and rooting it again would hand the collector a freed
  block. A binding from outside the block, shadowed in it, stays as its own
  block left it.
  - Sound because the emitter reads a block's binding only through a `Var`
    of it or a capture, both counted, and what a site saves (`saved_at`,
    `handler_saved`) is what the rest reads, by the same syntax: no retired
    binding is read, saved or rooted again. A stress mode checks it:
    `ELYA_GC_STRESS=N` collects at every Nth allocation; what it frees it
    poisons and keeps, never handed out again, so a value read after its
    last root reads poison at once, not its old words; and a block it freed,
    met while marking (a root, the current handler, a field), stops the
    program (`gc_gray_push`; `gc_mark` unchanged).
  - Cost: `free_vars` of each statement once per enclosing block, nesting
    depth times size, as the closure pass's captures already are (the
    review: 300 nested blocks over 4000 statements, 0.36 to 2.89 s; the
    parser's 1024 levels bound it). A resumption lowers the rest of its
    block again and reads its deaths again: a third more on a shape already
    quadratic through its saved sets (a site every 10 statements, 5.56 to
    7.49 s at 4000), which D11 is to take.
  - Not taken: rooting once at the binding (pushed when bound, popped at its
    scope's end, a site rooting its own operands alone). It would be linear
    even where everything stays live (below), but every exit (`ret`,
    `musttail`, the CPS jumps) would pop what its function holds: a
    GC-discipline change across both emitters, and it keeps a dead binding
    to its scope's end, which liveness frees.
  - What stays: a block whose values all stay live to its end
    (`let s{i} = "b" <> int.to_string(i)`, summed after) roots every one so
    far at every allocation: IR in proportion to the live pointers at each
    site, n^2 here.
- **Next (measured, to be designed):**
  - a site in an `if` arm copies the rest of its block twice over: a join (the
    conditional itself a site whose frame the arms continue to);
  - a site's path holds every `let` above it (its resumption reads that path
    quadratically) and its saved set reads the whole rest of its block;
  - anf's rule 4 renames the rest of a block at each alias of a
    row-polymorphic local (`let k{i} = k{i-1}`): emit 0.16 / 0.61 / 2.38 s at
    200 / 400 / 800, `anf::run` 82% of it (found by the review of part 3a;
    older than it). Carry the renaming down the walk instead. The front end
    on the same shape: 0.61 / 4.06 / 26.7 s, a part 1 matter.

## 4. Not in this step

- The front end run twice by `elya check` (three times by `elya build`): a
  constant factor.
- `infer_call` reads every parameter type in scope at every call (lambdas
  nested in lambdas: depth times calls); `generalize` reads the pending clause
  rows, deferred rows and generic sites at every `let`. Both bounded by what is
  open at once, not by the program; measured clean on every shape above.
- **A limit D2 kept (the review's finding 2), removed by D14:** generalization
  read every binding still open, so bindings that stay open piled up: a block
  of non-value `let`s of function type (`let r = id(fn(y) { y })`, its phantom
  rows open until the group ends) under generalized `let`s was quadratic (4000
  of each: 16.2 s, 14.8 s before D2), and so was a large recursive group whose
  members each bind a local `let`. Levels read none (part 4: 0.12 s and
  0.28 s at 4000).
- What part 4 keeps: E0435's check and a top-level group's outside read the
  open entries (once per `with any` clause, once per group); a flush with
  member inclusions pending reads the group's parameters (D16).
- One top-level group's outside is read once for the whole group (the
  review's finding 3: per member, a group of 4000 took 35 s), its members'
  names in a set.

## 5. Evidence

Parts 1 and 2:
- At c9405f1 and after (release CLI): a chain of 200 closures 22.8 s -> 0.08 s;
  200 nested performs 18.5 s -> 0.29 s; two towers of 400, 5.89 s -> 0.48 s;
  4000 functions 2.95 s -> 0.09 s; 4000 lets under a parameter 2.69 s -> 0.06 s;
  4000 closures calling a value 0.84 s -> 0.08 s; a name down 10 000
  statements, run, 0.88 s -> 0.08 s.
- At ten times the HANDOFF's sizes (debug CLI): 1000 chained closures 10.4 s
  (6.5 s at 100 before; the types are n^2: 0.09 s at 100 now), 50 000
  statements using a parameter, run, 2.1 s (4.5 s at 5 000 before), 500 nested
  performs 5.0 s (8.4 s at 100 before; 1000 is refused by the parser by name,
  "more than 1024 levels deep"), 40 000 functions 6.2 s, lets under a parameter 3.2 s,
  closures calling a value 6.0 s, two towers of 1000, 7.6 s.
- Tests (red first on the old code, with the measures added to it): many
  functions, lets under a parameter, closures calling a value at ratios 3.89,
  3.93, 3.71 (linear is 2); the chain 355 -> 1236 steps per type node; nested
  performs 90 -> 177 allocations per type node (its steps passed: copying
  visits nothing); two towers 20 -> 36 steps per type node; a long block passed
  (a control). The evaluators' lookups at ratio 3.96 (machine) and 3.99 (tree).
  After: ratios 1.95 to 2.00, and 0.96 to 1.00 per type node.
- Probe for D2's invariant: no quantified type variable bound, a quantified row
  variable bound only to the closed empty row, over the whole suite.

Part 3a (D12):
- Emit times (debug, `emit_ir`) at 4000 / 8000 / 16000 statements: a parameter
  down a block 0.36 / 0.69 / 1.48 s (0.83 / 2.63 / 9.23 before); closures calling
  a value 0.91 / 1.94 / 3.96 s (14.2 s at 4000 before, 283 s at 16 000).
- Nothing else changed: the IR of every module the codegen suite compiles,
  dumped by content hash before and after, the same 583 modules.
- Tests: `crates/codegen/tests/native_scalability.rs` (IR lines and the
  bindings examined, `elya_codegen::scope::probes()`, at n and 2n). Controls:
  K12, `Scope` searching from the innermost binding out, fails all three (ratio
  4.00); K13, rooting reading every name in scope, fails the two with calls.

Part 3b (D13):
- IR lines (debug, `emit_ir`) at 250 / 500 / 1000 statements, before: a block
  of strings 126K / 503K / 2.0M; under one name the same; in an effectful
  function 128K / 506K / 2.0M; a site every 100 statements 49K / 108K / 216K;
  every string live to the end 317K / 1.26M / 5.0M. After: 2.4K / 4.6K / 9.1K;
  the same; 3.5K / 6.7K / 13.2K; 4.9K / 10.3K / 21.1K; and every string live
  the same as before, 316,607 / 1,258,107 at 250 / 500 (what liveness
  cannot shorten). At 2000 / 4000 / 8000: strings 18K / 36K /
  72K lines, emit 0.28 / 0.41 / 0.95 s (8.0M lines and 17.6 s at 2000 before).
- Tests: `crates/codegen/tests/native_scalability.rs` (+4, red first at 3.96 to
  3.98), `crates/codegen/tests/native_gc_stress.rs` (programs under
  `ELYA_GC_STRESS=1` answering as the evaluator does), two unit tests in
  `lib.rs`. Controls K15 to K21, K28, K29 (the plan). The review: 1205
  sweep programs, 1200 generated and 26 adversarial ones under stress,
  base and change alike; of 600 modules' IR, 505 the same byte for byte and
  95 differing only by `gc_push` / `gc_pop` calls removed.

Part 4 (D14-D16), release CLI, `elya check`, at 1000 / 2000 / 4000, before (main
af3be7e) and after: closures over a function parameter 1.65 / 18.1 s / killed
(78 s) and 0.04 / 0.05 / 0.07 s; aliases of one 2.1 / 14.4 / 110 s and 0.02 /
0.04 / 0.18 s; non-value lets beside generalized ones 0.46 / 2.7 / 15.3 s and
0.03 / 0.11 / 0.12 s; a recursive group with local lets 2.7 / 21.5 / over 120 s
and 0.04 / 0.19 / 0.28 s. Tests: `tests/infer_complexity.rs` (+4, red first:
7.55 and 7.35 for the first two, the cubic; 3.29 for the group under D14 and
D15 alone), `tests/levels_agree.rs` (new, 6: the examples, the ui tests, the
row-parameter program, ten programs written for levels, the open shapes and
eight of the review's, all under `check_levels`; an annotation variable's
row one row for the function; an effect hidden through an annotated `let`
still refused). Controls: K23 a non-value `let` not lowering its type
(fails the written programs: a closure over its value quantified `L(b)`'s
`b`); K24 `bind` not lowering (the written programs and the open shapes);
K25 `bind_row` not lowering (the same); K26 rows bound older to younger
(aliases, 2.87); K27 quantifying at the `let`'s own level too (both); K30 a
`let` not lowering what it leaves unquantified, K32 its annotation checked
outside its level, K33 `resume` types not kept (the review's programs); K31
an annotation variable's rows not kept (those and both refusals). The
review: 1000 generated programs agree and check as before levels; the
codegen suite under `ELYA_CHECK_LEVELS` passes (322).
