# Slice 5c-3 — an operation may only be called (E0206)

**Status:** approved (maintainer, 2026-10-05), over the alternative of first-class ops.
Stacked on 5c-2 (PR #5). Front end only.

## 0. The cut

Found by the 5c-2 review and parked. An op named anywhere but a call's callee checks
clean -- inference types it `Ty::Error` silently -- and fails at run time. Measured on the
5c-2 branch (`7646c7d`):

| # | Program | check | run |
|---|---|---|---|
| o1 | `let f = ping  f()` | ok | E0300 "unbound variable `ping`" |
| o2 | `apply(ping)` with `fn apply(g) { g() }` | ok | E0300 "unbound variable `ping`" |
| o3 | `let ping = fn() { 5 }  let f = ping  f()` (a local) | ok | runs, 5 |

## 1. Rule

**E0206 — an operation used as a value.** An op name that is not a call's callee, and not
shadowed by a local in scope (5c-2), is an error at the name, with help: wrap it in a
lambda, `fn() { ping() }`. Checked in `resolve`, where `Expr::Var` is already resolved; the
callee position of `Expr::Call` is exempt. Resolve errors stop the front end, so it
arrives alone.

The alternative (an eta-expanded, effectful closure) was declined: native refuses effectful
closure calls today, so it would add a second check/run/build disagreement.

## 2. Acceptance

| # | Criterion |
|---|---|
| D1 | o1 and o2 are exactly `[E0206]`, with the lambda help. |
| D2 | o3 still evaluates to 5 (a local is a value; no E0206). |
| D3 | Existing tests unchanged; full gate green; Windows CI green. |
| D4 | Control: E0206 off -> D1 reads `[]`. |

Plan (one task): red tests in `tests/name_rules.rs` (3 tests, both gate configurations:
+6, predicted 641 passed, 63 suites); implement; control D4; gate; independent review;
commit; PR stacked on #5.
