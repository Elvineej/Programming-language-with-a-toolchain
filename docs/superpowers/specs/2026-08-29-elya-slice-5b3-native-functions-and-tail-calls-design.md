# Slice 5b-3 (arc node N2) — Native functions, calls, and guaranteed tail calls

**Goal.** A module with many functions compiles to a native binary in which every
Elya tail call is a machine tail call, so deep recursion runs in constant stack —
the same property `elya run` already has, obtained by a different mechanism and
proven by execution.

**Arc position.** N1 (arithmetic, Slice 5b-1) → N3 (control flow, Slice 5b-2) →
**N2 (this slice)** → N4 (ADTs + Match, the heap threshold). N2 is taken after N3
because branching is what makes recursion terminate; without `If` there is no
interesting recursive program to compile.

**Status.** Design approved in conversation on 2026-08-29, after the probe recorded
in §2. This document is the written form of that design.

---

## §0. Inherited ground rules

These come from 5b-1 and 5b-2 and are not re-argued here.

- **Proof is execution.** No `insta` snapshot of LLVM IR anywhere. Every claim about
  what the back end produces is proven by compiling, linking, running the binary, and
  reading its exit status, stdout, and stderr. No test may skip.
- **Semantic fidelity (5b-1 §3.4, extended 5b-2 §4.1).** Native codegen must be
  neither *more*- nor *less*-undefined than the tree evaluator. For every program both
  back ends accept, they must agree.
- **A clean refusal is fidelity-preserving.** Refusing a program removes it from the
  set *both* back ends accept, so it cannot make native more-undefined than the
  evaluator. This is the move `Div`/`Rem`, `Str` operands, and `Ty::Var` already make.
  N2 uses it four more times (§5).
- **Refused by name, never mis-compiled.** Every rejection is a `CodegenError`
  carrying a specific message, not a silent fallthrough.

---

## §1. What N2 adds

The back end today compiles exactly one function, named `main`, taking no parameters.
N2 lifts that to: any number of functions, each taking up to five parameters, calling
each other freely, including recursively and mutually recursively.

Concretely:

- `CoreFn` gains its signature — parameter *types*, not just names (§3).
- Every Elya function becomes an LLVM function with a mangled name and the `tailcc`
  calling convention (§4.1, §4.2).
- Calls in tail position emit `musttail`; calls elsewhere emit an ordinary call (§4.3).
- Four new refusals, all asserted by test (§5).

What N2 does **not** add: `Match`, ADTs, heap values, closures, lambdas, strings,
effects, floats. Those stay refused, and `repr_ty` already refuses most of them for
free. See §8.

---

## §2. The probe, and the decisions it forced

Before any design, a spike measured LLVM 18.1.6's behaviour on
`x86_64-pc-windows-msvc` across twenty-six hand-written modules, varying calling
convention (`ccc` / `tailcc`), tail marker (none / `tail` / `musttail`), and callee
signature. Every module went through `llvm-as`, `llc -O0`, `clang` as link driver, and
execution at recursion depth 1e6. The findings below are measured, not reasoned.

### Finding 1 — under `ccc`, `musttail` refuses ordinary Elya programs

The verifier rejects `musttail` under `ccc` whenever caller and callee signatures
differ:

> `cannot guarantee tail call due to mismatched parameter counts`
> `cannot guarantee tail call due to mismatched parameter types`

Mutual recursion between functions of different arity — `fn ev(n)` calling
`fn step(n, acc)` — is an ordinary Elya program, and the evaluator's TCE handles it.
Shipping `ccc` would therefore ship a *weaker* guarantee than `elya run` has, in a
shape that fires on real code. Under `tailcc` the same modules are accepted and run
bounded: `tailcc` + `musttail` is **arity- and type-blind**.

**Decision: `tailcc` from the start.** There is no cheap-`ccc`-first path, because the
escalation condition fires immediately.

### Finding 2 — the win64 argument-area gap, and why it is dangerous

`tailcc` + `musttail` is arity-blind *at the verifier*, but the x86-64 Windows backend
has an undocumented gap. Growing the outgoing argument area across a guaranteed tail
call fails during instruction selection with:

```
LLVM ERROR: Can't handle guaranteed tail call under win64 yet
```

