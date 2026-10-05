# Slice 5b-9a — effectful code inside a `match`, natively

**Status:** done (2026-10-05). The first half of HANDOFF step 2 (5b-9); 5b-9b, effectful
lambdas and closure calls, follows.

## 0. The cut

Measured at `main` (`e63ec31`): every program with an effectful call inside a `match`
checked and ran in the evaluator, and `elya build` refused it, "effectful call inside a
match (not yet compiled natively)". Two of seven measured shapes (s3, s7); the others are
5b-9b's (effectful lambdas) and N7's (s4, an effect-polymorphic function at a user effect).

The analysis already handled `match` (`cps.rs`: sites through arms, pattern binders in
scope, saves for a scrutinee site). Only the emitter refused, in three places.

## 1. Design

One routine, `Cx::match_dispatch`, the effect-aware twin of the direct emitter's match:
the same tag tests, field loads and `elya_match_fail` trap, with each arm lowered by the
CPS folds (`expr` for a value, joined in a phi; `tail` in tail position). Pattern binders
are bound through `St` in `pat_binders` order, so a site inside an arm finds them at the
binding indices the analysis recorded: saved in its frame, traced by the collector.

- `expr`/`tail`: evaluate the scrutinee (it may itself be a site), then dispatch.
- Resumption: a site in the scrutinee (slot 0) dispatches on the value it returned; a site
  in an arm (slot >= 1) carries the arm's value up -- the match's value -- as `If` does.

Still refused, by name, as in the direct emitter: nested constructor and literal patterns,
a non-ADT scrutinee, parametric ADTs.

## 2. Evidence

`MATCH_EFFECTS` corpus, value + evaluator differential: 6 rows predicted before the first
run (tail arm, value arm, binders across a site, scrutinee site in value and tail position,
variable and wildcard arms), all red first with the refusal; and 6 more from the
independent review (a scrutinee site whose arms use an outer local, scrutinee site then
arm binders, a binder shadowing an outer name, a Bool-valued match, all arms ending in
sites, an arm after a catch-all). Two `CPS_ROOTING` rows keep heap binders live across a
site while collections run (a constructor binder, 4000 deep, 8,030,000; a variable binder,
30,057). Three controls, each failing differently (recorded on the corpus). `gc_mark`
unchanged (637 bytes).

The review also found a PRE-EXISTING bug in the direct emitter: an arm after a catch-all
(only an E0431 warning) made `elya build` fail LLVM verification. Fixed (stop after a
terminal arm, as `match_dispatch` does), pinned by
`a_match_arm_after_a_catch_all_compiles_natively`, red first. Gate: 650 passed.
