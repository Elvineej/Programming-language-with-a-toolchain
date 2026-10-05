# Slice 5c-2 — name rules: duplicate clauses, and locals before ops

**Status:** approved (maintainer, 2026-10-05). Follows 5c-1 (unique op names). Front end,
evaluator and Core lowering; native inherits the result.

## 0. The cut — two parked language questions, decided

1. **Two clauses for one op in a handler** (PARKED, 5c-1 measurement m8). Legal today,
   first clause wins in both the evaluator and (since 5c-1) native. **Decision: error,
   E0204.** The second clause can never run.
2. **A name that is both an op and a function or local** (PARKED, 2026-10-03). Today an op
   wins over everything, in all three layers (inference `infer_call`, the evaluator's
   `CalleeSlot::Operation`, lowering's D11). **Decision: a local binding (let, parameter,
   lambda parameter, clause parameter, match binder) shadows an op, lexically; a top-level
   function named like an op is an error, E0205**, since it could never be called.

Measured at `66fb01b` (2026-10-05). Each program handles `E.ping() -> resume(1)` around a
body; the reported line says which `ping` ran:

| # | Program | check | run | build |
|---|---|---|---|---|
| n1 | two `Ask.ask()` clauses (5c-1 m8) | ok | first wins | first wins |
| n2 | `fn ping() -> Int { 5 }` beside op `ping` | ok | PERFORMED | PERFORMED |
| n3 | `let ping = fn() { 5 }  ping()` | ok | PERFORMED | PERFORMED |
| n4 | `fn user(ping) { ping() }` | ok | PERFORMED | refused, unrelated (`user` is generic: `Ty::Var`) |
| n5 | `{ let ping = .. ping() }` then `ping()` after the block | ok | both PERFORMED | both PERFORMED |

After this slice: n1 E0204; n2 E0205; n3 and n4 CALL the local; n5 calls the local inside
the block and performs after it.

## 1. Rules

1. **E0204 — a second clause for an operation already handled by this handler**, labelled
   at the second clause. Two clauses for one op whatever their spelling (`ask()` and
   `Ask.ask()` too: op names are unique, so the op name decides). In `resolve`'s `Handle`
   arm; skipped for an op already reported E0202 or E0203.
2. **E0205 — a top-level function named like an operation**, at the function. In
   `resolve`'s declaration pass.
3. **Locals before ops, everywhere a call is resolved.**
   - `resolve` is the one static scope walker: it records the spans of call sites whose
     callee is a local binding that shares an op's name, exposed as
     `resolve::locally_shadowed_op_calls(module)`.
   - Inference: `infer_call` takes the perform path only when the name is not bound in the
     type environment. Under E0205 the environment can hold such a name only as a local.
   - Evaluator: `CalleeSlot::Operation` only when the runtime `Env` (locals only; top-level
     functions are a separate table) does not bind the name.
   - Core lowering: a call is a `Perform` only when its span is not in the resolver's set.
   - *Amended (independent review):* the affine checker is a FOURTH layer. It treated any
     call named like a `multi` op as a multi-shot perform, so a local named `flip` drew a
     false E0429. It now reads the same resolver set.
   Four layers, one rule; the set is computed by the same walk that already decides
   E0200, so it cannot disagree with what `check` accepted.
4. *Amended (review):* a clause already reported E0203 is not judged by E0204 and does not
   count as its op's clause.

## 2. Non-goals

- A local shadowing a constructor or a builtin: unchanged.
- An op used as a first-class value (`let f = ping`): unchanged.
- n4 natively: its refusal is N7's (`Ty::Var`), not this slice's.

## 3. Acceptance

| # | Criterion | Kind |
|---|---|---|
| C1 | n1 and the mixed-spelling duplicate are exactly `[E0204]`. | execution |
| C2 | n2 is exactly `[E0205]`. | execution |
| C3 | n3, n4, n5 evaluate to the local's answer (n5: local inside, op after). | execution |
| C4 | n3 and n5 natively match the evaluator (handler-corpus differential). | differential execution |
| C5 | Nothing already accepted changes meaning except by these rules: existing tests unchanged, PARKED repros updated. | execution |
| C6 | Negative controls, one per layer, each failing differently, reverted. | procedure |
| C7 | Windows CI green. | execution |