This is `report_fatal_error`. It has **no source span, is not a recoverable `Result`,
and kills the process.** `install_fatal_error_handler` (which inkwell exposes) can
decorate the message but cannot recover — LLVM still exits. An `elya build` that
reaches this dies with no diagnostic.

Prevention is therefore mandatory, and must happen *before* any emission.

The measured matrix (`musttail` + `tailcc`, `llc -O0`; rows = caller arity, columns =
callee arity; `.` builds, `X` is the fatal error):

```
      1  2  3  4  5  6  7  8
 1    .  .  .  .  .  X  X  X
 2    .  .  .  .  .  X  X  X
 3    .  .  .  .  .  X  X  X
 4    .  .  .  .  .  X  X  X
 5    .  .  .  .  .  X  X  X
 6    .  .  .  .  .  .  .  X
 7    .  .  .  .  .  .  .  X
 8    .  .  .  .  .  .  .  .
```

Safe iff **callee arity ≤ 5**, or **caller arity ≥ callee arity**. The mechanism is
the callee's stack-argument bytes, rounded to 16, not exceeding the caller's. The gap
is platform-specific: the same module targeting `x86_64-unknown-linux-gnu` builds
without complaint.

**Decision: a flat ≤5-parameter cap, enforced as a hard refusal.** Not the
`callee ≤ 5 || caller ≥ callee` predicate, for two reasons. The flat cap needs no
call-site analysis and is safe by construction. And it rests entirely inside measured
ground — arities 1–8 were tested, and ≤5 extrapolates nothing. This is the same
discipline as `K_MAX`: pin what was measured, not what was hoped.

The refusal is preferred over argument-area padding. Padding is real engineering for a
case no current Elya program needs; it can be built later against actual demand. And
the cap counts *parameters*, not fields — so N4's ADTs and N5's closures do not
tighten it.

### Finding 3 — `tail` is not an acceptable substitute

`tail` is a hint, not a contract. A module using `tail` where `musttail` would be
rejected passed the verifier, passed `llc`, produced a binary — and that binary
**overflowed the stack at runtime**. That is silent failure exactly where the bound
matters.

