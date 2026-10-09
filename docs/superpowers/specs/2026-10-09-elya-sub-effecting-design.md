# Sub-effecting for function values

**Status:** done (2026-10-09). HANDOFF step 1. Decided by the maintainer (2026-10-08:
"Yes, for function values"); the design choices below are Claude's (rule 2).

## 0. Measured first

Rows unify by EQUALITY (spec 3.6), so a pure function could not stand where an effectful
one is expected, and the sweep of 2026-10-08 made the consequence visible as E0423. On a
probe set of about 840 programs (the corpora, every review's probes), before this slice:

| Program | `check` | evaluator |
|---|---|---|
| `if n == 0 { fn(s) { s } } else { fn(s) { s + go(n - 1) } }` under a handler | E0423 | 6 |
| `fn w(k) { k(0) + lg(1) }` called with a pure `k` | E0423 | 1 |
| m10 (5b-10): clause values `{T}` and pure joined in the handle result | E0423 | 15 |
| the review's c4ok (relay clause values) | E0423 | 24 |
| the review's c8 (a resumed body inside a clause handler) | E0423 | 12 |
| `relay_plus_own_effect_rejects_pure_callback_known_limitation` (effect_closures) | E0423 (pinned) | ok |

## 1. Options

- (a) Keep equality. Sound, strict; all of the above stay rejected.
- (b) **Upcasting by row polymorphism (chosen).** Koka's open/close discipline: a closed row
  in a covariant position becomes `{ε | ρ}` with a fresh ρ, so unification can instantiate
  ρ to whatever the context expects. No subtyping relation in the checker.
- (c) Inclusion constraints everywhere (`ε ⊆ ε'`), solved at generalization. Most precise,
  a rewrite of inference.

(b) is chosen because it fits the existing unifier, keeps printed schemes unchanged, and
can be checked against the evaluator program by program. Where it needs an inclusion
(below), it records one narrowly instead of rewriting inference.

## 2. Design

- **Phantom tails.** The fresh tails that opening adds are "phantom" (`Infer::phantom`).
  They only say that the value may be used at more effects. A meaningful row variable bound
  to `{.. | w}` makes `w` meaningful (`bind_row`). What remains of a phantom tail after
  absorbing labels is still phantom. An instantiation copies phantom-ness.
- **Where opening happens.** A lambda literal's TYPE gets a phantom tail when its body's
  row is closed; the body's own ambient stays closed, so nothing leaks in. A VALUE use of a
  name opens every closed row in a covariant position (`open_covariant`: the function's
  own row and rows in its result, never parameter rows). A callee is not a value use.
- **Calls.** The call's row starts phantom. If the callee's row ends in a phantom tail that
  no parameter type in scope shares, the call does NOT unify the two rows: that made the
  callee's use equal to the caller's whole row and merged rows related only by inclusion.
  It records `ambient ⊇ tail` (`pending_incl`) instead. The inclusions are flushed, labels
  only and to a fixpoint, before a lambda's ambient closes and when the group is done. A
  label that meets an ambient already closed is a surfaced E0423, never dropped. A tail
  shared with a parameter type is a RELAY and is unified as before, because generalization
  needs the shared variable.
- **Closing.** A phantom tail that nothing bound is closed when its SCC group is done, so
  schemes print as before and Core sees closed rows.

## 3. Native

A lambda's convention is fixed by its node type. A pure lambda joined with an effectful
one gets the joined (effectful) node type and compiles with the CPS convention, which is
consistent. A generalized, direct local USED where an effectful function is expected is
not: the CPS call jumped into direct code. Measured: `if c { f } else { fn(x) { t() + x }
}` printed 2 natively where the evaluator printed 13. The native prepass now refuses it by
name ("direct function used where an effectful one is expected") until an adapter closure
wraps it (HANDOFF). Programs the front end newly accepts that use effect-polymorphic
functions at user effects stay refused by D16's existing name (N7).

## 4. Evidence

On the same probe set, after this slice, no program changed from accepted to rejected,
and every newly accepted program runs in the evaluator. The only programs that check clean
and then hit "unhandled effect" are the two parked before this slice (resume's row lacks
return-clause effects). Expected outcomes changed, all approved under rule 2:
- `relay_plus_own_effect_rejects_pure_callback_known_limitation` is now
  `relay_plus_own_effect_accepts_a_pure_callback`;
- m10's resume-row test now checks and runs;
- `nonvalue_bound_relay_row_stays_monomorphic` accepts E0420 as well as E0423 (the guard
  is still a rejection);
- the row-soundness `if`-join witness is E0420 instead of the closed-row E0423 (the effect
  is no longer closed early at all).

Tests: `tests/sub_effecting.rs` (seven tests, with controls), the native `SUB_EFFECTING`
corpus (value and differential), and the refusal test.

## 5. The independent review

Three verified findings, each a red test first, then fixed:
- **F1, a hole this slice opened.** A call of a group member's RESULT recorded `ambient ⊇
  tail`; the tail later aliased a parameter's row (a meaningful variable), and the
  labels-only flush never relayed it: `f` was typed pure though it calls `k`, and an
  unhandled `lg` passed `check`. Fix: the flush unifies the ambient with a recorded tail
  that has become meaningful.
- **F2, an over-rejection.** The same tail, once free in a member's parameter types, stayed
  phantom and was closed pure at the group's end, forcing every `k` pure (base accepted
  the program). Fix: at step 2.4 such a tail stops being phantom.
- **F3, a native miscompile.** The upcast refusal tracked `let` and parameter types but not a
  handler's return binder; a direct closure reached through it was called as CPS (2 for
  102). Fix: the return binder carries the handled body's type.

After the fixes, over 757 probe programs (the corpora and every review's probes): no
program went from accepted to rejected against base; every newly accepted one runs in
the evaluator; the only accepted programs that stop on an unhandled effect are the two
parked before this slice; and of the 464 accepted programs, every one that builds natively
prints the evaluator's value.

## 6. Not covered (PARKED)

- Recursion through a lambda that the recursive function then handles around
  (`tests/row_soundness.rs` `go` under a handler) still needs inclusions for the group's
  own row.
- A lambda forwarding an ENCLOSING function's parameter still forces that parameter pure.
  Keeping it open (tried) makes the enclosing function effect-polymorphic, which natively
  is N7's refusal: it is deferred to N7.
- The adapter for direct closures used at effectful types.
