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
2. [ ] 4d, `selfhost/exhaust.elya`: Codex, after 4c.
3. [ ] 4e, `selfhost/affine.elya`: Codex, after 4d.
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
   - F4: effects. Declarations, performs, `handle` and `resume`, rows with
     labels and tails, row variables and their inclusions, phantom rows,
     parametric effects, `with any`, `multi`.
   - F5: duplicability (the marks on type variables, E0428, E0429, E0437),
     `linear` types, `Buffer` and `Array`, and the borrow facts (E0438,
     E0439). These read 4e's facts, so F5 waits for 4e.
   - F6: the grants of modules part 2 (E0606) on multi-file programs, and
     the whole corpus.
   Each stage gets negative controls in the Elya code (a row printed
   unsorted, a scheme generalized one level too deep, and so on), and a native run of the largest program in its fragment, timed
   against Rust.