(A correction to an earlier claim of mine: `tail` *is* honoured by `llc -O0` in easy
cases. That makes it worse, not better — it works until it quietly doesn't.)

**Decision: `musttail` only.** Its whole value is that the failure is a build-time
refusal instead of a runtime crash.

### Finding 4 — the print shim survives untouched

A `ccc` `@main` may make an ordinary (non-`musttail`) `call tailcc` to a `tailcc`
function. The existing runtime shim — `@main` calls `@elya_main`, then `printf`s the
result — needs no change.

---

## §3. Front end: `CoreFn` gains its signature

### §3.1 The gap

```rust
pub struct CoreFn {
    pub name: String,
    pub params: Rc<[String]>,   // names only — no types
    pub body: CoreExpr,
}
```

Codegen needs an LLVM function type, which needs a type per parameter. Two facts make
this a small change rather than a large one:

- **`lower_module` already lowers every function.** Multi-function modules reach Core
  intact today; only `validate_module` in the back end rejects them.
- **The return type is already there.** `lower_block` propagates the block's type onto
  the synthesized `Let` spine, so a function body's root `CoreExpr.ty` *is* its return
  type — inference unifies them (`unify(&body_ty, &result, f.body.span)`). No new
  field is needed for it, and adding one would create a second source of truth.

So only *parameter* types need recording.

### §3.2 The recorder change

The type table is `HashMap<Span, Ty>`, written at a single choke point (`infer_expr`
inserts every expression's type keyed by its span) and zonked once at the end of
`infer_all`. Recording a **non-expression** span in it already has precedent: 5a-2
Task 3 records the perform-callee shape at `callee.span` for exactly this reason.

The SCC loop in `infer_all` binds each parameter into the environment at a point where
its fresh type variable and its `Spanned<Param>` are both in hand. One additive line
there:

```rust
for (p, pty) in f.params.iter().zip(&params) {
    inf.node_types.insert(p.span, pty.clone());   // NEW
    env.insert(&p.node.name, Scheme { vars: Vec::new(), row_vars: Vec::new(), ty: pty.clone() });
}
```

The inserted type is a fresh variable at insert time and is resolved by the same final
zonk that resolves every expression type. The insert is unconditional, matching
`infer_expr`; `infer_all` already returns an empty table when types were not requested.

**Deliberate asymmetry:** `CoreKind::Lambda` still carries `Rc<[String]>` — names
only. Lambda parameters are not recorded, because N2 refuses lambdas outright (§5.3).
N5 will need them and will record them then. Recording now would be speculative
plumbing for an unreachable path, and recorder-totality obligations are closed by real
demand, not by blanket audit. Tracked as **T3** (§8).

### §3.3 The Core IR change

```rust
pub struct CoreParam {
    pub name: String,
    pub ty: Ty,
}

pub struct CoreFn {
    pub name: String,
    pub params: Rc<[CoreParam]>,
    pub body: CoreExpr,          // body.ty IS the return type
}
```

A named struct rather than a `(String, Ty)` tuple: `params` is a public IR type that
later slices extend, and the affine-resource work has already identified a parameter
multiplicity annotation as a plausible future field. Giving it a home now costs two
lines.

`lower_module` looks each parameter up in the frozen table:

```rust
let ty = table.get(&p.span).cloned().ok_or(LowerError::Untyped(p.span))?;
```

A missing entry is `LowerError::Untyped` — loud, not silent. That is the
recorder-totality tripwire for this new key class, and it is asserted by test (§7.2).

---

## §4. Back end: emission

### §4.1 Mangling — everything, uniformly

```rust
fn mangle(name: &str) -> String { format!("elya_{name}") }
```

Every Elya function gets the prefix. Prefixing is injective, so no two Elya names
collide, and no Elya name can collide with the three symbols the back end generates or
imports: `@main` (the shim), `@printf` (external), `@.fmt` (the format string).

`main` mangles to `elya_main` — byte-identical to the name the back end already
hardcodes, so the shim is unchanged. An Elya function literally named `elya_main`
mangles to `elya_elya_main` and does not collide.

Mangling everything, rather than only where a collision is possible, means there is no
rule to remember and no case analysis to get wrong later.

### §4.2 Two-pass emission

Mutual recursion means a body can call a function whose body has not been emitted.
Emission is therefore two passes over `core.fns`:

**Pass 1 — declare.** For each function: build the LLVM function type from `repr_ty`
of each parameter type and of `body.ty`; `module.add_function(&mangle(&f.name), ty,
None)`; `set_call_conventions(TAILCC)`. Record it in a `HashMap<String,
FunctionValue>`. A name inserted twice is `Unsupported("duplicate top-level
function")` — belt and braces, so that whatever the front end does about duplicate
declarations, the back end never silently emits two functions under one symbol.

**Pass 2 — emit bodies.** For each function: append an entry block, seed the value
environment from `func.get_nth_param(i)` under each parameter's name, then lower the
body in tail position (§4.3).

```rust
/// LLVM's `tailcc`. Value from llvm/IR/CallingConv.h: `Tail = 18`.
const TAILCC: u32 = 18;
```

Every function is `tailcc` and every Elya call site sets `tailcc`. Uniformity is what
makes `musttail`'s convention-match requirement true by construction rather than by
case analysis.

### §4.3 The tail/value split

`musttail` must be *immediately* followed by `ret`. The existing `If` lowering ends at
a join block with a `phi`, which cannot host a `musttail` call — so tail position has
to be threaded through lowering rather than discovered locally.

Two functions, not one:

```rust
fn lower_expr(...) -> Result<IntValue<'ctx>, CodegenError>   // produces a value
fn lower_tail(...) -> Result<(), CodegenError>               // emits a terminator
```

`lower_tail` by node kind:

| Kind | Emission |
|---|---|
| `If(c, t, e)` | condition via `lower_expr`; conditional branch; **`lower_tail` each arm into its own block — no join block, no `phi`.** Each arm terminates itself. |
| `Let(x, v, b)` | value via `lower_expr` (not tail); bind; `lower_tail` the body; restore any shadowed binding. |
| `App(Var(f), args)` | arguments via `lower_expr`, left to right; `build_call`; on the call site, `set_call_convention(TAILCC)` and `set_tail_call_kind(MustTail)`; then `build_return` immediately. |
| anything else | `let v = lower_expr(…)?; build_return(Some(&v))` |

`lower_expr` gains one case — `App(Var(f), args)` as an ordinary call, `tailcc`
convention, **no** tail-call kind — and one improved error: a `Var` that is not in the
environment but *is* a declared function is `Unsupported("function used as a value")`
rather than the generic "unbound var".

**Coverage note.** Routing `main`'s body through `lower_tail` means the top-level `if`
in several 5b-2 corpus programs no longer emits a `phi`. The `phi` diamond stays
exercised by the `nested_if` and `predicates` programs, whose `if`s sit in `let`-value
position and are therefore lowered in value mode. The 5b-2 corpus keeps both paths
covered, which is why it is re-run unchanged rather than replaced.

### §4.4 Why `musttail`'s requirements hold by construction

1. **Return types must match.** Inference unifies a tail expression's type with the
   function's return type, and `repr_ty` is a function of the Elya type. The LLVM
   return types are therefore equal.
2. **Calling conventions must match.** Every Elya function and every Elya call site is
   `tailcc` (§4.2).
3. **The call must be immediately followed by `ret`.** `lower_tail`'s `App` arm emits
   the call and the return adjacently; nothing can be inserted between them.
4. **Signatures may differ.** Probe modules D and J prove `tailcc` + `musttail` is
   arity- and type-blind at the verifier (§2, Finding 1).
5. **The argument area must not grow past the win64 gap.** Guaranteed by the ≤5 cap
   (§5.1), enforced before emission begins.

---

## §5. The refusal set

All four are asserted by test. Refusals shrink the set of programs both back ends
accept, and so are fidelity-preserving (§0).

`validate_module` becomes a whole-module scan, run **before any emission**, in this
order.

### §5.1 More than five parameters

`Unsupported("function takes more than five parameters")`.

This must be a pre-emission scan rather than a check at the point of use, because the
failure it prevents is LLVM's uncatchable `report_fatal_error` (§2, Finding 2). By the
time emission reaches a call site it is too late to produce a diagnostic.

### §5.2 An unrepresentable type anywhere in a signature

`repr_ty` maps `Int` to `i64` and `Bool` to `i1` and refuses everything else, including
`Ty::Var(_)`. Applied to every parameter type and every body type, it becomes **the
polymorphic-function refusal**: `fn id(x) { x }` generalizes to `∀a. a → a`, its
recorded parameter type is a `Ty::Var`, and the module is refused by name. It is also,
for free, the refusal for `Float`, `Str`, `Unit`, function-typed parameters
(`fn apply(f, x) { f(x) }` is refused at `f`'s `Ty::Fn`, not at the call site), and
every type N4 onward will introduce.

**Whole-module strictness is deliberate.** Every function must be representable, even
one `main` never calls. The alternative — emitting only functions reachable from
`main` — is a new analysis pass and is out of scope. The accepted cost is real and
worth stating: a file containing an unused polymorphic helper cannot `elya build`,
even though `elya run` handles it. Relaxation is strictly additive and is tracked as
**T2** (§8).

### §5.3 A callee that is not a `Var`

`Unsupported("computed callee")`. Witness: `(fn(x) { x + 1 })(3)` — the surface
language has lambda expressions (4b-1) and the parser's postfix-call loop applies to
any atom, so this parses to `App(Lambda(…), …)`. A hand-built `CoreModule` remains
available as a second instrument, following the pattern the codegen crate's inline
tests already use.

### §5.4 A `Var` callee that is not a declared top-level function

`Unsupported("callee is not a top-level function")`, with the companion
`Unsupported("function used as a value")` for a function name in value position
(`let f = add3`).

Together §5.3 and §5.4 draw the N5 boundary: N2 compiles calls to statically known
functions, and says so.

### §5.5 `main` still returns `Int` — and only `main`

`require_int` refuses a `Bool`-bodied `main` with `Unsupported("non-Int value")`,
because `@elya_main`'s signature and the shim's `%lld` both say `i64`. That rule
survives N2 unchanged, but its *scope* narrows: today it applies to the only function
there is; from N2 it applies to `main` alone.

Every other function may return `Bool` freely — `bool_across_a_call` (§7.1) depends on
it. Applying `require_int` per-function would refuse ordinary predicates and is the
most likely way to implement this section wrong.

---

## §6. Fidelity: what is preserved, and the one thing that is not

### §6.1 Preserved — answers

For every program both back ends accept, native output equals what the evaluator
computes. This is not a remembered rule; it is the differential test 5b-2 built, and
N2's corpus is added to it (§7.1).

### §6.2 Preserved — the tail-call bound, by a different mechanism

Both back ends run tail calls in bounded space, but the guarantees have different
shapes and different failure signals:

| | `elya run` | native |
|---|---|---|
| Mechanism | CEK machine, TCE by construction | LLVM `musttail` |
| Enforced | at test time, `peak_kont_depth() <= K_MAX` | at build time, by the LLVM verifier |
| Regression looks like | a `K_MAX` assertion failure | a build failure, or `STATUS_STACK_OVERFLOW` in the deep corpus program |

### §6.3 Limitation L1 — the machine stack is not the Kont stack

**Space fidelity is not preserved for non-tail recursion, and N2 does not attempt to
preserve it.**

`elya run` grows an explicit continuation stack, allocated on the heap and bounded by
available memory. A native binary grows the machine stack, bounded by the thread's
stack reservation — 1 MiB by default on Windows. A deep *non-tail* recursion that the
evaluator completes will die natively with `STATUS_STACK_OVERFLOW`. (The two bounds
differ structurally, by construction of the two designs; the exact depth at which they
part company is not measured, and N2 does not depend on knowing it.)

The differential harness compares **answers, not resource behavior**. It cannot see
this divergence and is not asked to. The consequence is a constraint on the corpus,
stated here so it is a design decision rather than an accident of test selection:
**non-tail recursion in the corpus is deliberately shallow.**

This is accepted rather than fixed because both closures are worse than the gap. A
stack-depth check in every native prologue is a per-call cost paid by every program to
serve almost none. Heap-allocated frames are a different execution model, not a back
end. Nor is it measured by a test: a test that deliberately overflows a process to
demonstrate a non-guarantee costs runtime and asserts nothing anyone relies on.

L1 becomes revisitable if N4's heap runtime ever grows a stack-limit check. Not
before.

---

## §7. Proof

### §7.1 The corpus

Eight programs, each aimed at one thing calls can get wrong. Every one runs through
**both** existing harnesses: the direct assertion (exit status, stdout, empty stderr)
and the differential check against `eval_main_int`.

| Tag | Program | Expected | Proves |
|---|---|---|---|
| `two_functions` | `fn add3(x) { x + 3 }` / `pub fn main() { add3(4) }` | `7` | declaration, call, parameter binding |
| `five_params` | a 5-parameter function summing its arguments | `15` | the cap boundary is *exercised*, not only refused |
| `distinct_envs` | two functions each with a parameter named `x`, one shadowed by a `let` | `50` | environments are per-function; shadowing survives a call |
| `call_in_operand_position` | `dbl(dbl(3)) + dbl(1)` | `14` | non-tail calls as operands |
| `bool_across_a_call` | a `Bool`-returning function used as an `if` condition | `1` | `i1` crosses the call boundary |
| `shallow_non_tail_recursion` | `sum(100)` | `5050` | ordinary recursion — deliberately shallow, per L1 |
| `deep_self_tail_recursion` | `down(1000000)` | `0` | **the `musttail` proof, self** |
| `deep_mutual_tail_recursion` | `ev`/`od` at 1e6, wrapped: `if ev(1000000) { 1 } else { 0 }` | `1` | **the `musttail` proof, mutual** |

The last two mirror `tests/tce.rs`'s two evaluator loops (`down(1000000)`, and
`ev`/`od` at 1e6), whose functions are arity 1 and therefore inside the cap. The mutual
pair returns `Bool`, so `main` wraps it in an `if` — per §5.5 the wrapping is required,
and it also makes the answer a number the shim can print. Running both through the
differential check costs a few seconds of evaluator time and buys agreement between the
two back ends *at depth*.

Bool literals are `True`/`False` — lowercase lexes as an identifier and dies at E0200
in the front end, before any LLVM code runs.

### §7.2 The tail-call proof is black-box, and its failure signal is named

The assertion is the ordinary one — exit status success, correct stdout, empty stderr.
What makes it a tail-call proof is that the *failure* is distinct and diagnosable
rather than a generic crash:

```rust
/// Windows STATUS_STACK_OVERFLOW. Observing it from a corpus binary means one
/// thing: a call that must have been eliminated was not. Inert on other platforms.
const STACK_OVERFLOW: i32 = 0xC00000FDu32 as i32;   // -1073741571
```

The runner checks for it first and panics with a message naming the cause, so a future
regression reads as "the tail call was not eliminated" rather than "the binary
crashed". Nothing branches on the code beyond that diagnosis.

Recorder totality for the new key class is asserted directly: for every corpus
program, lowering succeeds — no `LowerError::Untyped` arises from a parameter span.

### §7.3 Existing tests that must move

- `corpus_lowers_to_core_through_the_real_pipeline` asserts `core.fns.len() == 1`.
  N2's corpus is multi-function; the assertion is scoped to the single-function corpus
  or relaxed.
- The `emit_ir` smoke corpus inside `crates/codegen/src/lib.rs` gains the new programs,
  as it did in 5b-2.
- The 5b-1 and 5b-2 corpora run **unchanged**. Any edit to them is a regression, not a
  migration.

---

## §8. Out of scope, and the obligations this slice creates

**Deferred to later arc nodes**, all refused by name today: `Match` and ADTs and any
heap value (N4); lambdas, closures, function values, and higher-order calls (N5);
strings and `io.println` (N6); runtime polymorphism (N7); effects, `handle`, `resume`
(N8). `Float` and `Unit` are refused by `repr_ty`. `Div`/`Rem` remain refused on the
5b-1 §1.1 rationale.

**New tracked obligations:**

- **T1 — the ≤5 cap is a back-end limit leaking into the language's buildable
  subset.** A six-parameter function is perfectly good Elya that `elya run` executes
  and `elya build` refuses. Relaxation paths, in preference order: pad the callee's
  outgoing argument area to the caller's at the call site; or replace the flat cap with
  the measured `callee ≤ 5 || caller ≥ callee` predicate. Build against real demand,
  and re-measure before extending the claim past arity 8.
- **T2 — whole-module representability.** One unused polymorphic helper blocks
  `elya build` for the whole file (§5.2). Relaxation: emit only functions reachable
  from `main`.
- **T3 — lambda parameters still carry no types in Core** (§3.2), an asymmetry with
  `CoreFn`. N5 closes it.
- **L1 — the space-fidelity divergence** (§6.3). Not an obligation to fix; a
  limitation to remember when reading a native stack overflow.

---

## §9. Completion checklist

- [ ] Parameter types are recorded at their `Spanned<Param>` spans and survive zonking
- [ ] `CoreFn.params` carries `CoreParam { name, ty }`; no separate return-type field
- [ ] A missing parameter type is `LowerError::Untyped`, asserted absent on the corpus
- [ ] Every emitted function is mangled `elya_*` and carries `tailcc`
- [ ] Emission is two-pass; mutual recursion resolves; duplicate names are refused
- [ ] Tail position is threaded: `lower_tail` emits terminators, `lower_expr` values
- [ ] A tail-position `If` emits no join block and no `phi`
- [ ] Tail calls emit `musttail` + `tailcc`; non-tail calls emit `tailcc` only
- [ ] ≥6 parameters refused before emission, by whole-module scan
- [ ] A polymorphic function refuses the module (`Ty::Var` via `repr_ty`)
- [ ] A non-`Var` callee refuses, witnessed by an immediately-invoked lambda
- [ ] A function name in value position refuses with its own message
- [ ] `require_int` applies to `main` alone; a `Bool`-returning helper compiles
- [ ] All eight corpus programs pass the direct and the differential harness
- [ ] Mutual recursion at 1e6 exits 0 with the right answer
- [ ] `STATUS_STACK_OVERFLOW` is diagnosed by name, not reported as a generic crash
- [ ] The 5b-1 and 5b-2 corpora pass unchanged
- [ ] No IR snapshots anywhere; no test skips
- [ ] Full five-stage gate green, both configurations, default parallelism
