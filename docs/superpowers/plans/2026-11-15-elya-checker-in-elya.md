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
   - F2: lambdas and closures, let-polymorphism by levels (D14-D16 of the
     inference-levels spec), tuples, `type` declarations, constructors and
     `match`, the builtins' signatures (`int.*`, `float.*`, `string.*`).
   - F3: annotations (parameter, return, `let`), checked, and the kinds of
     declaration parameters (`decl_kinds`).
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
