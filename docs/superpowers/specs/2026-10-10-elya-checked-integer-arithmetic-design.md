# Integer arithmetic is exact or fails by name (and `/`, `%` natively)

**Status:** done (2026-10-10). A prerequisite of HANDOFF step 1 (replay): the
example's random-number generator needs `%`, which native code refuses. Design choices
are Claude's (rule 2).

## 0. Measured first

At `2e37b1c`, `elya run` (debug build, as the tests run it) and `elya build`:

| program `main` | evaluator | native |
|---|---|---|
| `7 / 0` | E0300 "division by zero", exit 1 | refused: unsupported construct (Div) |
| `7 % 0` | E0300 "remainder by zero", exit 1 | refused (Rem) |
| `MIN / (0 - 1)` | **Rust panic**, exit 101 | refused |
| `MIN % (0 - 1)` | **Rust panic**, exit 101 | refused |
| `MAX + 1` | **Rust panic** (debug), wraps (release) | wraps to MIN |
| `(0 - 7) / 2`, `(0 - 7) % 2`, `7 % (0 - 2)` | -3, -1, 1 | refused |

(`MIN` is written `0 - 9223372036854775807 - 1`, `MAX` is `9223372036854775807`.)

So the reference semantics crashes the host on four inputs and disagrees with itself
between debug and release builds, and native wraps silently. 5b-1 §11 and 5b-2 deferred
this ("integer overflow: still unreconciled"); the replay example forces it.

## 1. Design

### D1. `Int` arithmetic is exact or fails by name

`+`, `-`, `*`, `/` and unary `-` on `Int` either produce the mathematically exact result,
which fits in 64 bits, or stop the program with a named error: E0300 "integer overflow" in
the evaluator, `elya: integer overflow` on stderr and exit 1 natively. A zero divisor
keeps its existing names ("division by zero", "remainder by zero"). Division truncates
toward zero, and the remainder takes the dividend's sign (`a == (a / b) * b + a % b`), as
both evaluators already do. `MIN % -1` is 0: exact, so not an error.

Options:
(a) two's-complement wrapping everywhere -- what native does and what a release
evaluator happens to do; silent wrong answers, and the evaluator would need `wrapping_*`;
(b) **exact or a named error (taken)** -- the stricter one: no program observes a wrapped
value, the evaluator's debug/release split disappears, and every disagreement becomes a
failure on both sides by the same name;
(c) arbitrary precision -- exact for every input, but a heap-allocated number natively for
every `Int`, which reverses 5b-1's unboxed `i64`; a separate `BigInt` type can come later.
Wrapping arithmetic, when wanted (hashes, generators), should be explicit operations, not
the default: a later builtin (`int.wrapping_mul`), noted in HANDOFF.

### D2. The evaluator uses checked operations

`apply_binop` uses `checked_add/sub/mul/div`, and `apply_unop` `checked_neg`; `%` checks
zero and maps a divisor of -1 to 0. Both the CEK machine and the tree-walker share these,
so the oracle crosscheck is unaffected.

### D3. Native: overflow intrinsics and named traps

`prim_values` (one table for the direct and the CPS emitters) emits
`llvm.s{add,sub,mul}.with.overflow.i64` and branches on the overflow bit to a cold block
that calls the runtime's `elya_int_overflow` (noreturn: message, `exit(1)`). `/` checks the
divisor for zero (`elya_div_zero`) and `MIN / -1` (`elya_int_overflow`) before `sdiv`; `%`
checks zero (`elya_rem_zero`) and computes `srem` against a divisor of 1 when it is -1
(`select`), so `srem` never sees the one input where it is undefined. Every trap block ends
in `unreachable` after the noreturn call. Unary minus is not in Core yet ("Unary" is
refused by name in lowering); that refusal is unchanged.

Expected-value change the decision requires: the codegen unit test
`rejects_div_specifically` (asserting `Unsupported("Div")`) becomes `div_lowers`, replaced
red-first and named in the commit.

## 2. Negative controls

- K1: emit `add` without the overflow check -> the native overflow test fails (prints a
  wrapped value instead of the named trap).
- K2: drop the `-1` select in `%` -> `MIN % -1` natively (undefined in LLVM; on x86 the
  `idiv` faults, SIGFPE) fails the corpus.
- K3: evaluator `checked_mul` back to `*` -> the evaluator overflow test panics.

## 3. Results

Gate 869 passed, 83 suites (predicted 869). Controls: K1 -> `add` printed
-9223372036854775808 with exit 0; K2 -> `min-rem-minus-one` died on signal 8 (SIGFPE);
K3 -> the evaluator panicked on `MAX * 2`. Each reverted (`cmp`).

Independent review: no findings over 6 055 generated programs (every operator over the
boundary values, operands from literals, parameters, closures, match arms, `if` joins and
seven effectful shapes), native and both evaluators agreeing on value, exit status and
words. It confirmed phi incoming blocks are read after operand lowering, so the new
`arith_ok` blocks are safe. Unrelated findings, recorded in HANDOFF: a two-parameter
lambda passed to a higher-order function is "unrepresentable" natively; the one-shot
trap's words differ between the evaluator and native. Found while writing the tests: the
parser continues a call across a newline (`ix()` then a line starting `(`).
