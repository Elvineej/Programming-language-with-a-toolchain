# Effects dropped on a closed row — a front-end soundness fix

**Status:** done (2026-10-08). HANDOFF step 1 (the soundness sweep), first item.

## 0. Measured

`add_effect`/`add_row` return a `RowConflict` when the ambient's row is already closed;
all six callers (a perform, `io.println`, a call, a handle's residual, a handle's seed, a
resume) discarded it, so the effect vanished from the type.

| Program | `check` before | evaluator | after |
|---|---|---|---|
| `go`: a lambda calls the recursive `go`, then `go` performs `lg` (unhandled) | ok | unhandled `lg` | E0423 "already closed" |
| `w`: an `if` joins `k` to a pure lambda, then `lg` (unhandled) | ok | unhandled `lg` | E0423 |
| `go` under an L handler | ok (L dropped) | 64 | E0423 (cost) |

## 1. Fix

`surface_conflict` reports the conflict as E0423 at the perform/call. No corpus program
changes result (the gate is unchanged apart from the new tests).

## 2. Tried and reverted

Leaving a lambda's row tail open when it is free in the environment (so `go` keeps L
instead of closing early). The independent review found two regressions on programs base
accepted soundly: a recursive function that handles its own effect around a call of a
lambda that recurses (the handled effect leaked into its row), and a forwarding lambda in a
function that performs its own effect (the effect was forced onto the forwarded parameter).
Rows unify by equality; accepting both kinds needs sub-effecting: a language question
(PARKED). The three programs are regression guards in `tests/row_soundness.rs`.

Gate: 709 passed, 71 suites.
