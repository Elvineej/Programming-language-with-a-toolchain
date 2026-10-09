# `resume` carries its handle's row — a front-end soundness fix

**Status:** done (2026-10-08). HANDOFF step 1, found by slice 5b-10 (PARKED).

## 0. Measured

| Program | `check` before | evaluator | after |
|---|---|---|---|
| a resuming lambda escapes T's handler and is called in `main` (PARKED) | ok | "internal: unhandled effect `t` reached the machine" | **E0420** |
| m10: the same lambda called where T is handled, return clause `fn(s) { x }` | ok | 15 | E0423 (see §2) |
| m10 with both clause values carrying T (`fn(s) { x + 0 * t() }`) | E0423 | 15 | ok; **compiles natively** (15) |
| state passing over the handled effect only | ok | 3 | ok |

Inference typed `resume(v)` effect-free: "its latent effect is the clause's ambient,
already threaded". True for a resume directly in a clause, false inside a lambda in one:
the lambda was typed pure although calling it runs the rest of the handled body.

## 1. The fix

`infer_handle` records ε = the handled body's row minus the handled effect; at each
`resume` the LABELS of ε, re-read at that point, are added to the ambient. Not its tail:
the first version used `add_row`, which unifies tails, and the independent review found it
merged the resume site's ambient with the body's -- a NEW hole (a direct resume tied a
function's row to a lambda that then closed it, dropping a later effect) and three
over-rejections. Each is a red test in `tests/resume_row.rs`, with controls C1-C3 recorded
there, plus a native corpus row.

## 2. Costs and gaps

- **Strictness (no sub-effecting, spec 3.6).** m10's two clause values now have different
  closed rows (`{T}` and pure) and do not unify. It checked clean before only because the
  lambda's row was wrong. Whether clause values should get sub-effecting is a language
  question for the maintainer.
- **Not covered (PARKED):** effects of the return clause and of clauses the resumed body
  re-enters; effects the body only relays through an open row. Both need the clause rows
  before the clauses are typed.
- The review also found pre-existing holes unrelated to `resume` (an effect dropped when
  added to a row closed early; `let` annotations ignored). PARKED.

Gate: 695 passed, 69 suites.
