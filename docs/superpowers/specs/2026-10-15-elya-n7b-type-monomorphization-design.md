# N7 part 2: type variables and parametric ADTs natively

**Status:** done (2026-10-15). HANDOFF step 1. Design choices are Claude's under the
maintainer's delegation (rule 2), recorded here.

## 0. Measured first

At `0e92180` (`auto/elya`, = `main` after #22), through the evaluator and `elya_codegen`
(the probes are this slice's corpus, `TYPE_VARS` in `native_codegen.rs`):

| Program | evaluator | native |
|---|---|---|
| q1 `twice(f, x)`, no annotations (`x: 'a`) | 49 | refused: unrepresentable type |
| q2 `id` at `Bool` and at `Int` | 42 | refused (same) |
| q3 `compose(f, g)` returning a lambda, at two instantiations | 57 | refused (same) |
| q4 `type List(a)`; `len` at `List(Bool)`, `sum` at `List(Int)` | 601 | refused: parametric ADT |
| q5 `map`, `foldl`, `range`: a `List(List(Int))` of 2000 | 1999000 | refused: unrepresentable type |
| q6 a `let`-bound `id` lambda at `Bool` and at `Int` | 42 | refused (same) |
| q7 `Option(a)`, `get(o, d)` | 7 | refused (same) |
| q8 `twice` with an effectful `f` (`{L}`): a row AND a type variable | 23000 | refused (same) |
| q9 `pair(a, b)` returning `fn(k) { k(a, b) }`; `mk(x)(y)` | 8 | refused (same) |
| r10 `dup() { put(get()) }` over `effect St(s)`, `s` only in `dup`'s own row | 5 | refused (same) |
| r11 `let n = Nil` used at `List(Int)` and at `List(Bool)` | 12 | refused: parametric ADT |
| r13 `twice_get(d)`: a handler for `St(s)` inside a generic function | 42 | refused (same) |
| r15 mutually recursive generic `ev`/`od` at two types | 133 | refused: unrepresentable type |
| r17 `type Box(a) { Box(fn(a) / {L} -> a, a) }` | 40 | refused (same) |
| r18 a non-regular type `Nest(a) { NNil, NCons(a, Nest(List(a))) }` | 1 | refused: parametric ADT |
| r19 `seven(Nil)`: an unconstrained (ambiguous) type variable | 14 | refused: unrepresentable type |
| r20 a `List(Option(Int))` of 100 000 (the collector runs) | 5000050000 | refused: parametric ADT |
| r21 a two-parameter lambda passed to a generic function | 3 | refused: unrepresentable type |
| r22 a generic local passed to a generic local | 15 | refused (same) |
| r23 nested patterns over `List(List(Int))` | 43 | refused (same) |
| r24 r10 with `St(String)` | 5 | refused (same) |
| r25 `effect Gen(a)`: a generic `each` performing `yield(h)` | 21 | refused (same) |

Every generic program is refused natively. HANDOFF's open item "a two-parameter lambda
passed to a higher-order function is unrepresentable" is r21: a type variable, not a
lambda arity problem.

## 1. The problem

The back end is monomorphic by construction: `repr_ty` maps a type to a machine
representation (`i64`, `i1`, a pointer), the collector's descriptor rows say per field
whether it is a pointer (`is_heap_ty`), and a frame's root map is computed from the
types of the values it saves. A type variable has no representation, and a parametric
constructor's field mask depends on its arguments (`List(Int)`'s head is not traced,
`List(List(Int))`'s is).

## 2. Options and choices

### Representation of a type variable

- (a) **Uniform representation** (OCaml, Koka): every value is one word; integers are
  tagged (or boxed) so the collector can tell a pointer from an integer at run time.
  Needs a tag bit on every `Int` (arithmetic and the checked-overflow intrinsics of
  2026-10-10 change: 63-bit integers, or boxing), a dynamic pointer test in `gc_mark`,
  and Bools widened. It changes the semantics of `Int` (63 bits) or its cost (boxing),
  and makes `gc_mark` guess where it now knows.
- (b) **Dictionary passing** (a runtime type descriptor per type variable): every generic
  function takes a hidden descriptor per variable; frames consult it to root a value
  of type `'a`; constructors of `List(a)` carry or look up a mask at run time. Every
  emitter path that reads a type (rooting, frames, descriptors, `repr_ty`) gains a
  dynamic case.
- (c) **Whole-program monomorphization (chosen)** (MLton, Rust): a Core-to-Core pass makes
  one clone of each generic function per ground instantiation reached from `main`'s
  module, and one ground nominal type per parametric-type instance. After it, every
  type the back end sees is ground, so `repr_ty`, the masks, the frames, the CPS
  partition and the leak analysis apply unchanged -- the same argument that chose
  Core-to-Core substitution for rows in part 1.

(c) is the stricter, more principled option: the collector stays PRECISE (every mask is
computed from a ground field type, as today), `Int` stays a full 64-bit checked integer,
and no part of the emitter learns that polymorphism exists. Its costs are code size
(bounded, below) and that polymorphic recursion cannot be monomorphized -- Elya's
inference never produces it (recursion within a binding group is monomorphic, and
annotation variables are flexible, not rigid), so it cannot arise; a non-regular TYPE
(r18) can, and is handled by instantiating types lazily (below).

### Pipeline order

1. `mono::run` (new): type variables in functions and local generic values.
2. `specialize::run` (part 1): rows and upcasts -- unchanged, except that a pattern
   binder of a parametric constructor is now tracked with its substituted field type.
3. `mono::instantiate_types` (new): parametric ADT instances become ground nominal
   types; constructors and patterns are renamed.
4. `patterns::run`.

Types are instantiated AFTER part 1 because part 1 substitutes row variables that may
occur inside type arguments (`Option(fn(Int) / {|r} -> Int)` at `r := {L}`): an instance
made before it would freeze a field's open row, and a pattern binder read from that
field would be called with the direct convention while holding CPS code.

### Functions

- A top-level function is **generic** iff a type variable is free in its signature
  (parameter and result types). A reference to it (a `Var` node, as a callee or as a
  value) matches the generic signature against the reference's type; the reference goes
  to the clone for that substitution (`name.mono.k`), made once per canonical key and
  scanned in turn (to a fixpoint). Generic originals are not emitted.
- A `let` whose value is a syntactic value (`Lambda`, `Var`, a nullary `Ctor` -- the
  inference's value restriction) and whose type has a free type variable is a **generic
  local**: each use goes to a clone bound in the scope the value sees (part 1's F1
  lesson), one per substitution; the original binding is dropped (every use is
  redirected; a use that is not is an internal error, never a silent fallback). A
  nullary constructor is cloned too: `let n = Nil` used at `List(Int)` and at
  `List(Bool)` must build two DIFFERENT instances' `Nil` (r11), since a match on
  `List(Int)` reads `List(Int)`'s tags.
- **Ambiguous variables default to `Unit`.** A type variable left after substitution
  is unconstrained (r19: `seven(Nil)`); no value of that type is ever built, so any
  representation is sound, and `Unit` (an untraced word) is the one that needs nothing.
  The default is applied in the clone key too, so `seven` gets one clone, not one per
  call site.
- **Except in an effect position: refused by name.** (Narrowed after the review, F2: only
  an effect parameter that some operation's signature mentions counts --
  `CoreModule::effect_params_used`; a phantom parameter carries no value and defaults.) A variable that appears only in a
  function's OWN row (r10, r24: `dup` is generic in `s` through `St(s)` alone) carries
  real values -- the handler's `get` answers a `String` -- but `CoreFn` does not record
  a function's own row (part 1 §6), so the instantiation is invisible at the
  reference. Defaulting it would type a string as `Unit` and leave it unrooted. So a
  leftover variable in a `Perform`'s type, a clause parameter, or an effect label's
  arguments refuses the module by name (`"type variable only in an effect row"`).
  Guessing is not an option; recording the own row in Core is the follow-up (it moves
  the typed table's node-count tripwire, so it is its own step).
- **Clone keys are canonical:** spans are ignored and row variables are renamed by first
  appearance, so two call sites whose types differ only in fresh row variables share a
  clone. The row variable kept is open either way (a direct convention); part 1 then
  row-specializes the clone where a caller's rows name a user effect.
- **Budget:** 256 type clones per module (`"too many type specializations"`), stated as
  part 1 states its own; termination does not depend on it.

### Parametric types

- Lowering records parametric declarations in a new `CoreModule::generic_types`
  (fields with `Ty::Var(i)` for the i-th parameter). `types` keeps only ground
  declarations, so the Core printer and its snapshots are unchanged.
- After part 1, every `Con(T, args)` with `T` parametric is renamed to a ground nominal
  `Con("T.mono.k", [])`, one name per canonical `(T, args)`. Instances are built
  **lazily**: a name is assigned when a type is renamed, but its constructors (and so
  its descriptor rows) are made only when code DEMANDS it -- a constructor node or a
  constructor pattern at that type. No value of an undemanded type exists, so it needs
  no rows. This is what makes a non-regular type (r18) finite: `Nest(Int)`'s field type
  `Nest(List(Int))` is named but only built if code builds or matches one.
- Constructors are renamed per instance (`Cons.mono.k`); a pattern's constructor is
  renamed from its scrutinee's type, nested patterns from the instance's field types.
- Instances are keyed exactly (`List(Int)` and `List(Bool)` are two types with their
  own tags), for the same reason part 1 kept exact keys: the types say what runs. Merging
  instances whose masks agree is a possible code-size measure, not a soundness one.
  **Except row tails** (the review's F1): a constructor built at `Option(fn(Int) -> Int)`
  and a match at `Option(fn(Int) / {|r} -> Int)` must be ONE type, or the match reads
  another instance's tags. A tail never changes a representation or a convention (only
  labels do, part 1 §4 K3), so the instance key leaves tails out.
- **Budget:** 256 type instances (`"too many type instances"`).

## 3. Where

`crates/codegen/src/mono.rs` (LLVM-free, unit-tested without a `Context`), run from
`build_module`. Front end: `src/core.rs` (`generic_types`, `CoreGenericType`).

## 4. Evidence

- Every program of §0 except r10/r24 (refused by name, by design) compiles and prints the
  evaluator's value natively: `TYPE_VARS` in `native_codegen.rs` (value and differential;
  q9 binds `mk(5)` first -- a computed callee is refused by its own name, 5b-1 §5.3), plus
  r26 (added by K3) and the review's four programs. r20 runs 100 000 `Some` cells through
  collections (`a_parametric_list_survives_collection`).
- `mono.rs` unit tests: no type variable survives (q1, q5, q9); one clone per
  instantiation shared across call sites; a generic `Nil` cloned per use; one `Unit`
  clone for two ambiguous sites; both row-only refusals; both budgets; lazy instances
  for the non-regular `Nest` (4 built -- the plan predicted 3 and missed the innermost
  `NNil`, which IS built at `Nest(List(List(Int)))`).
- Two existing tests asserted the refusal this slice lifts and now assert the module
  compiles (`lib.rs`: `a_polymorphic_function_is_not_emitted_unless_instantiated`,
  `a3_an_unconstrained_polymorphic_effect_is_not_emitted`).
- Gate: 899 passed, 85 suites (predicted 899) before the review; 900 after it.
- Negative controls, each reverted (`cmp` against a saved copy):
  - K1, references not redirected: the corpus is refused, "callee is not a top-level
    function" (the originals are no longer emitted).
  - K2, instance fields unsubstituted: the compiler panics inside inkwell on all three
    native tests. K2b, the mask alone wrong (`Cons.mono.*` fields untraced): r20's binary
    exits 1 after a collection; only the collector test catches it.
  - K3, no ambiguity default: the native corpus still PASSED (r19's variable sits under an
    instance name). A finding: r26 (an ambiguous lambda parameter) was added; it is
    refused "unrepresentable type" under K3.
  - K4, the effect-position check off: r10 and r24 compile and print 5 by luck; r27
    (`dup` holding a `List` across 200 000 allocations, then `put(v)` summed by the
    handler) crashes natively (signal) where the evaluator prints 700500. With the check,
    refused by name.
  - K5, instances built eagerly: r18 hits "too many type instances".
  - K6, label arguments ignored by the effect check: the `getter`/`putter` unit test
    fails (its `String` crosses `dup3` only through callee rows).

## 5. The independent review

About 150 adversarial probes against this change and a `HEAD` worktree. Fixed, red first:

- **F1 (high, new).** A pure closure inside a parametric type (`total(Some(fn(x) { x + 1
  }))`, lists of closures, `map` building 200k closures): "match failed" natively. The
  constructor's instance was keyed at a closed row and the matching parameter's at an
  open one -- two instances with distinct tags. Instance keys now leave row tails out.
- **F2 (medium, a regression).** An effect with a phantom parameter (`effect E(a) { fn
  e() -> Int }`) compiled at `HEAD` and was refused by the effect-position check. The
  check now counts only parameters an operation uses (`effect_params_used`, recorded by
  lowering); the `getter`/`putter` test (K6) keeps the real case refused.

Pre-existing and unchanged (refused at `HEAD` too): a top-level function used as a value,
`==` on strings and ADTs, `<>` natively. Passed: handlers inside generic functions,
`with multi` over generics, shadowing of generic locals (part 1's F1 shapes), aliases,
nested and non-regular types, ambiguous variables, and GC-heavy runs with collections.

## 6. Not covered

- A type variable only in a function's own row (r10, r24, r27): refused by name (§2).
  Recording a function's own row in Core lifts it.
- A top-level function used as a value is still refused by its own name (pre-existing).

