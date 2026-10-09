# N7 part 1: convention specialization and the upcast adapter (native)

**Status:** done (2026-10-09). HANDOFF step 1. Design choices are Claude's under the
maintainer's delegation (rule 2), recorded here.

## 0. Measured first

At `5799ba9` (`main`, #21 merged), through `elya check`, the evaluator and `elya build`
(the programs are the slice's test corpus, `CONVENTIONS` in `native_codegen.rs`):

| Program | `check` | evaluator | native |
|---|---|---|---|
| p1 `fn w(k) { k(0) + lg(1) }`, a pure `k` | ok | 15 | refused: effect-polymorphic function used at a user effect |
| p3 `fn apply(f) { f(1) + 1 }` at pure and at `{L}` | ok | 842 | refused (same) |
| p5 a recursive `mapl(f, n)` at pure and at `{L}` | ok | 55110 | refused (same) |
| p6 a `let`-bound generic lambda `app` at `{S}` | ok | 20 | refused (same) |
| p9 `twice(f: fn(Int) -> Int, x: Int)` at pure and at `{L}` | ok | 18311 | refused (same) |
| p11 mutually recursive `ev`/`od` over `f` | ok | 5015 | refused (same) |
| p12 `apply2` calls the generic `apply` (transitive) | ok | 22077 | refused (same) |
| p13 `apply` at `{L}`, at `{M}` and at `{L, M}` | ok | 111715 | refused (same) |
| p14 a generic call inside an effectful lambda | ok | 126 | refused (same) |
| p15 `guard(f)` handles `E` around `f()`; `f` also performs `L` | ok | 11046 | refused (same) |
| p19 `go(f, n)` non-tail recursion, n = 2000 | ok | 2001000 | refused (same) |
| p2 `twice(f, x)` (no annotations: `x: 'a`) | ok | 311 | refused (same); a type variable, part 2 |
| p4 `if c { f } else { fn(x) { t() + x } }`, `f` a direct closure | ok | 23 | refused: direct function used where an effectful one is expected |
| p7 the same through a handler's return binder | ok | 11102 | refused (same) |
| p17 a direct closure passed for a parameter typed `/ {T}` | ok | 6365 | refused (same) |
| p18 p4 with a capture | ok | 271702111 | refused (same) |
| **p8** `f = fn() { fn(x) { x + 1 } }` joined with `fn() { fn(x) { t() + x } }` | ok | **3306** | **2 — a miscompile** |

p8 is a native bug on `main`: the upcast is one level down, in the RESULT of the outer
function. The outer conventions agree (both direct), so the refusal of 2026-10-09 (which
compares only the outermost row) let it through, and the inner closure, direct code, was
called with the CPS convention.

## 1. The two problems

Both are about CONVENTION, not representation: every closure is one pointer, and a call
site picks direct or CPS from the callee's TYPE at the use.

1. **A row-polymorphic function** (`apply(f: fn(Int) / {|r} -> Int)`) is compiled once,
   direct (D16: an open tail alone is direct). Used at `r := {L}`, its calls of `f` would
   have to be CPS calls.
2. **An upcast** (sub-effecting): a value whose type says direct is used where the type
   says effectful, at any covariant function layer. The value's code is direct.

## 2. Options and choices

### Specializing polymorphic functions

- (a) An emitter-level CPS clone per function in which "an open row variable counts as
  effectful" (the HANDOFF sketch). One clone is not enough once a function has two row
  variables (`compose(f, g)` at `f: {L}`, `g` pure would call the pure `g` with CPS), so
  this needs one clone per subset of effectful variables, and the leak analysis of
  handles inside the clone needs synthetic labels.
- (b) **Core-to-Core monomorphization of rows (chosen).** At every reference to a
  row-polymorphic function, match its generic signature against the type at the use; if
  some row variable is instantiated at a row naming a user effect, the reference goes to a
  clone of the function in which each such variable is SUBSTITUTED by the labels it was
  instantiated with (its tail closed). The clone is ordinary Core with ordinary types, so
  every existing analysis (`needs_cps`, the CPS partition, the leak analysis of handles,
  sites, frames, descriptors) applies unchanged and needs no new case. One clone per
  distinct `(function, substitution)`; clones are themselves scanned, so a generic
  function calling another generic one (p12) and recursion (p5, p11: the clone refers to
  itself) resolve to a fixpoint.
- (c) Dictionary passing (pass the convention at run time). Every call of a parameter
  would test a flag; it trades code size for a branch at every call and complicates the
  frames. Rejected for this slice; part 2 revisits it as a code-size fallback.

(b) is the stricter, more principled option: the clone's types are exactly the types at
the use, so no part of the back end has to know that specialization happened, and the
existing D16 refusal stays in place as a guard that now fires only if this pass missed a
case. A variable instantiated at a row with no user effect (pure, `{IO}`, or another open
row) needs no clone: the direct original is right for it. Tails are closed in the
substitution because a convention depends only on the labels; leaving the caller's tail
open would make the clone generic again. **Budget:** at most 256 clones per module,
refused by name beyond (`"too many convention specializations"`). Termination does not
need it (label sets are finite), but a code-size bound should be stated, not implied.

`let`-bound generic lambdas (p6) get the same treatment locally: a clone of the lambda is
bound next to the original (`let app = ..  let app.1 = <clone>`) and the uses at that
instantiation are renamed. The clone captures exactly what the original captures, and is
in the same scope.

Type variables (`Ty::Var`, p2) are NOT substituted in this slice: monomorphizing types is
N7 part 2, with its own spec (specialization vs dictionary passing, a code-size budget).

### The upcast adapter

- (a) A runtime adapter closure kind with its own descriptor row
  (`adapt(clos, args.., k) = k(code(clos.inner, args..))`), emitted in LLVM.
- (b) **Eta-expansion in Core (chosen).** An upcast use of `x` (binder type `B`, use type
  `U`, conventions differ at some covariant layer) becomes
  `fn(a1, .., an) { coerce(x(a1, .., an) : B.ret, U.ret) } : U`. The new lambda's type
  names the effect, so it compiles with the CPS convention, and its body calls `x` with
  `x`'s own (direct) convention: that IS the adapter, built from parts the back end
  already proves correct. A layer whose result also differs (p8) is coerced recursively:
  the inner result is let-bound once and wrapped the same way.

(b) is chosen for the same reason as above: no new runtime object, no new descriptor
row, no new emitter path, and `gc_mark` stays byte-identical by construction (the
runtime is not touched). The cost is one closure allocation per upcast use, the same
as (a).

Parameter positions are never opened by sub-effecting (spec 2026-10-09 §2), so they agree
at an upcast; if they ever differ in convention, the pass refuses by name
(`"upcast in a parameter position"`) instead of guessing. Values reach native code
through tracked binders only (`let`, parameters, lambda and clause parameters, return
binders): ADT fields cannot be function-typed (annotations spec) and parametric
constructors are refused natively.

### The guard

The prepass refusal for upcasts compared only the outermost convention (which is how p8
slipped through). It now compares every covariant function layer, so if the adapter pass
is ever switched off, p8 is refused by name instead of miscompiled (negative control K2b).

## 3. Where

A new LLVM-free module `crates/codegen/src/specialize.rs`, run first in `build_module`;
every later phase works on its output. Clone names are `name.k` (`.` cannot appear in an
Elya identifier, so a clone cannot collide with a source function; the symbol is
`elya_name.k`, the shape LLVM itself uses for uniquified names, valid in ELF, Mach-O and
COFF).

## 4. Evidence

- Every program of §0 except p2 (a type variable, part 2) now compiles and prints the
  evaluator's value natively; they are the `CONVENTIONS` corpus (value and differential)
  plus three named tests. Eight more probes (two row variables instantiated differently,
  a capturing generic local at two effects, a generic call in a `match` arm, a two-parameter
  adapter, a clone whose handle the clone's own effect leaves, an upcast of a parameter)
  also agree with the evaluator.
- Gate: 794 passed, 77 suites (predicted 794) before the review; 796 after it.
- Negative controls, each reverted (`cmp` against a saved copy):
  - K1, references not rewritten: the eleven polymorphic programs are refused by the D16
    guard, by name.
  - K2, adapter off: the five upcast programs are refused by name, p8 included (the deep
    guard).
  - K2b, adapter off and the guard back to the outermost layer: p8 prints 2.
  - K3, one clone per function whatever the substitution: s2 fails (1 clone), the
    budget test fails, and the native corpus still PASSES. A finding: which user labels a
    variable stands for never changes a convention natively -- only whether it names
    one does (a handle inside a clone leaks as soon as any user label is added). So exact
    keying is precision, not soundness; it is kept because the clone's types then say
    exactly what runs, and part 2's code-size budget may merge clones whose conventions
    agree.

## 5. The independent review

About 100 adversarial probes, each run against this change and against a copy of `main`.
Fixed, each red first (`CONVENTIONS_REVIEW` and the alias test in `native_codegen.rs`):

- **F1 (high, a regression of this slice).** Local clones were bound INSIDE the scope of
  the binder they copy (`let x = v  let x.spec.k = ..`), so a free `x` in the lambda --
  meaning an outer `let`, parameter, clause parameter, return binder or top-level function
  -- captured the generic lambda itself: compiler panics, a segfault, "match failed", and
  1007 where the evaluator printed 12 (`main` refused all of them by name). The clones are
  now bound BEFORE the original, in the scope its value sees.
- **F2 (high, pre-existing).** An upcast in the RESULT of a callee (`if c { f() } else {
  .. }`: inference records the upcast on the callee `Var`) was neither adapted nor guarded:
  11 for 1511 (also on `main`). The adapter now keeps the call at the binder's own type
  and coerces its result; the callee guard compares every layer.
- **F3 (medium, pre-existing).** An alias of a generic local (`let mk2 = mk`) gets no
  clone; its result was a direct closure called CPS, and natively the program hung where
  the evaluator printed 8. The deep callee guard now refuses it by name ("effect-polymorphic
  function used at a user effect"); following aliases is left for later.

After the fixes, the whole probe set: no program differs from the evaluator; the refusals
left are by name (aliases, a computed callee, a function used as a value, type variables,
and b05 -- a generic function that handles an effect around its parameter, whose own row
is not in its signature, §6).

## 6. Not covered

- Type variables (p2, part 2).
- A row variable that appears only in a function's own row and not in its signature: its
  instantiation is invisible at the reference. If that ever makes conventions disagree,
  the existing D16 guard refuses it by name.
- p16 (`compose(f, g)` with one pure and one effectful argument) and `fn wrap(f) {
  fn(x) { f(x) + 1 } }` used at `{L}` are E0423 in the front end: a lambda forwarding an
  enclosing function's parameter forces that parameter pure (PARKED by the sub-effecting
  slice, "deferred to N7"). With this slice the native side no longer blocks keeping it
  open; it is the next front-end step.
