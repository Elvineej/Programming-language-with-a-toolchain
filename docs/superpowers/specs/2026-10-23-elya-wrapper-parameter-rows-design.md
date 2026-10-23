# A wrapped function's row shared with a parameter, natively

**Status:** in progress (2026-10-23, branch `auto/elya`, a scheduled run). HANDOFF
step 1 (the review of upcasts through data, F2). The design choices are Claude's
under the maintainer's delegation (rule 2), recorded here.

## 0. Measured first (at 99a84e2, gate 1198 / 117)

`effect E { fn e() -> Int }`. Each program passes the top-level `mko` as a
value where a function with effectful parameter and result types is expected.
`elya run --value` against the native binary:

| program | evaluator | native |
|---|---|---|
| f2: `fn mko(k: fn() -> Int) -> fn() -> fn() -> Int { fn() { k } }`, used at `fn(fn() / {E} -> Int) -> fn() / {E} -> fn() / {E} -> Int` | 16 | **a pointer** |
| effarg: f2, the argument performing `e()` | 16 | **a pointer** |
| s8: f2, the middle layer unwritten (pure) in the use | 16 | **a pointer** |
| r8: `-> fn() -> Option(fn() -> Int)`, `Some(k)` | 16 | **a pointer** |
| s3: r8, the middle layer pure in the use | 16 | **a pointer** |
| s4: r8 through `let g: T = mko` | 16 | **a pointer** |
| one: `fn mko(k: fn() -> Int) -> fn() -> Int { k }` (one layer) | 16 | 16 |
| unused: f2's type, `k` unused | 16 | 16 |
| call: f2's `mko` called directly, not as a value | 16 | 16 |

One cause. `anf`'s wrapper types its inner call of `mko` with `mko`'s own
result when that is ground (2026-10-23 scope-captures D2), else with the use's.
The own result here is `fn() / {} -> fn() / {|r} -> Int`, `r` the row of the
parameter `k`. The upcasts-through-data slice's D1 binds a free row variable of
the own result from the use's result, but deliberately not one a parameter
mentions, so the own result stays unground and the use's result
(`fn() / {E} -> ...`, the outer layer effectful) types a call of `mko`'s
DIRECT result closure: the CPS call jumps into direct code. `one` works because
its own result IS the parameter's type; `unused` because `r` is free there.

## 1. Decisions

- **D1. A parameter's row variables are bound from the use's parameters.** The
  reference instantiates `mko` at the use's parameter types (the use's type is
  the instantiated scheme; only results and data are widened by sub-effecting,
  never a parameter), so a row variable a parameter mentions takes the use's
  row at the same position: the instantiation, exactly as `specialize`'s
  `user_subst` reads it to choose the clone. Matching walks every position of
  the parameter types (their own parameters too). Only then are the own
  result's remaining free variables bound from the use's result (the previous
  slice's D1). A variable whose use row is OPEN (row-polymorphic code) binds
  nothing, as before: the own result stays unground and the use's type is kept.
  f2's inner call becomes `fn(fn() / {E} -> Int) / {} -> fn() / {} -> fn() / {E} -> Int`:
  `specialize` clones `mko` at `{E}` and coerces the outer layer.
  - *Rejected:* refusing the shape by name. Six ordinary programs would stay
    refused where the evaluator runs them, and the exact answer is cheap.
  - *Rejected:* binding from the result position first. A result layer may be
    an upcast (wider than the instantiation); a parameter layer never is.

## 2. Evidence (planned)

`crates/codegen/tests/native_upcast_data.rs` gains
`wrapped_results_sharing_a_parameter_row_agree`: f2, effarg, s8, r8, s3, s4,
plus one and unused (already agreeing, pinned). Negative control: D1 off (the
parameter pass skipped), f2 prints a pointer again.
