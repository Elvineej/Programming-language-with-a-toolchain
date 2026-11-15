# Plan: linear arrays, read by borrowing (self-hosting, part 2b)

Spec: `docs/superpowers/specs/2026-11-14-elya-arrays-and-borrowing-design.md`
(D1-D8). Base: `main` at 2b12eab.

## Measured first

The sum of every element read by index, natively (this container):

| n | a list's `nth` | `array.get` |
|---|---|---|
| 20,000 | 1.010 s | 0.004 s |
| 40,000 | 3.330 s | 0.006 s |

Twice the elements, 3.3 times the time for the list (quadratic); part 4's
parser reads tokens by position.

## Steps

1. The parser: `borrow` before a function declaration's parameter (a
   contextual word: `borrow` alone is still a name); `Param.borrow`.
2. The affine pass (`src/affine.rs`):
   - the resolver marks a use of a binding from outside the lambda or the
     operation clause it is in (`outer`);
   - `Modes`: which positions each callee borrows (`array.get`,
     `array.length`, `buffer.length`; a top-level function's `borrow`
     parameters, by name);
   - the counter skips a borrow (a borrowed position given a binding of the
     same body) and records it;
   - `Order`, a forward walk per body in evaluation order: what has been given
     away so far (a call's argument when the call is made, any other use where
     it is written, a lambda's captures where it is written), joined over the
     arms of an `if` and a `match`; a borrow of a binding already given away
     is `Facts.late`;
   - `Facts.given`: a `borrow` parameter with a use that is not a borrow;
     `Facts.borrow_values`: a function with `borrow` parameters used as a
     value.
3. The checker (`src/types.rs`): `Array(a)` a builtin linear type of one
   argument, its name reserved (E0211, `src/resolve.rs`), not data (E0436);
   the five builtins and `buffer.length` typed, pure; an array's element
   duplicable where the array is made (E0437); `late`, `given` and
   `borrow_values` become E0438 and E0439 where the type is not duplicable
   (through the marks generic code already has, E0437 at an instance).
4. The evaluator: `Value::Array(Rc<RefCell<Vec<Value>>>)`; `array_builtin`;
   both machines; `literal::show` refuses an array.
5. Native: `elya_array_new/push/set/get/length` in `runtime.c`; one new
   descriptor row after the continuation's, arity -1, which `gc_scan` reads as
   "`length` words from the fourth" (an array of heap values); an array of
   anything else carries the string tag; the element converted at the word
   boundary a constructor's field uses; `KIND_ARRAY` in the reflection table.
6. Controls, each reverted and compared byte for byte:
   - K1, the escaping handler's barrier removed: the escaping-continuation
     test fails.
   - K2, no late read recorded: the E0438 test fails.
   - K3, `gc_scan` not following a traced array's words: the collector test
     fails under stress.
   - K4, no element check where an array is made: the E0437 test fails.
   - K5, a call's arguments given away where written, not at the call: the
     acceptance test fails (`array.set(xs, i, array.get(xs, i) + 1)`).
   - K6, an evaluator push that copies: the linear-time test fails.
   - K7, no borrow modes for named functions: the acceptance test fails.
7. Review (ChatGPT, adversarial on D3 again); the gate; Windows CI on a
   `claude/*` branch.

## Review

ChatGPT's second adversarial review (maximum effort) found one hole, the same
in its Findings 1-5: a continuation that escapes its handler (a `resume` in a
lambda a clause returns, kept in an `Outcome`, an array, or passed through a
`borrow` function) runs the handled body's pending borrow after what follows
the `handle`. Run here, Finding 1 printed 99: the read saw the later write.
Fixed by D4's third case (a borrow of an outside binding in such a handler's
body is a use); all five are refused now (E0428, E0439 for a handler in a
borrowing function). Its 27 precision tests (shadowing, callees evaluated
first, constructors and operations taking the array after a read, pattern
bindings, generic borrows, multi-shot) all behave as it predicted; the
sharpest are in `tests/array.rs`. Its other recommendations (loan lifetimes
through continuations, once-callable function types) are the roadmap's
"continuation-aware ownership" line, after the switch.

## Tests

- `tests/array.rs` (new, 9): the program's output; out of range by name;
  reads accepted (before the consuming use, in a later argument of the call
  that consumes, in both arms, a `borrow` parameter read twice and passed on,
  `borrow` on a duplicable type, generic, a lambda run once); reads refused
  (E0438 in sequence, ChatGPT's four counterexamples, after an `if`, after a
  lambda took it, through a function value; E0428 in a clause and for a borrow
  in a lambda run twice; E0429); a `borrow` parameter only read (E0439:
  returned, consumed, grown, captured, stored, the function as a value twice;
  E0437 for a generic one at an array); an array used once, of no affine
  element, not data (E0428, E0437 three ways, E0436 twice, E0211, E0400, two
  type errors, main's value); linear time in the evaluator (80,000 elements
  within fourteen times 10,000's); an escaping continuation keeps no borrow
  (the review's round two, three refused, three accepted); borrows follow
  bindings, not names (the review's precision tests).
- `crates/codegen/tests/native_array.rs` (new, 4): the evaluator's output
  natively, also with `ELYA_GC_STRESS=1`; an array of strings kept through
  growths, a `set` and a thousand later allocations, a collection at each;
  out of range and main's value in the evaluator's words; linear natively
  (4,000,000 elements under 2 s, within ten times 1,000,000's).
- `crates/codegen/tests/native_boards.rs`: the same program on both boards (a
  thirteenth program in an existing test).

## Predictions

- Gate, configurations A and B: 1580 + 2 x 9 + 4 = **1602 passed, 177
  suites** (174 + 2 + 1: the root suite counts in both). First written as 7
  tests (1598): the review added two. Measured: 1602 passed, 177 suites;
  configuration C 8 passed, 2 suites. Configuration C: **8 passed, 2 suites** (unchanged:
  one more program inside an existing test).
- Existing expectations that change: the descriptor table's unit tests in
  `crates/codegen/src/lib.rs` (one more row after the continuation's).
