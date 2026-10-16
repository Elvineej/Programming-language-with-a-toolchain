# A function's own row in Core

**Status:** done (2026-10-16). HANDOFF step 1. Design choices are Claude's under
the maintainer's delegation (rule 2), recorded here.

## 0. Measured first

At `7cbef8a` (gate 998 / 93), natively (the evaluator runs all four):

| Program | evaluator | native |
|---|---|---|
| r10 `fn dup() { put(get()) }` over `effect St(s)`, handled at `St(Int)` | 5 | refused: type variable only in an effect row |
| r24 r10 handled at `St(String)` | 5 | refused (same) |
| r27 `dup` holds a `List(Int)` from `get` across 200 000 allocations, `put` sums it | 700500 | refused (same) |
| g1 `getter() { get() }`, `putter(v) { put(v) }`, `dup3()` between them, at `St(String)` | 0 | refused (same) |

N7 part 2 (spec 2026-10-15 §2) refused these on purpose: `dup` is generic in `s`, but `s`
occurs only in `dup`'s own row, which `CoreFn` does not carry, so the instantiation at a
reference is invisible, and defaulting `s` was measured unsound (r27 crashed).

## 1. Where the row lives

- (a) A separate side table returned next to the typed table (`infer_typed_table` grows
  a third result, `lower_module` a third argument): every caller of `lower_module` -- the
  CLI, three test harnesses, the codegen unit tests -- changes.
- (b) **The function's type recorded in the typed table at its DECLARATION's span
  (chosen)**, read by `lower_module` into a new `CoreFn::row`. The table already records
  non-expression spans (parameters, callee spans), and one declaration span per function
  is a recording path of the same kind; nothing outside lowering reads it. Its one cost
  is the table's node-count tripwire (`infer_with_types_records_every_node_and_zonks_after_inference`),
  which pins exactly how many spans are recorded: it moves 5 -> 6 for its one-function
  program, and its "all `Int`" check now skips the declaration's function type. That
  test exists to be moved deliberately; this is that. Measured on the way: the seven
  `typed_inference` snapshots render the whole table, so each gains one line per
  declaration (`0..20 \`fn add1(n) { n + 1 }\` : fn(Int) -> Int`); no existing line
  changes. Accepted with the decision (the new lines show the own rows, `fn worker() :
  fn() / {State(Int)} -> Unit`, which is what the snapshots are for).
- (c) Record only when the row carries type arguments: keeps the tripwire, but makes the
  field partial and its presence a coincidence of the program. Rejected.

(b) is the stricter option: every function's Core carries its whole type, with one
source of truth (the zonked table), and lowering refuses a function without one
(`Untyped`), as it does for a parameter.

## 2. Use

- `mono.rs`: a function is generic iff a type variable is free in its parameters, its
  result OR its own row's effect-label arguments; a reference matches the whole
  `fn(params) / row -> result` against its type, so `dup` referenced at
  `fn() / {St(String)} -> Int` gets `s := String`. Clones substitute the row too.
- Part 1 (`specialize.rs`) keeps matching rows without the own row (its spec §6: a tail
  only in the own row cannot exist after closing -- unrelayed tails close), so its
  behaviour is unchanged; it substitutes the row of its clones.
- `instantiate_types` renames the row's label arguments like every other type.
- The "type variable only in an effect row" refusal stays as a guard: after this change
  it fires only for a variable no reference instantiates (an ambiguous type inside a
  handled effect), where it was always conservative.

## 3. Evidence

- `native_codegen.rs::row_only_type_variables_run_natively` (OWN_ROW: r10, r24, r27, g1;
  value and differential), red before (all refused by name).
- `core_lowering.rs::a_function_carries_its_own_row`; `mono.rs`
  `a_row_only_type_variable_is_instantiated_from_the_reference` and
  `..._through_generic_callees_is_instantiated` (were the two refusal tests; changed by
  this decision).
- Gate: 1001 passed, 93 suites (predicted 1001).
- A correction to N7 part 2's K4 note, measured here: r27 as written there crashed
  natively because 200 000 NON-TAIL `len` calls overflow the native stack (SIGSEGV, also
  with the refusal on, and gone under `ulimit -s unlimited`). With a tail-recursive count
  the defaulting it guarded against is still unsound, now shown directly: with the row
  ignored and the refusal off, native prints 4130074439 (a collected list summed) for
  700500. The OWN_ROW row uses the tail-recursive count.
- Negative controls: K1, the signature without the row -- the four programs are refused
  again by name; K2, that and the refusal off -- r27 prints 4130074439.
- The independent review: 52 probes (values of the effect's type held across 100k-300k
  allocations; mutual recursion through the row; lambdas, closures, generic and
  `with multi` handlers; two effects; part 1 together; phantom parameters), each through
  the evaluator and natively, against this change and `HEAD`: no finding, no regression.
