# Native arity: argument records past three parameters

**Status:** done (2026-10-16). HANDOFF step 1. Design choices are Claude's under
the maintainer's delegation (rule 2), recorded here.

## 0. Measured first

At the tip after the callees slice (gate 1012 / 95), natively (the evaluator runs each):

| Program | evaluator | native |
|---|---|---|
| a1 a direct function of six parameters | 91 | refused: function takes more than five parameters |
| a2 an effectful function of five | 154 | refused: effectful function takes more than four parameters |
| a3 a lambda of five | 48 | refused: lambda takes more than four parameters |
| a4 an effectful lambda of four | 109 | refused: effectful lambda takes more than three parameters |
| a5 an operation of four parameters, performed and handled | 1235 | refused: effect operation takes more than three parameters |
| a6 a six-parameter tail loop over 100 000 elements | 5000150006 | refused: function takes more than five parameters |
| a7 a five-parameter lambda passed to a higher-order function | 15 | refused: lambda takes more than four parameters |

The caps are LLVM parameter counts: win64 cannot guarantee a tail call past five
(5b-3 §2, obligation T1); a closure pointer, a continuation and a handler frame each take
one, so the source caps are 5, 4, 4, 3 and 3 by kind.

## 1. Options

- (a) Spill only past each kind's own cap. The kind of a FUNCTION TYPE (is a value of
  this type called with the CPS convention? with a closure pointer?) is not known where
  types are rewritten -- a top-level function's convention comes from the CPS partition,
  computed later -- so the same source type would need different shapes at different
  places, and every caller would have to agree on which.
- (b) **One rule for every function, lambda, function type and operation (chosen):** at
  most three source parameters travel as themselves; past three, the first two do and
  the rest travel in ONE argument record -- an ordinary constructor of a synthesized
  ground type `$Args.k`, one per distinct list of spilled types. Three is the smallest
  cap of any kind, so after the pass no kind can exceed its cap, on any platform. The
  record is built at the call (an ordinary `Ctor`, rooted by the existing machinery) and
  taken apart by an ordinary `match` at the callee's entry, so no emitter path, no
  runtime function and no collector change is needed: `gc_mark` stays byte-identical.
- (c) Spill only on win64. Two conventions, and the Linux gate would never run the one
  Windows uses. Rejected (as HANDOFF suggested).

(b) is the stricter option: the shape of a call depends only on its parameter count. The
cost: functions of four or five parameters, which compiled before, now allocate one
record per call. Measured on a6 (a six-parameter tail loop over a list of 100 000, the
list built by the same program): 23 ms natively, against 16 ms for the same loop written
with three parameters -- one three-word record per iteration.

## 2. Where

A Core-to-Core pass `crates/codegen/src/spill.rs`, run after pattern compilation (every
type is ground and every match flat by then) and before the CPS analyses. It rewrites
every `Ty::Fn` (in node types, parameters, perform types, clause parameters and
constructor fields), every top-level function, lambda and clause with more than three
parameters (entry `match $rest.k { $args.k(p2, ..) -> body }`), every call and perform
with more than three arguments. The emitters' arity refusals stay as guards behind it.

## 3. Evidence

- `native_codegen.rs::ARITY` (a1-a7 and the review's r1-r3; value and differential),
  red at the start (every row refused by an arity cap) and r1-r3 red against the first
  version (measured by the review).
- `spill.rs` unit test; the five lib.rs arity refusals now assert compilation (changed
  by this decision), and `MAX_SOURCE_PARAMS + 2 <= MAX_PARAMS` is a compile-time
  assertion: the emitters' arity guards cannot be reached.
- Gate: 1015 passed, 95 suites (predicted 1015; the review added corpus rows only).

## 4. The independent review

About 60 probes (4-8 parameters, direct and effectful, multi-shot operations resumed
twice, 300k-step mutual tail recursion, many-parameter closures through generic
higher-order functions and ADT fields, operations of 4-5 parameters inside lambdas and
clauses, GC pressure with records holding strings, lists and closures; all examples
byte-identical to `HEAD`). Fixed, red first:

- **F1 (high).** A function in a spilled position (`k(3, 1, 0, fn(p) { p + 1 })`): the
  call keyed its record by the ARGUMENT's type (an open row), the callee by its
  PARAMETER's (closed) -- two records, "match failed". Records are now keyed by what a
  value looks like in a field (a function type by its calling convention, never its
  row's labels or tail), and a call packs the record its callee's parameter types name.
