# Plan: the checker in Elya (self-hosting parts 4d to 4f)

Spec: `docs/superpowers/specs/2026-11-15-elya-checker-in-elya-design.md`.

1. [x] D2's print forms: `elya exhaust --dump`, `elya affine --dump`
   (`elya::check_dump::affine_facts`), and a warning printed as `warning`
   by `dump_diagnostics`. Test: `exhaust_and_affine_dumps` in
   crates/cli/tests/run_cli.rs (each of the six fact kinds; a warning's line).
   Gate predicted: +2 (the CLI suite runs in configurations A and B), no new
   suite: 1663 passed, 193 suites; measured 1663 and 193. Review: no defect;
   every field printed, nothing ordered by hashing (md5 equal over repeated
   runs), the same input as inference's, the `warning` change breaking no
   pinned output. Its notes, now in the spec: the exhaust dump runs on
   ill-typed programs on purpose; `resumes` keeps the walk's order; inference
   analyzes the same module. Docs only after the gate.
2. [x] 4d, `selfhost/exhaust.elya` (**built** 2026-11-24 by Claude, since
   Codex was out of reach): Maranget's usefulness, as in src/exhaust.rs,
   keeping Rust's behaviour on ragged rows (an ill-typed constructor
   pattern) and the walk's order of diagnostics. Tests:
   tests/selfhost_exhaust.rs and crates/codegen/tests/selfhost_exhaust.rs
   compare `elya exhaust --dump` on every corpus program that resolves
   without an error, ill-typed ones included (503), and 20 focused
   programs, in the evaluator and natively. Controls E1-E18 in the test's
   header, all caught. Review: no defect; 7,948 random programs that
   resolve (5,334 with diagnostics) agree natively. Of 102 mutants, 10 are
   equivalent (a witness never holds an Int or String literal, those
   columns being infinite, so `debug_str` is kept only as Rust's twin; a
   constructor is never declared twice, E0209; a message never needs
   quoting), and 14 lived through the tests: ten more focused programs
   kill them.
