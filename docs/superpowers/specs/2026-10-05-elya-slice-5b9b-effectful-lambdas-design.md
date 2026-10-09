# Slice 5b-9b — effectful lambdas and closure calls, natively

**Status:** done (2026-10-05). HANDOFF step "5b-9b"; completes 5b-9 with 5b-9a (match).

## 0. The cut

Measured at `main` (5b-9a spec §0): s1, s2, s5, s6 -- every lambda whose row names a user
effect -- were refused by `elya build`, "effectful lambda (not yet compiled natively)",
and every call of such a closure "effectful closure call". The evaluator ran them all. s4,
an effect-POLYMORPHIC function used at a user effect, is N7's and stays refused (D16).

## 1. Design

- **Convention.** An effectful lambda's lifted function is `(closure, params.., k) -> i64`,
  the CPS convention of an effectful top-level function with the closure first. MAX_PARAMS
  (5) leaves three source parameters; a fourth is refused by name, "effectful lambda takes
  more than three parameters". `LambdaSite::effectful` = `needs_cps` of the lambda's type.
- **Body.** `emit_cps_lifted`: the body is a CPS region that CONTINUES its enclosing
  scope's binding indices (`cps::LambdaRegion`, recorded by the same walk that numbers
  sites). Each capture is placed at the index of the innermost binding of its name there --
  the index every site inside the body saved it under -- and the parameters are bound from
  the region's depth on. D17's handle refusals apply inside it.
- **Calls.** A call of an effectful closure from a CPS region is an indirect `musttail`
  jump `code(clos, args.., k)`: `k` is a fresh site frame when non-tail (the closure rooted
  with the arguments while the frame is allocated) or the region's continuation in tail
  position -- in `expr`, `tail`, and on a resumption's path.

## 2. Evidence

`EFFECTFUL_LAMBDAS` corpus, value + evaluator differential: 9 rows predicted before the
first run, all red first with the refusal; 2 rows added because controls K1 and K4 PASSED
on the first nine (a capture whose binding index differs from its closure slot; a closure
call on a resumption's path). A `CPS_ROOTING` row: a captured heap list survives a
30,000-cell build between calls (70,017). A million-iteration loop through a tail effectful
closure call, native (7,000,000). The 5b-8 unit test that pinned the refusal is replaced:
the same program now compiles, and the four-parameter refusal is pinned. `gc_mark`
unchanged.

## 3. The review's regression, caught before commit

An effect-POLYMORPHIC local (`let app = fn(g) { g() + 1 }`: open row, so its lifted body
is DIRECT) called at a user effect (`app(fn() { ask() })`) was compiled as a CPS jump into
a direct function -- native SIGSEGV where the evaluator printed 8. Before this slice the
program was refused by name; the new call path made it reachable. A closure's convention is
fixed at its definition, a call site picks by the use's instantiated type, and only a
generic binding lets them differ. D16 already refused this for top-level functions; the
pre-pass now tracks local binding types (`check_local_conventions`) and refuses the same
case for locals, by the same name. Pinned red first by
`a_let_bound_effect_polymorphic_lambda_used_at_a_user_effect_is_refused`. Three of the
review's verified probes became corpus rows and one a GC row (collections inside a lambda
body between sites). Gate: 672 passed.
