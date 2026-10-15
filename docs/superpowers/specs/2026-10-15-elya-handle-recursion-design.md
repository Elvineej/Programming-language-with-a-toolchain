# Recursion through a handle body

**Status:** done (2026-10-15). A prerequisite of HANDOFF step 1 (async step 2: a recursive
scope handler) and step 2 (handler frames in deep chains). Design choices are Claude's
under the maintainer's delegation (rule 2), recorded here. PARKED since slice 5b-10.

## 0. Measured first

At `9eb8d5f`, `elya check`:

| Program | check | evaluator (with the fix) |
|---|---|---|
| n1 `fn nest(n: Int) / {T} -> Int { if n == 0 { t() } else { handle { nest(n - 1) + get() } with { S.get() -> resume(1) .. } } }` | E0423 + E0420 | 1000 |
| n2 n1 unannotated | E0420 (`S` never handled) | 1000 |
| n3 n2 without `T` | E0420 | 1000 |
| n5 split into `nest` / `wrap` (mutual recursion) | E0420 | 1000 |
| n6 n4 under a `T` handler, depth 300 | E0420 | 3070 |
| n8 the handled effect performed at the bottom, `T` too | E0420 | 1004 |

Controls that must stay rejected: n4 (`T` performed inside the handle, never handled) and
n7 (`get()` performed OUTSIDE any handle at `n == 0`): E0420 both, before and after.

## 1. Cause

A call pours the callee's row into the ambient by unifying tails. Inside a group of
mutually recursive functions a member's row is its own ambient `amb_f`, monomorphic. In
`handle { nest(n - 1) + get() }` the body's ambient is `{S | ρ}`; the recursive call
unified it with `amb_f`, so `amb_f` BECAME `{S | ρ}`: the handled effect entered the
function's own type, and every caller had to handle an effect that never escapes.

## 2. Options and choice

- (a) Inclusion constraints for every row (`ambient ⊇ callee`), PARKED's "rows unify by
  equality" question: a real inference change touching every call.
- (b) **Inclusion for exactly this shape (chosen):** a call of a member of the group being
  inferred, made DIRECTLY in a handle's body (not in a lambda inside it), whose row is not
  already the ambient's, records `body ⊇ member` -- labels only, flushed with the
  sub-effecting inclusions -- instead of unifying. The tail needs no constraint: the
  handle's residual (its body minus the handled effect) already flows into the
  function's ambient, which IS the member's row, so whatever the member's open part
  becomes reaches the body's tail through that equation.
- (c) Unify only the TAILS after copying labels: re-creates a cycle (`{L | ρ} = ρ`) when
  the residual flows back (reasoned, not built: rows of one tail variable with different
  labels conflict).

(b) is the stricter option of the two that work: it changes typing only where the
equality was provably wrong (a handled effect leaking into a recursive function's own
row), leaves lambdas and every other call exactly as they were, and keeps every label
the recursive call performs visible in the handled body (so a `resume` captured in a
clause still carries it).

## 3. Evidence

- `tests/handle_recursion.rs`: n1, n2, n3, n5 check clean and evaluate to 1000; n4 and
  n7 stay E0420. Natively (`native_codegen.rs`, `recursion_through_a_handle_body_runs_natively`):
  n1 (a chain of 1000 nested handler frames, each `get` walking to the innermost), n6
  and n8 agree with the evaluator.
- Negative control (`member_in_handle_body` always `None`): the check tests fail with
  E0420/E0423 as at `9eb8d5f`; the native test's programs no longer pass the front end.
- Gate: 907 passed, 87 suites (predicted 907) before the review; 909 after it.

## 4. The independent review

68 probes against this change and a `HEAD` worktree, evaluator and native compared on
every accepted one. Fixed, red first (`the_reviews_soundness_holes_are_rejected`):

- **F1 (high).** The labels-only inclusion skipped a label the body already had, so its
  type ARGUMENTS were never unified: a member performing `St(String)` hid under a handled
  `St(Bool)` -- accepted, the evaluator stopped on a type error, natively a silent wrong
  value. A shared label's arguments are now unified (`unify_effect_args`, reported once).
- **F2 (high).** A `resume` escaping in a lambda copies the body's labels when it is
  inferred; a member label arriving later (a `t()` after the handle) never reached the
  lambda -- accepted, then "unhandled effect `t`". An escaped resume now also includes
  the members the body includes, so a later label reaches it or is reported (E0423)
  against its closed row. This also rejects the reviewer's a03, which `HEAD` accepted and
  crashed on (a pre-existing hole of the same shape, reached there through an outer
  handler).
- **F3 (low, a regression by design).** A function annotated with the effect that used
  to leak (`fn f(n: Int) / {S} -> Int` whose `S` is all handled inside) is now E0423
  ("declared row includes an effect the body never performs"): the exact-row rule
  applied to the corrected row. Accepted, not changed.
- Minor, left: the E0426 multi-shot lint reads the body's labels when the handle is
  inferred, so it can miss a member label known only later (a best-effort lint).
