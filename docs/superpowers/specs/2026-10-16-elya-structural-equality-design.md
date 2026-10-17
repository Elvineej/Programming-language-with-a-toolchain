# Structural equality: `==` on data, checked; compiled natively

**Status:** done (2026-10-16). HANDOFF step 1. Design choices are Claude's under
the maintainer's delegation (rule 2), recorded here.

## 0. Measured first

At `1ce1a9d` (gate 1039 / 99), debug `elya`:

| Program | `elya check` | evaluator | native |
|---|---|---|---|
| e1 `P(1, 2) == P(1, 2)` | clean | eq | refused: Eq on an unrepresentable operand type |
| e2 `C(1, N) != C(1, C(2, N))` | clean | ne | refused (Ne) |
| e3 `(fn(x) { x }) == (fn(x) { x })` | clean | ne | refused |
| e5 `let f = fn(x) { x }  f == f` | clean | **ne** | refused |
| e9 `let g = f  f == g` | clean | **ne** | refused |
| e6 `B(fn(x) { x }) == B(fn(x) { x })` (a function in a field) | clean | ne | refused |
| e10 `fn same(a, b) { a == b }` then `same(fn.., fn..)` | clean | ne | refused |
| e7 `Unit == Unit` | clean | eq | refused |
| e8 `True == False` | clean | ne | ne |
| d1 two 1 000 000-element lists compared | clean | **host stack overflow, abort (rc 134)** | refused |

`==` is typed `a -> a -> Bool` for every `a` (`src/types.rs`, the `Binary` arm), and the
evaluator decides it with `Value`'s `PartialEq`: structural on constructors, strings and
scalars, and on functions whatever `Rc` identity and environment comparison give -- a
closure is not even equal to itself (e5). That comparison recurses on the host stack, so
comparing two long lists aborts the evaluator (d1). Natively only `Int`, `Bool` and
`String` compile.

## 1. What `==` means

- (a) Keep `==` total and give functions identity equality. Natively that is pointer
  equality, which depends on allocation (two evaluations of the same lambda are different
  closures, one closure shared through a variable is the same) -- a program's answer
  would depend on how the back end shares values, which the differential tests forbid.
- (b) Make comparing functions a run-time error. Decided only when the comparison runs,
  possibly deep inside a data structure; the type checker knows earlier.
- (c) **`==` and `!=` are defined on data only, and that is checked (chosen).** Data is
  the notion E0434 already uses for `with any`: base types, and declared types whose
  fields are data at arguments that are data; never a function. Comparing anything else
  is the new **E0436** at check time. Equality on data is structural: same constructor
  and equal fields, strings by their bytes.

(c) is the stricter option: the meaning of `==` no longer depends on representation, and
the rejected programs are exactly those whose answer the evaluator could not justify.

### D1. Equality type variables

A comparison at a type variable cannot be decided where it is written:
`fn same(a, b) { a == b }` is fine at `Int` and wrong at a function type. As in Standard
ML's equality type variables, the variable is **marked**: it must become data. A mark
travels with the variable: when it is bound, the type it is bound to must be data (a
variable it is bound to is marked in turn); when a scheme quantifying a marked variable
is instantiated, the fresh variable is marked. So `same(fn(x) { x }, fn(y) { y })` is
E0436 at the call, and a function that passes its arguments on to `same` is marked too.
Marks are set and checked eagerly -- at the comparison, at every binding of a marked
variable and at every instantiation -- not when the binding group is solved: a local
`let` generalized before the group ends then already carries its marks, so no
instantiation escapes the check (the lesson of E0434's S3, solved without keeping the
variable monomorphic). A variable still marked and unbound is generalized marked.
The printed scheme is unchanged (no `''a` syntax). An error found when a marked variable
is bound labels both the binding and the comparison it reaches.

Every type argument must be data, as in E0434, so a phantom parameter at a function type
(`type Tag(a) { Tag(Int) }` at `Tag(fn(Int) -> Int)`) is rejected though it stores no
function: the check reads types, not declarations' use of their parameters. Accepted.

