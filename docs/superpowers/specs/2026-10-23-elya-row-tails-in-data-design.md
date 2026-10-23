# Upcasts through data, natively

**Status:** in progress (2026-10-23, branch `auto/elya`, a scheduled run). HANDOFF
step 1 ("a row variable inside data in a function's own result"). The design
choices are Claude's under the maintainer's delegation (rule 2), recorded here.

## 0. Measured first (at fb56487, gate 1193 / 116)

`effect E { fn e() -> Int }`, `type Option(a) { None, Some(a) }`; each program
passes a value holding a pure closure where data of an effectful one is expected
(sub-effecting through a data constructor). `elya run --value` and `elya build`:

| program | evaluator | native |
|---|---|---|
| r1: top-level `fn mko() -> fn() -> Option(fn() -> Int)` used as a value at `fn() -> fn() / {E} -> Option(fn() / {E} -> Int)` (the 2026-10-23 review's F2) | 15 | **a pointer** (140216857793728) |
| top_val: `mk2() -> fn() -> Option(...)` returning top-level `mk` | 16 | **a pointer** |
| let_lam: `let mk = fn() { Some(fn() { 15 }) }`, `run(mk())` | 16 | **"match failed"** |
| list: the same with `Cons(fn() { 1 }, Cons(fn() { 20 }, Nil))` | 321 | **"match failed"** |
| nest: `Two(4000, Some(fn() { 15 }))` (nested data) | 4115 | **"match failed"** |
| direct: `run(Some(fn() { 15 }))` | 16 | 16 |
| let_val: `let v = Some(...)`, `run(v)` | 16 | 16 |
| top_ann: `fn mk() -> Option(fn() -> Int)`, `run(mk())` | 16 | 16 |
| sink: a contravariant field (`Sink(fn(a) -> Int)`) | E0423 (the checker refuses it) | (front end) |

Three wrong outcomes, by two causes:

- **r1, top_val (the wrapper).** `anf` wraps a top-level function used as a value
  and types the inner call with `f`'s own result when that is ground. A row
  variable inside DATA in the own result (`Option(fn() / {|r} -> Int)`) is never
  ground (`mono::match_tys` binds no row tail), so the use's result was used, and
  the call jumped into `mko`'s direct result closure with the effectful
  convention.
- **let_lam, list, nest (an upcast of a call's result through data).** The
  binder's own type has the pure closure inside `Option`, the use's the effectful
  one. `specialize` coerces a call's result only through function layers
  (`coercible`); through a data constructor it left the call as it was. The
  value was built at one instance of `Option` (direct field) and matched at the
  other (CPS field): two tag sets. The backstop guard (`check_local_conventions`)
  did not refuse it either: it runs after `mono::instantiate_types`, where both
  are argument-less instance types, which `conv_eq` counted as agreeing.

The ones that work do so because the producer is instantiated at the use (a
top-level function's clone, or inference unifying a `let` value with its use).

## 1. Decisions

- **D1. The wrapper's own result binds its own row variables from the use.** A
  row variable of `f`'s own result that no parameter and not `f`'s own row
  mentions is instantiated by this use alone, so it takes the use's row at the
  same position (results and data arguments; parameters keep their own types), as
  a call's instantiation does; `specialize` then clones `f` for it. A CLOSED row of
  the own result stays closed: that layer is a real upcast, which the coercion
  reaches. r1's inner call is `fn() / {} -> Option(fn() / {E} -> Int)`, coerced
  at the outer layer.
  - *Rejected:* closing such variables (the direct convention `mono` gives an
    unbound tail). It made the own result ground but turned r1 into an upcast
    through data at the wrapper, where a clone is the exact answer.
- **D2. An upcast through a data constructor is coerced by rebuilding the data.**
  `specialize`'s coercion learns a third case: a value of `T(a1..an)` used at
  `T(b1..bn)` (same declared type, arguments differing in convention) becomes a
  call of a generated pure function `$coerce.T.k`, which matches every
  constructor and rebuilds it with each field coerced (a function field by the
  existing adapter lambda, a data field by its own `$coerce`, a recursive field by
  the function itself). One function per (from, to) pair, made once.
  - The call-result path (`coercible`) and the value path both use it.
  - Data is immutable, and E0436 keeps functions out of `==`, so a copy is
    indistinguishable from the original.
  - The cost is a copy of the value's spine, once per upcast, natively only.
  - *Rejected:* refusing by name ("upcast through a data constructor", the
    message the value path already had). It would leave five ordinary programs
    of the measured family refused, where the evaluator runs them.
  - *Rejected:* making data invariant in the type checker (no sub-effecting
    under a constructor). That changes which programs check, for a back-end
    reason.
- **D3. The guard compares instance names.** After `mono::instantiate_types`, two
  data types agree in convention only if they are the same instance; `conv_eq`
  compares names as well as arguments, so a disagreement D2 misses is refused by
  name instead of matching the wrong tags.

## 2. Evidence (planned)

- `crates/codegen/tests/native_upcast_data.rs` (execution, differential): r1,
  top_val, let_lam, list, nest, plus a list of a million closures upcast once
  (the copy is iterative enough natively: the 1 GiB stack), a value path upcast
  (`let v: Option(fn() / {E} -> Int) = mk()` style), and an upcast of data
  holding data holding a function.
- Negative controls (in its header): D1 off (r1 prints a pointer again), D2's
  data case off (let_lam is refused by name, since D3 now catches it: the
  control shows the guard works), D3 off with D2 off (match failed).
