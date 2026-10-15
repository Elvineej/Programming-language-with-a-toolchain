# Plan: native strings

Spec: `docs/superpowers/specs/2026-10-15-elya-native-strings-design.md`. Baseline gate at
258dda6: 920 passed, 87 suites.

1. **Front end and evaluator: `int.to_string`.**
   - Red `tests/strings.rs`, 3 tests (E0201 today).
   - Then add it to `resolve::builtins`, to `infer_call` (`fn(Int) -> String`, pure), to
     Core lowering, and to both evaluator paths (the CEK machine and the tree-walking
     oracle that `crosscheck` uses).
   - Predicted gate: 920 + 3 × 2 = **926**, 87 + 2 = **89** suites.
2. **Native `<>`, `==`/`!=` on strings, `int.to_string`.**
   - Red, first:
     - the strings corpus, which fails with "Concat";
     - the collection test and the CPS test;
     - the two flipped unit tests;
     - the ADT version of the allow-list test, which passes already: its refusal is
       unchanged.
   - Then add the runtime functions `elya_str_concat`, `elya_str_eq` and
     `elya_int_to_string`, and the emitter arms (direct, plus both CPS paths).
   - Native tests: +3. Predicted gate: 926 + 3 = **929**, **89** suites.
3. **Negative controls.**
   - C1: the left operand not rooted while the right is lowered. The collection test must
     fail (wrong text or a crash).
   - C2: `==` compares lengths only. The corpus fails on a same-length pair.
   - C3: `int.to_string` without the sign. The corpus fails on the negative row.
   - Each control is reverted and checked with `cmp`.
4. **Sweep re-run.** The 16 programs refused as "Concat" must compile and match the
   evaluator, and nothing else may change.
5. **Independent review**, fix findings test-first, then HANDOFF (step 2 to Done, write a
   new step 5) and the log.
