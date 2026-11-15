# `&&` and `||` short-circuit

2026-11-15. Status: built.

## Why

`&&` and `||` have been strict since slice 5b-2: both operands were evaluated
and then combined. The 5b-2 plan chose that so native code would agree with
the two evaluators, which were strict, and called short-circuiting "a
language-surface change, not a codegen one" (5b-2 spec §4.1). That change is
due now:

- Every language a newcomer arrives from (Python, C, Rust, JavaScript, Java)
  short-circuits. Elya aims to be as simple to write as Python, and a strict
  `&&` is a trap. `i < n && array.get(xs, i) != 0` reads past the end;
  `ok && check()` runs `check`'s effects when `ok` is false.
- The parser in Elya (self-hosting part 4b) fell into it: a float check on the
  right of `&&` ran on tokens that were not floats, and had to be rewritten as
  an `if`.

## Decisions

**D1. Meaning.** `a && b` means `if a { b } else { False }`, and `a || b`
means `if a { True } else { b }`. The right operand runs only when the left
does not decide the answer; its effects, failures and non-termination happen
only then.

**D2. Tail position.** The right operand is in tail position, as an `if`
branch is: `n == 0 || reaches(n - 1)` is a loop in constant stack, in the CEK
machine and natively. The tree evaluator does not eliminate tail calls, here or
anywhere else.

**D3. Where.** The parse is unchanged: still `Binary` with `And` or `Or`. The
tree dump the parser in Elya is held to stays byte for byte the same. Each
consumer gives the operator its meaning:

- the tree evaluator and the CEK machine evaluate the right operand only when
  the left does not decide (the CEK machine with a frame that branches on the
  left's value);
- lowering to Core produces the `If` of D1, so native code (and the noalloc
  and rooting analyses) see an ordinary branch;
- the affine checker is unchanged. It reads `a && b` as if both operands
  always ran, and that gives the `if`'s verdicts exactly: the `if`'s other
  branch is a literal, which uses, gives away and keeps alive nothing, so the
  `if`'s use counts (the larger branch), give-aways (either branch) and
  liveness (either branch) are the right operand's. A test pins the checker's
  codes on six programs to the codes on their `if`s; controls that made the
  checker read the right as one branch only were not caught, as this predicts
  (the plan's K5 to K7).

The type checker is unchanged (both operands and the result are `Bool`).

**D4. Native.** An `If` whose condition and branches allocate nothing
allocates nothing (`may_allocate`, `allocates_here`), just as the strict
`Prim` it replaces allocated nothing. Without that, a value held across an
argument that is a `&&` would be rooted (found by the review; control K8).
`Prim(And | Or)` no longer comes out of lowering for these operators; the back end keeps its strict `and`/`or` instructions for any Core
built by other means (the derived structural equality builds `And` of
comparisons, which cannot fail, so strictness there is unobservable).

**D5. The right operand's value is not checked to be a `Bool` by the CEK
machine,** as an `if` branch's is not: it is the answer, in tail position (D2),
and a frame to check it would undo that. Only a program the type checker never
saw can tell (the tree evaluator, which checks, says "type error in binary
operator" there; the CEK machine answers the value).

## Tests

tests/short_circuit.rs and crates/codegen/tests/native_short_circuit.rs, over
tests/programs/short_circuit.rs:

- a division by zero on the skipped side is never reached, in both evaluators
  and natively;
- effects on the right happen only when it runs, after the left's;
- the index guard `i < array.length(xs) && array.get(xs, i) != 0`;
- a loop through the right operand of `||` and of `&&`: the CEK machine's
  peak continuation depth the same at 100 and 10,000 calls, a million natively;
- a multi-shot effect performed on the right (`flip() || flip()` counts 2;
  read strictly, 3);
- the affine checker reports the same codes for each `&&`/`||` as for its
  `if`, over six programs that give a linear array away on the left, on the
  right, or read it on the left, and four more with a multi-shot point
  (E0429), a `linear` type (E0428) and a `borrow` parameter (E0439);
- natively, a string held across a `&&` argument is not rooted
  (crates/codegen/tests/native_roots.rs);
- the parse still holds the operator.
