# Async step 2, part 1: structured concurrency -- plan

Spec: `docs/superpowers/specs/2026-10-15-elya-async-step2-structured-design.md`. Branch
`auto/elya` at `4c40a49`. Baseline Linux gate: 913 passed, 87 suites.

- `examples/06_structured.elya` (written first, as the measurement: E0423 before the
  lambda fix; red snapshot -- none stored -- until accepted).
- `tests/examples.rs::structured_concurrency`: root crate, counts twice: +2.
- `crosscheck.rs`: the example joins the effectful list (the tree-walker must refuse it;
  no count change).
- `native_codegen.rs::the_structured_concurrency_example_runs_natively`: +1.

Predicted gate: 913 + 2 + 1 = **916 passed, 87 suites**.
