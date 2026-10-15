# Plan: native strings

Spec: `docs/superpowers/specs/2026-10-15-elya-native-strings-design.md`. Baseline gate at
258dda6: 920 passed, 87 suites.

1. **Front end and evaluator: `int.to_string`.** Done, cc044d4.
   - Red `tests/strings.rs`, 3 tests (E0201 today).
   - Then add it to `resolve::builtins`, to `infer_call` (`fn(Int) -> String`, pure), to
     Core lowering, and to both evaluator paths (the CEK machine and the tree-walking
     oracle that `crosscheck` uses).
   - Predicted gate: 920 + 3 × 2 = **926**, 87 + 2 = **89** suites. Measured: 926 / 89.
2. **Native `<>`, `==`/`!=` on strings, `int.to_string`.**
   - Red, first:
     - the strings corpus, which fails with "Concat";
     - the collection test and the CPS test;
     - the two flipped unit tests;
     - the ADT version of the allow-list test, which passes already: its refusal is
       unchanged.
   - Then add the runtime functions `elya_str_concat`, `elya_str_eq` and
     `elya_int_to_string`, and the emitter arms (direct, plus both CPS paths).
   - Native tests: predicted +3. Measured: **+4**, gate 930 / 89. The fourth is
     `a_left_operand_survives_a_collection_in_the_right_one`, which control C1 made
     necessary (step 3).
3. **Negative controls.**
   - C1: the left operand not rooted while the right is lowered. Predicted to fail the
     collection test. Measured: it failed NOTHING. A sweep frees about 30,000 blocks and
     the free lists hand back the oldest dead block first, so the freed operand was
     never reused before it was read. A program built so that it is (a 9-word literal,
     the only block of its size class) became the fourth test; C1 fails it, with 0
     instead of 1.
   - C2: `==` compares lengths only. Fails `eq-same-length` (1 for 10), as predicted.
   - C3: `int.to_string` without the sign. Fails `int-to-string`, as predicted.
   - C4 and C5: the CPS emitter's `root_live` around `<>` and around `int.to_string`
     removed. My probes saw nothing; the review showed both are load-bearing (step 5).
4. **Sweep re-run.** Predicted: the 16 programs refused as "Concat" compile and match
   the evaluator, and nothing else changes. Measured: those 16, plus the one string
   `==` refusal, compile and match byte for byte, and no other native result changed.
5. **Independent review.** No correctness bug, but four findings, each fixed test-first:
   - C4 and C5 are load-bearing. In a resumption function the resumed frame is dead, so
     only `root_live` keeps the continuation and the saved bindings alive. Two
     deterministic tests now fail under C4 (7021 for 20000) and C5 (5242 for 20000).
   - A builtin used as a value (`let f = int.to_string`) passed `check`, then failed at
     run time. This was already true for `io.println` before this change. It is now
     E0208 (spec D6), with one test in `tests/strings.rs`.
   - String-literal patterns compile natively now, because `patterns.rs` turns a literal
     test into `==`. They are pinned by three corpus rows.
   - Docs: the README's "strings have no operations" sentence, the `gc_root` doc
     comment that my helpers had separated from its function, and this plan's counts.
   - Final predicted gate: 930 + 2 native + 1 × 2 root = **934**, **89** suites.
6. HANDOFF (step 2 to Done, write a new step 5) and the log.