An operation caught by `with any` (`$Op`) is data: E0434 already requires every operation
such a handler handles to take data, and its answer-type argument is an index, not a
field. So N3 (`o == o`) stays: two operations compare by what they are.

### D2. The evaluator compares iteratively

`==` and `!=` use one structural comparison over an explicit worklist, so d1 compares two
million-element lists without growing the host stack. Neither side takes a shortcut for a
shared value: a `Float` NaN is not equal to itself, in a field or alone, so sharing must
never decide (the review; `x == x` on `F(NaN)` was briefly `True` in the evaluator). Functions never reach it (E0436);
if one did, it is an internal error by name, not a silent `false`.

### D3. Natively: one runtime walk

`elya_eq(a, b, string_tag)` in runtime.c compares two heap values with an explicit
worklist, guided by the collector's descriptor table: tags must agree; a field whose
`ptr_mask` bit is set is compared as a value (pushed), any other field as a word; a
string (the string tag) compares its length and bytes. It does not allocate, so nothing
is rooted across it; `gc_mark` is untouched. After N7 part 2 every value is ground, so
the descriptor of a constructor instance says exactly which fields are pointers.
`Unit == Unit` is `True` (both operands still evaluated). Per-type generated equality
functions (the other option HANDOFF named) would duplicate N7's instances and recurse;
the walk is one function for every type.

## 2. Evidence

- `tests/structural_eq.rs`: E0436 for every rejected shape (e3, e5, e6, e10, a two-level
  generic, a local `let`, a parametric field, a top-level function as a value) beside
  accepted generic uses (`same` at `Int`, a list, a string, a declared type) (red: all
  check clean); the evaluator compares d1 (red: host stack overflow).
- `native_codegen.rs::EQUALITY`, value and differential: e1, e2, e7, nested and
  parametric types, strings in fields, nullary constructors, generic `same`, `==` inside
  a handler clause (the CPS emitter), long lists (red: refused).
- Negative controls: (K1) the binding rule stops propagating marks -> the instantiation
  and two-level rows check clean; (K2) `elya_eq` ignores `ptr_mask` (compares every field
  as a word) -> the structural rows that build equal values separately answer "ne";
  (K3) the runtime skips the string case -> the strings-in-fields row differs.
- First full gate: red. Cargo stopped at the first failing suite (lib.rs's unit test,
  H1 below); the review found the second.

## 3. The independent review

About 40 probes: no soundness hole (functions through local lets, curried and
higher-order uses, annotations, mutual recursion, ADT fields pulled out by `match`, effect
parameters and results, return clauses, `with any` values, closures over `resume`), no
native miscompile (Bool, Unit, String and nullary fields; mono instances; `==` in
lambdas, match arms, tail calls, CPS code and `with multi`; GC pressure; 1 000 000-cell
lists and a 1 000 000-deep tree). Fixed:

- **H1.** Two existing tests failed: `any_handler.rs::an_operation_equals_itself` (N3) and
  lib.rs's `rejects_eq_on_an_unrepresentable_operand_type`. The first is kept by making
  `$Op` data (D1); the evaluator compares operations by effect, name and arguments,
  iteratively. The second's operand was an ADT, which now compiles; no source program
  reaches the guard any more, so the test retypes the operands of `1 == 2` in a
  hand-built Core (**changed by this decision**; the expected refusal is unchanged).
- **L1.** The error did not point at the comparison: a second label now does
  (`the_error_names_the_comparison_it_reaches`, red first); an operation's skolem answer
  type no longer appears in a message (operations are data).
- **L3.** Phantom parameters: accepted and recorded (D1).
- **L4.** The evaluator's pointer shortcut made `F(NaN) == F(NaN)` depend on sharing;
  removed in both evaluator and runtime (`sharing_never_decides_equality`, red first).

Predicted gate after the review: 1045 + 4 = **1049 passed, 100 suites**. Measured:
**1049 passed, 101 suites** -- the suite prediction was wrong: a new root test file is a
suite in each of the gate's two configurations.