3. [x] 4e, `selfhost/affine.elya` (**built** 2026-11-24 by Claude, since
   the checker's F5 stage needed its facts first; `pub fn facts` is the
   entry F5 calls, and `Frame`/`Why` became `CFrame`/`AWhy` so the names do
   not clash with check.elya's). Tests: tests/selfhost_affine.rs and
   crates/codegen/tests/selfhost_affine.rs compare `elya affine --dump` on
   every corpus program that resolves, in the evaluator and natively.
   Review: no defect; 16,000 random programs agree natively (about 1.9M
   fact lines), and the read-through found every HashMap order that could
   reach the output replicated (the size swaps in `add`/`max_of`, the walk
   order of `resumes`). Of 128 mutants, 23 are argued equivalent (D9 among
   them: an arm's walk only adds earlier uses or removes its own binders,
   so joining onto what follows the match equals joining onto nothing);
   46 of the rest lived through the corpus and the focused set, and 44 more
   focused programs, one per mutant and named in the comments, kill them
   all. Known limit: a span key is `start * 2^20 + end`, so a file of 1 MiB
   or more would order keys wrongly.
4. [ ] 4f, inference in Elya (`selfhost/check.elya`): Claude. Staged by
   the fragment of the language a program uses. Each stage compares `elya
   check --dump` byte for byte on every corpus program inside its fragment.
   A Rust classifier over the AST (tests/support/selfhost_check.rs) picks
   them and says how many, so each stage's subset is measured and only
   grows:
   - F1, first order (**built** 2026-11-22: `selfhost/check.elya`,
     tests/selfhost_check.rs and crates/codegen/tests/selfhost_check.rs;
     49 corpus programs and 20 focused ones with errors agree, in the
     evaluator and natively; controls K1-K3 in the test's header; the
     parser in Elya gained `parse_tree`, a public entry for later passes.
     Review: no defect in the checker (80 random call graphs agree, Tarjan's
     order included); the classifier let through calls of call results,
     callees shadowed by a local, and base types given arguments (fixed:
     those are F2 and F3); five behaviours were unpinned (the value
     restriction, two level lowerings, the occurs check, the group's member
     order), now pinned by four more focused programs, the occurs check
     moving to F2 with calls of call results; the native test gained the
     evaluator's floor. Gate 1666 passed, 196 suites, as predicted): `Int`, `Float`, `Bool`, `String`, `Unit`, `let`,
     `if`, operators, calls of top-level functions, recursion and mutual
     recursion (the call graph's groups, generalized in Rust's order). With
     it: the type printer (`write_ty`, `write_row`, `letter`, the `Names`
     shared across lines of one dump) and the diagnostics of this fragment.
   - F2a (**built** 2026-11-22): `type` declarations and their
     constructors' schemes, constructor calls (E0433), `match` with every
     pattern (E0402 on fields, E0430 on an empty match), the pure builtins
     (`int.*`, `float.*`, `string.*`), annotations naming declared types and
     type variables (E0400, E0432). 32 corpus programs and 32 focused ones
     agree, in the evaluator and natively; controls K4-K7 in the test's
     header. Review: one defect (a `let` could quantify the function's
     annotation variables, `kept_monomorphic`; and the unquantified
     variables above the level were not lowered), fixed with K7; its
     mutation testing found 15 unpinned behaviours, now pinned by focused
     programs; about 55 probes otherwise agreed. Gate 1666 passed, 196
     suites, as predicted. No effect rows: everything here is called directly, so every
     row closes to pure.
   - F2b (**built** 2026-11-23; the classifier's stage 3; F3's annotations
     went to F2a and F2b, the kinds of declaration parameters to F4):
     higher order. Rows are tails only (no labels before F4), so
     unification of rows is a union-find whose phantom-ness belongs to a
     class, and the inclusions reach their fixpoint at Rust's three flush
     points (a lambda's exit, steps 2.4 and 2.7) without its event queue.
     Equality on functions (E0436, with the marks on type variables) came
     with it. 36 corpus programs and 35 focused ones agree, in the
     evaluator and natively; controls K8-K13 in the test's header (two of
     them survived the corpus and got a focused program each). Review
     (about 130 handwritten and 2,200 random stage-3 programs, an
     instrumented oracle): one defect, a `let` quantifying a deferred
     lambda's row (Rust's `kept_monomorphic` keeps `deferred_rows`), 7 of
     300 random programs; fixed with K13. It confirmed the fixpoint claim
     (phantom-ness is a class property; every mark read is on a
     representative or an unbound variable), and its mutations found 13
     unpinned behaviours, now pinned by 12 focused programs. Gate 1666
     passed, 196 suites, as predicted. Lambdas and closures, functions and n-ary
     constructors as values, calls of locals and of expressions, function
     types in annotations, with let-polymorphism over them (D14-D16 of the
     inference-levels spec). These bring effect rows into the printed types
     even in pure programs (`fn app(f, x) { f(x) }` is
     `forall a b c. fn(fn(a) / {c} -> b, a) / {c} -> b`), so F2b ports the
     row variables' tails: phantom rows, relays through parameters,
     inclusions, closing what nothing relays.
   - F4: effects, in two parts (the classifier's stages 4 and 5).
     - F4a (**built** 2026-11-23): effect declarations and their
       operations, performs, `handle` with return and operation clauses,
       `resume` (escaping into a lambda too), rows with labels (sorted, each
       keeping the span that introduced it, the first along a chain), rows
       written in annotations and declared on functions (E0421, E0423),
       unknown effects (E0432), the host's builtins (`io.*`, `time.*`), and
       `main`'s discharge (E0420). The inclusions now carry labels, so a
       conflict ends one (`incl_dead`); `flush` still passes over all of
       them until nothing gets in, which reaches what Rust's queue of the
       woken ones reaches in the same order. 232 corpus programs and 58
       focused ones agree, in the evaluator and natively; controls K14-K28
       in the test's header (K21 survived the corpus and got a focused
       program; K24, K26 and K27 cannot be caught inside F4a, and the
       header says why). Review: no defect (about 5,350 handwritten and
       random F4a programs agree byte for byte; the port read against Rust
       function by function; the repeated-pass `flush` argued equivalent to
       Rust's queue, its kills load-bearing for termination). Its 39
       mutants left six unpinned behaviours, now pinned by eight focused
       programs; six more argued equivalent in F4a, and one (a
       conflict's span among several labels) unreachable from the programs
       it tried. The classifier's `Op` rule was dropped: a written `Op` is
       an ordinary name in Rust (`$Op` is internal), so F2b's focused #12
       is compared again.
     - F4b (**built** 2026-11-23; the classifier's stage 5, stage 4's
       remainder split in two by measuring: 27 corpus programs here, 65 in
       F4c): `with multi` (E0427 over a one-shot effect, the warning E0426
       when the body performs `IO`; diagnostics gained their kind), named
       row variables (one row per function, kept from a `let`'s
       quantifying, a declared row ending in one checked at Rust's step
       2.48 with its member's names, E0432 and E0400 where a name first
       appears), and row parameters of declared types (their kinds by
       Rust's fixpoint, a row argument as its phantom `fn() / R -> $Row`,
       printed as its row; E0400 either way round, E0432 for an undeclared
       row variable). 27 corpus programs and 32 focused ones agree, in the
       evaluator and natively, all at the first run; controls K29-K43 in
       the test's header (three survived the corpus and got focused
       programs; K41 cannot be caught, and the header says why). Review
       (21,104 handwritten and random stage-5 programs agree; no classifier
       hole; the port read against Rust): no defect; of its 74 mutants, 21
       survived and are now pinned by 20 focused programs, 16 argued
       equivalent. It found two quirks in Rust, kept by the port and left
       for a decision: step 2.48 checks a declared row's name against the
       annotation variables of the group's last member, not its own
       (`fn a(x: e) / {e} -> Int` reports E0400 or not by declaration
       order); and `{io}` is a row variable, since only its first letter
       is capitalized before the comparison with `IO`.
     - F4c (**built** 2026-11-23; the classifier's stage 6, `with any`
       measured apart into stage 7): parametric effects. A label carries
       its arguments through every walk of a type (resolving, free
       variables and rows, occurs checks, levels, instantiation,
       printing); where a label meets itself its arguments agree
       (`unify_effect_args`, E0423 for two that cannot be one type), read
       again until no label is newly shared, and a label met twice along a
       chain is settled first (`settle_duplicates`); a row whose labels
       mention its own tail joins an ambient whole; effects' parameters
       have kinds too, are instantiated once per perform and once per
       handler, and are written in annotations (fresh when unwritten) and
       declarations (the effect's own when unwritten in its own); a declared
       row's written arguments (2.47) and the labels its named tail cannot
       hold are checked. 38 corpus programs and 42 focused ones agree, in
       the evaluator and natively (the 38 at the first run); controls
       K44-K64 in the test's header (seven survived the corpus and got
       focused programs; K49, K57, K58 argued). Review (about 15,500
       handwritten and random programs): two defects, fixed with focused
       programs: a settled chain was not compressed (Rust rebinds each of
       its variables to the chain's end), so its duplicate was reported
       again (the F4d review met it too), and a clause for another effect's
       operation took the handled effect's substitution, not its own
       effect's parameters zipped with the handle's arguments. One
       classifier hole: a label's arguments were not walked, so `St(Buffer)`
       counted as stage 6. Two of the three arguments failed (K49 and K57
       are caught now); of its 63 mutants, 11 survivors are pinned, five
       argued equivalent, two (the settled pairs' order with three
       instances, `add_row` not settling its own row) unpinned, reachable
       only in principle.
     - F4d (**built** 2026-11-23; the classifier's stage 7): `with any`
       and `op.*`. A clause's operation is the `$Op` type, printed
       `Op(t / {..})`, of a rigid answer type `'tN` per clause; its
       `resume` reruns only the body's `IO`; `op.perform` forwards; the
       handled row (the body's but `IO`) is data, checked when the group
       is solved (E0434, step 2.8) and again where a scheme quantifying a
       still-open handled tail is instantiated (unless it is the
       function's own row), and stays monomorphic in a `let` until then;
       an answer type may not escape its clause (E0435); an operation is
       data for `==`. Rust's `cur_span` is read only by those
       instantiations, so it is passed, not tracked (tracked, it cost
       native checking 3.7 times). 27 corpus programs and 29 focused ones
       agree, in the evaluator and natively (the 27 at the first run but
       for `==` on an operation); controls K65-K81 in the test's header
       (two survived the corpus and got focused programs; K79 cannot be
       caught, and the header says why). Review (about 12,200 programs):
       two defects, both fixed with controls: E0435's walk of the
       environment counted a binding a later `let` hid in its block (Rust
       walks each scope's open entries; the port now marks blocks), and
       F4c's settling left a chain's duplicate in place, reported again by
       the next settling (Rust compresses the chain; ported). Its 34 mutants
       left 13 unpinned behaviours, now pinned; the `cur_span` claim and
       the omitted `hold_*` bookkeeping (stage 8's) confirmed.
   - F5: duplicability (the marks on type variables, E0428, E0429, E0437),
     `linear` types, `Buffer` and `Array`, and the borrow facts (E0438,
     E0439). These read 4e's facts, so F5 waits for 4e.
   - F6: the grants of modules part 2 (E0606) on multi-file programs, and
     the whole corpus.
   Each stage gets negative controls in the Elya code (a row printed
   unsorted, a scheme generalized one level too deep, and so on), and a native run of the largest program in its fragment, timed
   against Rust.
