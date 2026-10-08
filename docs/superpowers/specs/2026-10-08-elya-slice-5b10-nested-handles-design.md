# Slice 5b-10 — nested handles and handles inside effectful code, natively (lift D17)

**Status:** done (2026-10-08). HANDOFF step 1 at the time. Lifts 5b-8's D17 and discharges the
PARKED entry "`elya_current_handler` depends on today's handle refusals".

## 0. Measured first (branch `claude/slice-5b10-nested-handles`, from 5b-9b's tip)

Every program checks and runs in the evaluator; values are the evaluator's
(`run_module_value`).

| # | Shape | `check` | evaluator | `build` today |
|---|---|---|---|---|
| m1 | a clause performs the effect its own handle handles; an outer handle answers (`inner` handles S, its clause does `resume(get() * 10)`) | ok | 51 | "calling convention disagrees with the callee" |
| m2 | a handle (T) nested in a handle body (S); the inner body performs S | ok | 51 | "handle nested inside another handle" |
| m3 | a handle that discharges everything, inside an effectful function | ok | 41 | "handle inside an effectful function" |
| m4 | a handle inside a clause body | ok | 15 | "handle nested inside another handle" |
| m5 | a clause resumes INSIDE a handle of its own (T), so the resumed body's `t()` goes to the clause's handler, not the outer one | ok | 8 | "handle nested inside another handle" |
| m6 | a handle whose clause performs an outer effect, in a function with a parameter | ok | 22 | "calling convention disagrees with the callee" |
| m7 | a non-tail `resume` in a clause that performs, and a second perform to the same handle | ok | 206 | "calling convention disagrees with the callee" |
| m8 | a return clause that performs an outer effect | ok | 31 | "calling convention disagrees with the callee" |
| m10 | an escaped continuation of a handle whose body performs an outer effect (state passing) | ok | 15 | "calling convention disagrees with the callee" |
| m12 | a leaking handle on every iteration of a tail loop | ok | 3002 (N = 1000) | "calling convention disagrees with the callee" |
| m13 | after a leaking handle finishes, the next perform of ITS effect goes to the outer handler | ok | 111 | "calling convention disagrees with the callee" |
| m9, m11 | recursion through a handle body (`nest -> handle { nest(n - 1) + get() }`, direct and via a helper) | **E0423 / E0420** | 1000 | front end |

m9/m11 are a front-end finding, not this slice's: inference rejects recursion whose call
sits in a handle body that discharges an effect, though the evaluator runs it. Parked.

"Calling convention disagrees" is D17 seen from the other side: `contains_effect` says a
`handle` contributes nothing to its region, so a function whose handle LEAKS an effect (its
clause, return clause or body performs something the handle does not handle) was compiled
direct while its type said effectful.

## 1. The model

A handle whose row names no user effect ("non-leaking") is a **direct** handle, exactly as
in 5b-8: a native nesting call whose return clause and clauses return the answer natively.
It may now appear anywhere -- inside another handle, a clause, or an effectful region --
because nothing inside it can perform past it.

