# Plan: compiler scalability, part 3a (the native passes find bindings without searching)

Spec: `docs/superpowers/specs/2026-10-31-elya-scalability-design.md` (D12).
Base: `main` after parts 1 and 2.

## Steps

1. Measure natively (front end, lowering, `emit_ir` apart; callgrind): a
   parameter down 16 000 statements took 9.2 s to emit (0.83 s at 4000);
   closures calling a value 14.2 s at 4000 (1.0 s at 1000). Profiled: the
   scope searches of mono, specialize (rewrite and adapter), the convention
   check, anf, closure and CPS analyses; a copy of the scope per lambda;
   rooting sorting every name in scope at each call.
2. `crates/codegen/src/scope.rs`: `Scope` (bindings in order, each name's
   positions) and `Env` (the emitter's names, its pointers in name order, its
   hidden keys counted); `probes()` counts the bindings lookups and rooting
   examine. Every pass above uses them.
3. Proof that nothing else changed: the IR of every module the codegen suite
   compiles, dumped by content hash before and after: the same 583 modules.
4. Tests, `crates/codegen/tests/native_scalability.rs` (new, 3): a parameter
   down a block, calls down a block, closures down a block; IR lines and probes
   at n and 2n, each linear. A unit test of `Scope` in `scope.rs`.
5. Controls: K12, `Scope` searching from the innermost binding out: all three
   fail (ratio 4.00); K13, rooting reading every name in scope, sorted: calls
   and closures fail, the parameter (no rooting point) passes.

## Predictions

- Gate: 1429 + 1 (the unit test) + 3 = **1433 passed, 154 suites**.
- `gc_mark` unchanged (637 bytes); the runtime untouched.
- Emit times at 4000 / 8000 / 16000 statements (debug): a parameter down a
  block 0.36 / 0.69 / 1.48 s (0.83 / 2.63 / 9.23 before); closures calling a
  value 0.91 / 1.94 / 3.96 s (14.2 s at 4000 before, 283 s at 16 000).
