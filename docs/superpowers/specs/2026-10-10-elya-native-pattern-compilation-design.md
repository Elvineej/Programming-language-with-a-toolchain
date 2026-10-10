# Native pattern compilation: nested and literal patterns

**Status:** done (2026-10-10). A prerequisite of HANDOFF step 1 (replay): the
example's replay handler matches `Ent(Rolled(s, v), rest)`. Design choices are Claude's.

## 0. Measured first

At `6b6307d`. Every program checks clean and runs in the evaluator; `elya build`:

| # | shape | native today |
|---|---|---|
| p1 | two-level nested constructors, falling through to later arms | "nested constructor pattern" |
| p2 | `match n { 0 -> .. 1 -> .. k -> .. }` and `match b { True -> .. False -> .. }` | "match scrutinee is not an ADT" |
| p3 | `Some(0)` before `Some(n)` (a literal inside a constructor) | "literal pattern" |
| p4 | `P(x, Q(1))` failing, then an arm that reads an OUTER `x` | "nested constructor pattern" |
| p5 | nested patterns in an effectful function whose arms perform | "nested constructor pattern" |
| p6 | `Cons(a, Cons(b, rest))` in a tail-recursive loop, 100 001 elements | "nested constructor pattern" |
| p7 | a literal pattern inside a lambda | "literal pattern" |
| r | `examples/05_replay.elya` (with nested patterns flattened by hand it builds and prints the evaluator's lines, value 2106) | "nested constructor pattern" |

Both emitters (`lower_match` and the CPS `match_dispatch`) handle exactly one shape: an ADT
scrutinee, constructor patterns whose arguments are variables or wildcards, and a final
variable or wildcard.

## 1. Design

### D1. A Core-to-Core pass compiles patterns before either emitter runs

`crates/codegen/src/patterns.rs`, run right after N7's `specialize`: every `Match` whose
arms are not all flat is rewritten into flat matches, `if`s and `let`s. Both emitters, the
closure collector and the CPS analysis then see only shapes they already support, so no
emitter changes.

Options: (a) teach both emitters nested tests -- two copies of the same algorithm in two
emitters that must not drift (the reason `prim_values` is one table); (b) compile in Core
lowering (the front end) -- would change what the evaluator's tests see of Core and every
Core snapshot, for a back-end need; (c) **a codegen Core-to-Core pass (taken)**, the
`specialize.rs` pattern: one place, invisible to the front end.

### D2. Test, then destructure: every arm and its continuation appear once

*Revised after the independent review; the first version is recorded below.*

`expand(s, arms)`, top to bottom:
- a run of consecutive FLAT constructor arms becomes one flat `match s { .. ; _ -> rest }`;
- a wildcard is its body; a variable `x` is `let x = s in body` (later arms are dead);
- any other arm (a literal, or a constructor with nested sub-patterns) becomes
  `if <test> { <bind; body> } else { rest }`, where `rest` is the expansion of the arms
  below it. `<test>` is a pure `Bool` expression over fresh names -- nested flat matches
  and `if`s answering the literal `False` at every point where the pattern can fail -- and
  `<bind>` destructures the (now known to match) value with one-arm flat matches, binding
  the user's variables.

So `rest` appears exactly once per arm and the output is linear in the size of the match
(pattern sizes plus bodies). The cost, stated: a matching nested arm reads the fields it
tests twice (once to test, once to bind) -- loads, never re-evaluation (the scrutinee is
bound once).

**The first version, and why it went.** It backtracked by duplication: each failure point
of an arm continued with a deep copy of the expansion of the arms below. The review
measured it: size(n) ~ (k + 1) size(n - 1) for k failure points per arm; 6 arms of three
nested literals took 11.4 s and 6.3 MB of code, and an ordinary 11-arm expression
simplifier 9.2 s and 6.4 MB (both now 0.1 s, 22-26 KB, corpus rows p8 and p9). The spec's
"O(arms x sub-patterns)" was wrong. Options weighed for the fix: join points
(`let rest = fn() { .. }`, which turns an effectful continuation into an effectful closure
call and allocates per match), a Maranget decision tree (no re-reads, but its own body
duplication and a column heuristic), and test-then-destructure (**taken**: linear, no
copies, no closures, and every node in the output appears once by construction -- which
also retires the deep-copy rule the first version needed, since the CPS analysis keys on
node addresses).

### D3. User binders are bound only on success

Every field is bound to a fresh `$pN` name (not writable in source); a user variable is
bound by `let x = $pN` only after every test in its arm has passed. Binding early would let
a failed arm's binder shadow an outer name that a later arm reads (p4: `P(x, Q(1))` fails
and the next arm returns the OUTER `x`). Negative control K1 binds early and p4 changes.

### D4. Exhaustiveness is the front end's; a residual failure still traps by name

Where every arm has been tried on an ADT value, the expansion ends in a flat match with no
catch-all, whose fall-through is `elya_match_fail` as today -- never undefined behaviour.
A non-ADT value (`Int`, `Bool`) has no trap block, so a literal arm with no arm and no
fallback below it is taken unconditionally. That is sound because the checker proved the
match exhaustive: a `Bool` match covers both values, and an `Int` match needs a catch-all,
so its last arm is never a literal. A unit literal `()` always matches (a wildcard).

### D5. No node appears twice

The CPS analysis and the closure collector identify Core nodes by address, so the output
must never contain one node in two places. The first version needed deep copies for that
(found on p5, "saved binding is not available"); test-then-destructure places each arm
body and each continuation once. Pinned by the unit test `no_node_is_shared_in_the_output`.

## 2. What does not change

The evaluator, Core lowering and every existing native expectation. A function
with no non-flat match is not rebuilt at all, so no existing program's Core changes.

## 3. Negative controls

Each applied to the final version, run, and reverted (`cmp`).

| control | observed |
|---|---|
| K1: bind the user's variables before the test (`bind(if test {body} else {rest})`) | p4: 505 for 509; p1 traps "match failed" |
| K2: sub-patterns' tests always `True` | p4: 505 for 509 |
| K3: swap the `if`'s branches | p4 traps; the corpus fails |
| K4: the first version (copying continuations) | `a_compiled_match_grows_linearly_with_its_arms`: the test process was killed (out of memory at 16 arms) |

## 4. Results

Gate 873 passed, 83 suites. The replay example (`examples/05_replay.elya`, nested
patterns as written) builds and prints the evaluator's lines and value natively.
