# Plan: operation-generic handlers (`with any`)

Spec: `docs/superpowers/specs/2026-10-16-elya-any-handlers-design.md`. Baseline: 946
passed, 91 suites. Root-crate tests count twice; a new root test file adds two suites.

1. **Parse.** `Handler.any`; `with any { op(o) -> e  return(x) -> e }`; E0100 for any
   other clause, for a second `op` clause, for `op` with other than one binder, and for
   `with multi any`. Resolve: `o` is bound in its clause, and `op.name/show/perform/
   encode/decode` are builtins. Red `tests/any_handler.rs`, 2 tests (accepted shapes;
   refused shapes). Predicted **950**, **93** suites.
2. **Types.**
   - `o : Op(t)`, with `t` a fresh rigid type per clause; `resume : t -> R`.
   - The builtins' types.
   - The residual is `IO` only; `op.perform` adds the handled row to the clause.
   - E0435 when `t` escapes.
   - 4 tests: the types; the residual keeps `IO` and drops the rest; E0435 through the
     result type and through an outer binding; builtins applied to a non-operation.
   - Predicted **958**.
3. **E0434.** Data checks for concrete labels, and generically handled row variables
   through schemes. 3 tests:
   - a function in an operation's type;
   - `IO` reaching the handled tail through a call;
   - a generic caller passing its own row on, checked at ITS caller.

   Predicted **964**.
4. **Evaluator.** An `any` handler matches every perform. `o` is a value carrying the
   effect, the operation and the arguments. The five builtins (decode checks the
   declared answer type). 3 tests:
   - a forwarding recorder over two effects;
   - an encode/decode round trip of an ADT answer;
   - a decode failure by name.

   Predicted **970**.
5. **Native refusal by name** (Core lowering). One codegen unit test. Predicted **971**.
6. **The example** `examples/09_record_any.elya`: one generic `record`/`replay` over two
   effects, with a snapshot (+1 ×2) and a crosscheck listing (an effectful example).
   Predicted **973**, **93** suites.
   - Controls:
     - A1: `with any` lets `IO` be handled (the residual empties). The `IO` test fails
       with a missing `IO`.
     - A2: no escape check. The E0435 tests fail.
     - A3: no mark copied at `instantiate`. The propagation test fails.
     - A4: `decode` does not check the type. The named-failure test fails.
7. **Review**, then HANDOFF and the log.
