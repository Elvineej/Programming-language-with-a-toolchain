# `resume` carries relayed, return-clause and re-entered-clause effects

**Status:** done (2026-10-09). HANDOFF step 1 (soundness). Closes the three gaps the
resume-row fix of 2026-10-08 left PARKED, and the parked "forwarding lambda" over-rejection
of sub-effecting, which stood in the way. Design choices are Claude's (rule 2).

## 0. Measured first

At `1a59091` (async step 1). Over the probe programs of the last three reviews (268
programs), these check clean and then stop in the evaluator with "unhandled effect"
(natively: refused by name, or the runtime's "no clause" guard):

| Shape | gap |
|---|---|
| `fn task(body) { handle { body() } with { Y.yld() -> K(fn() { resume(Unit) }) .. } }` (min1, min2, min4, t12, t13, t26-t31) | the body RELAYS `body`'s row: only labels reached the resume |
| `return(x) -> D(x + lg())` beside `Y.yld() -> K(fn() { resume(Unit) })` (min3) | the RETURN clause's effects |
| `S.get() -> { let y = t()  fn(s) { (resume(s + y))(s) } }` over `get() + get()` | a clause the resumed body RE-ENTERS |

And the over-rejection: `fn wrap(f) { fn(x) { f(x) + 1 } }` used at `{L}` is E0423, because
a lambda's row was closed unless relayed through ITS OWN parameters, so forwarding the
enclosing `f` forced `f` pure (PARKED by sub-effecting, "deferred to N7").

## 1. Design

Under deep handlers, `resume(v)` runs the rest of the handled body, which may re-enter any
clause of the handle, and ends in the return clause. So the row of a lambda that resumes
must include: the body's effects (minus the handled one), every clause's effects (minus the
handled one), and the return clause's effects.

- **Relays are unified, other tails are not.** The first resume-row fix added the body's
  LABELS only, because unifying with the body's tail made the resume site's ambient EQUAL
  to it and merged rows related only by inclusion (it opened a hole). Kept. But when that
  tail is a RELAY -- a row variable of a parameter in scope -- it is unified, exactly as a
  call through that parameter would be (`task(body)` then has `body`'s row in the stored
  lambda's row, and a field that admits less rejects it, E0423).
- **The return clause is typed first**, in an ambient of its own, closed unless relayed
  (a region, like a lambda body), then poured into the handle's ambient. Its labels reach
  every resume. (Typed after the clauses, they would arrive after a lambda in a clause had
  closed its row.)
- **Each clause is a region of its own** (`clause_amb`), typed, flushed, closed unless
  relayed, then poured into the handle's ambient: so each clause's effects are known
  apart.
- **An escaped resume takes the handle's CLAUSES ROW as a tail.** A resume inside a
  lambda (deeper in lambdas than its clause) ESCAPES: its ambient's tail is unified with
  one fresh row per handle, `clauses_row`, which `infer_handle` fills with every clause's
  labels (minus the handled effect) once all clauses are typed, then closes. Until then
  that row is neither closed (`close_unrelayed_residual`) nor quantified by a `let`
  (`generalize`), so a lambda that holds it -- or a nested handle, or a lambda around
  either -- keeps receiving the labels. (The first version kept the LAMBDA's row open
  instead; the review found two holes in that, §3.)
- **A lambda's tail stays open when it relays ANY parameter in scope**, not only its own:
  `fn(x) { f(x) }` inside `wrap(f)` relays `f`'s row. This is what keeps `task(body)`
  usable at all: the stored lambda relays `body`. Natively the resulting effect-polymorphic
  results compile since N7 part 1 (`wrap` gets a clone at `{L}`).

Rows still unify by equality, so a relay is shared both ways: a stored lambda that relays
`body` and also performs `z` puts `z` into `body`'s row and so into `task`'s (r5 below:
rejected, E0420, where the evaluator, with `Z` handled only around the call of the lambda,
printed 2). This is the existing discipline for any function that calls its parameter and
then performs something; precise inclusion constraints would accept it (PARKED, the
language question on row equality).

## 2. Evidence

- Gate: 830 passed, 79 suites (predicted 830: seven new root tests, twice; one new
  `CONVENTIONS` row); 836 after the review (three more root tests).
- `tests/resume_row.rs` (six new: relay, return clause, returned lambda, re-entered
  clause, and two controls that check and run), `tests/sub_effecting.rs` (the `wrap`
  acceptance), and natively `a-returned-lambda-relays-an-enclosing-parameter` (11072).
- Over the 268 probes, against `1a59091`: the 15 programs that checked clean and then hit
  "unhandled effect" are now rejected at `check`; newly accepted: `wrap`, `compose`-like
  forwarding (q1, q9, q10, p16, a14, f06, f07), each running to the evaluator's value (and
  natively, where no type variable is involved); newly rejected besides the unsound ones:
  t32 (a stored lambda typed `/ {Z}` that performs `L` -- the field's row was wrong, so
  the rejection is right) and r5 (above, the equality over-approximation). One diagnostic
  changed (t25: E0401 infinite type -> E0400 mismatch, still rejected).
- Negative controls, each reverted (`cmp`):
  - C1, a relayed tail not unified at the resume: the two relay tests check clean;
  - C2, the return clause's labels not added at the resume: the return-clause test checks
    clean;
  - C3 (first version), an escaping lambda closed at its own end: the re-entered-clause
    test checks clean;
  - C4, a lambda's relay check with its own parameters only: the relayed control is
    rejected again (E0423).

## 3. The independent review

About 120 adversarial programs plus the 275-program corpus, against the baseline:
- **F1 (unsound, in the first version).** A `let`-bound resuming lambda
  (`let k = fn() { resume(Unit) }  K(k)`) was generalized while its row was still open,
  so the clause labels added later never reached its uses: `check` clean, "unhandled
  effect `lg`".
- **F2 (unsound, in the first version).** The escape was recorded on the resume's
  immediate ambient; inside a nested handle in the lambda that is the nested handle's, so
  the lambda closed at its end.
- Both fixed by the clauses row (§1); pinned by
  `a_let_bound_resuming_lambda_carries_the_clauses_it_re_enters` and
  `a_resume_in_a_nested_handle_inside_a_lambda_carries_the_clauses_it_re_enters`.
- **F3 (over-rejection).** A LOCAL lambda forwarding a parameter (`let k = fn(x) { f(x)
  }`) now shares `f`'s row, so its uses under different handlers meet in `f`'s row
  (reg1: E0420, the baseline printed 113). Calling `f` directly there was already
  rejected the same way; before, `k` was closed at the lambda (forcing `f` pure), which
  accepted this but rejected every `wrap(f)` used at an effect. Kept, pinned as
  `a_local_lambda_forwarding_a_parameter_shares_its_row_known_limitation` (the
  equality-vs-inclusion question, PARKED).
- After the fixes, over all ~390 probes: no program checks clean and then stops on an
  unhandled effect (the only evaluator errors left are one-shot violations, a run-time
  check both backends make); the newly rejected programs are the unsound ones, ones whose
  field rows were wrong (t32, p04, p11, q05, om2), and the F3/r5 equality class.

## 3. Not covered

- Rows unify by equality (above).
- Recursion through a handle body (PARKED) is unchanged.
