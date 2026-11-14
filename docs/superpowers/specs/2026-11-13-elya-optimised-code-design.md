# Optimised code

**Status:** done (2026-11-13, on `main`, an interactive session). HANDOFF step 2's
first item, found measuring the platform layer and again by ChatGPT's review:
`compile_module` selected instructions at `OptimizationLevel::None` and ran no
LLVM pass. Design choices are Claude's (rule 2).

## 0. Measured first

A benchmark suite did not exist. ChatGPT wrote one to Claude's brief (eight
programs, each in Elya, C and Python: naive recursion, a counting loop, n-queens
on lists, binary trees, a merge sort of a million, trial division, a list of a
thousand closures, a state handler); checked here: all eight Elya programs
compile and write what their C twins write. They are in the scratchpad's `bench/`
(the repository gets them with the benchmark runner, HANDOFF step 2).

Release compiler, best of three, against `clang -O2` of the C twin:

| program | C | Elya before | | Elya after | |
|---|---|---|---|---|---|
| `fib(38)` | 0.121 s | 0.262 s | 2.2x | 0.206 s | 1.6x |
| `sum` (300M iterations) | 0.290 | 1.950 | 6.7x | 0.339 | 1.2x |
| `queens` (11) | 0.025 | 0.142 | 5.6x | 0.120 | 5.5x |
| `trees` (depth 22, 20 times) | 8.885 | 8.892 | 1.0x | 9.558 | 1.1x |
| `sort` (1M) | 1.827 | 3.515 | 1.9x | 3.800 | 2.2x |
| `primes` (below 2M) | 0.373 | 0.600 | 1.6x | 0.307 | 0.9x |
| `closures` | 0.163 | 2.130 | 13.0x | 2.285 | 13.7x |
| `effects` (5M) | 0.014 | 0.530 | 38x | 0.562 | 42x |

(`effects`' C twin is two indirect calls, not a handler.) The arithmetic programs
reach C; what allocates, calls a closure or performs is where it was: those costs
are the runtime's and the calling convention's, not instruction selection's.
`default<O1>` and `default<O3>` measured within noise of `O2`.

## 1. Decisions

**D1. `default<O2>` is what a program is built with.** `compile_module` runs
LLVM's `default<O2>` over the module, verifies it again, and selects instructions
at `OptimizationLevel::Default`. `compile_module_with(.., Optimise::None)` is the
IR as written.

**D2. Why the passes are safe here.** Every heap value live across a call or an
allocation is on the shadow stack, in memory LLVM does not own and through calls
it cannot see into; the collector moves nothing, so a pointer LLVM keeps in a
register across a call stays right; tail calls are `musttail`; a trap (overflow,
division by zero) is a call that does not return, which no pass may move before
an earlier line's write.

**D3. Building stays linear.** The first gate with the passes on did not end:
`native_long_blocks` (blocks of 5,000 `let`s in every place a block can be) went
from half a minute to more than fifteen. Measured with `opt -time-passes`:
LLVM's instruction combiner took 117 s over one such function (100,000
instructions), 99.7% of the time. A function of more than 10,000 instructions,
which no person writes, is marked `optnone` (and `noinline`) and built as
written; every other function is optimised. `native_long_blocks` takes 34 s
(31 before the passes).

## 2. Tests

`native_optimise.rs` (new, 3): the eight benchmarks at small sizes and a program
that writes a line and then overflows write the same optimised and unoptimised,
plainly and under `ELYA_GC_STRESS=1`; the counting loop, `fib` and the trial
division are smaller objects optimised; a function of 6,000 `let`s of the slow
shape builds in a time the combiner would exceed many times over. Every other native test now runs
optimised code.

Changed expectation (named): `elya_stack_mib_sets_the_stack_and_is_checked` asks
for 2 MiB where it asked for 8: 200,000 optimised frames fit in 8 MiB.

## 3. What this leaves (HANDOFF step 2, in the order the table gives)

- A closure's call (13x) and a perform (the handler's continuation allocated at
  each): the calling convention and the frames.
- `queens` and `sort` (2x to 5x): allocation and the shadow stack's pushes at
  every call (ChatGPT's review, 6: root frames per function).
- One trap block a function (each checked `+` has a block of its own to call
  `elya_int_overflow`: 28 lines of IR a `let`), which is what makes a long
  function long.
- The benchmark runner and these programs in the repository, C twins beside
  them, so a change's cost is seen at the gate.

## 4. Predictions

Gate: 1527 + 3 = **1530 passed, 163 suites**.