A handle that leaks is a **CPS** handle, and appears only in a CPS region (its effects are
its region's). It is a continuation site of its region:

- **Frame.** Every handler frame is `[tag][code = return clause][next][table][parent][saved..]`.
  `parent` (new, traced) is the handler that was current when the handle was installed; a
  perform that finds no clause in a frame's table moves to its `parent`. `next` is null for
  a direct handle and the handle's continuation for a CPS one.
- **Install.** Build a site frame for the rest of the region (or take the region's
  continuation in tail position) as `next`, store `parent = current`, make the frame current,
  and `musttail` into the body.
- **Clauses run outside their handle** (the PARKED requirement): every clause, direct or CPS,
  sets the current handler to its frame's `parent` on entry. A CPS clause is a CPS region
  whose continuation is the frame's `next` as of the perform; a CPS return clause is one whose
  continuation is `next`, and it too runs with `parent` current.
- **Perform.** Walk from the current handler through `parent`s to the first frame whose table
  has the op; the continuation object is `[tag][k][h][innermost]`: `k` the chain, `h` the frame
  that handles the op (word 2, not traced -- `h` is a frame on `k`'s chain, so `k` keeps it
  alive), `innermost` the handler current at the perform (traced). `h` replaces the one-shot
  flag: 0 once consumed. The descriptor row is unchanged (`[3, 0b101]`).
- **Resume.** Check and clear word 2, then re-install `h` *at the resume*: `h.parent =` the
  current handler, and for a CPS handle `h.next =` the resume's own continuation (a site frame
  for the rest of the clause, or the clause's continuation in tail position). Then make
  `innermost` current and jump. A direct handle's resume is the 5b-8 nesting call, now also
  setting `h.parent`.

Rebinding `next` at every resume is what makes a CPS handle deep: a second perform to `h`
reaches a clause whose continuation is the first clause's remainder. Rebinding `parent` is
m5: performs the resumed body does not handle go to the handlers around the RESUME.

Which handle a `resume` belongs to is lexical (the innermost enclosing clause binds
`$cont`), so whether a resume is CPS is known statically: `cps::Fx`. Leaking is computed
structurally -- the user effects a region performs or calls, a handle subtracting the effect
its clauses name, a resume contributing its handle's leaks -- to a fixpoint. The convention
checks already in place refuse any disagreement with the types by name; a CPS handle or
resume that reaches direct code is refused by name too.

## 2. Predictions

- `NESTED_HANDLES`, value and evaluator differential, rows m1-m8, m13 and a three-level
  dispatch row: every row red first with today's refusal, green with the values above.
  m10 was predicted green and is not: see §4.
- m12 at N = 1 000 000, natively: 3 000 002, bounded stack.
- A `CPS_ROOTING` row: heap values saved in a CPS handler frame and live in a CPS clause
  across a 30,000-cell build.
- The two 5b-8 unit tests that pin D17's refusals are replaced: the same programs now compile.
- `gc_mark` unchanged (637).

## 3. Controls (each reverted, each failing differently)

K1 dispatch stops at the current handler; K2 clauses run with their own handle current;
K3 resume leaves `parent`; K4 resume leaves `next`; K5 the tail-resume install in
`clause_tail` removed (A6's finding: no test isolated it); K6 a CPS return clause leaves its
handle current. Each failed differently; the footprints are recorded on `NESTED_HANDLES`.

## 4. Findings

- **m10 stays refused: `resume` is typed effect-free.** Probing the Core types showed the
  lambda `fn(s) { (resume(s))(s) }` typed `fn(Int) -> Int` with an empty row, though calling
  it runs the rest of a body that performs T. So its body is direct code and the CPS resume
  inside it is refused by name ("resume of an effectful handler in direct code", pinned by
  `an_escaped_resume_of_a_leaking_handle_is_refused_by_name`). The same gap is a front-end
  SOUNDNESS hole: a program that checks clean stops in the evaluator with "unhandled effect
  `t` reached the machine" (PARKED). Fixing it is the next HANDOFF step.
- **Partial handles (the independent review).** The front end accepts a handle with clauses
  for some of an effect's ops and types it as discharging the whole effect; at run time the
  other ops go to an outer handler. `Fx` subtracted the effect name too, so such a handle was
  compiled DIRECT, and the new parent walk then let an outer clause capture a continuation
  ending at the inner direct frame -- silent wrong values (2220 for the evaluator's 1220;
  before this slice, a named stop). Fixed: a handle handles an effect only if it has a clause
  for every op of it the module performs, so a partial handle leaks the effect and is a CPS
  handle; where the types disagree (a function typed pure) the convention check refuses it by
  name. Three corpus rows and a refusal test, red first. The front-end side (a partial handle
  checks clean) is parked.
- **Recursion through a handle body** is rejected by inference (m9/m11); parked.
- Direct return clauses now also run with their frame's `parent` current (the review: they
  ran with their own frame current, harmless only while nothing in them performs past it).
