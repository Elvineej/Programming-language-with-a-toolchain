# Exact replay and handler-based testing, in Elya

**Status:** done (2026-10-10). HANDOFF step 1 (ROADMAP priority 3, first route item: a
`Record`/`Replay` pair in Elya). Design choices are Claude's (rule 2).

## 0. Measured first

`examples/05_replay.elya` was written first, as the measurement, at `2e37b1c`:

| | evaluator | native |
|---|---|---|
| first draft, effects `Dice` and `Clock` | E0423 "a handler must cover a single effect" (x4) | -- |
| one effect `World { roll, now }` | runs: the four expected lines | "unsupported construct (Rem)" |
| after checked arithmetic (8080b9f) | same | "nested constructor pattern" |
| nested patterns flattened by hand | same | same lines, value 2106 |
| after pattern compilation (c684b0b) | same | same lines, value 2106, as written |

So the step needed two prerequisites, each its own slice and spec: `Int` arithmetic that
is exact or fails by name (with `/` and `%` natively), and native pattern compilation.

## 1. Design

### D1. The world is one effect; the handlers are ordinary Elya

`World { roll(sides), now() }`. A handler covers exactly one effect, every operation of it
(E0207/E0423), so a recorder records one effect; recording several effects is one recorder
per effect, nested. A recorder generic over ANY effect would need effect-polymorphic
handlers (a handler over "every operation of some effect `e`"), a language feature Elya
does not have; recorded in HANDOFF as the next design question for replay.

### D2. `record` builds the log as the computation returns

`World.roll(sides) -> { let v = roll(sides)  match resume(v) { Rec(x, log) -> Rec(x, Ent(Rolled(sides, v), log)) } }`.
The clause re-performs the operation (it runs outside its own handle, so the outer world
answers), resumes with the answer, and prepends the entry on the way out: the log is in
program order with no reversal and no state. Cost: a non-tail resume per answer, so the
recording holds one frame per answer until the run ends (O(answers) stack in the
evaluator's heap continuation and natively). Option rejected: state-passing (`fn(log)`),
which appends in reverse and needs a reversal; this is simpler and exact.

### D3. `replay` threads the unused log as the handler's answer

Each clause answers with a function of the log not yet used (`fn(l: Log) { .. resume(v)(rest) }`),
applied to the recording. Strict, by name:
- a roll answers only from a `Rolled` entry for the SAME number of sides; otherwise
  `Diverged(WantRoll(sides), l)`, naming the operation and argument the program asked for
  and the rest of the log;
- a clock reading answers only from a `Told` entry, else `Diverged(WantNow, l)`;
- a program that finishes with entries left is `Unconsumed(x, l)`, not `Replayed(x)`: the
  run was not the same one.

Options: a lenient replay (ignore the argument, accept leftovers) reproduces some runs it
should reject; controls R1 and R2 show each leniency is caught.

### D4. Tests swap handlers instead of mocking

`fixed` answers every roll with the highest face and every clock reading with 0. The
program under test is unchanged; `tests/replay.rs` runs the same `game` under `fixed`,
under `record` inside `live`, and under `replay`, for 20 seeds.

Scope, stated: the CLI flags (`elya run --record/--replay`) need real I/O effects and a
serialized log (strings natively), so they stay on the roadmap.

## 2. Tests

`tests/replay.rs` (5; the fifth, an order-swapped program, added after the review found
that no test asked in a different order than the recording), `tests/examples.rs::replay` (snapshot), `tests/crosscheck.rs`
(the tree-walker refuses it by name), native `the_replay_example_runs_natively`.

## 3. Negative controls

| control | observed |
|---|---|
| R1: replay answers a roll without checking its sides | `replay_names_the_first_divergence`: -1 (replayed to a score) for 806 |
| R2: replay accepts a log with entries left over | `a_replay_with_answers_left_over_is_not_the_same_run`: -1 for 3 |
| R3: the `now` clause answers from a `Rolled` entry | `replay_answers_each_question_only_from_its_own_kind`: -2 for 1000 |

Independent review: no correctness findings; the R3 gap and three stale comments, fixed.
