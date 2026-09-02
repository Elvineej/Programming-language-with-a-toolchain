# Slice 5b-4 (arc node N4) — Native ADTs and Match, the heap threshold

**Goal.** A module with algebraic data types compiles to a native binary:
constructors allocate heap objects through one runtime symbol (`elya_alloc`),
and `match` becomes a sequential tag-dispatch chain that reads constructor
fields back and joins every arm's result through a single `phi`. The widening
is proven by execution — construct → match → extract an `Int` — keeping N6
(strings) out of the subset entirely and dropping straight into the existing
differential harness.

**Arc position.** N1 (arith, 5b-1) → N3 (control flow, 5b-2) → N2 (functions +
guaranteed tail calls, 5b-3) → **N4 (this slice)** → N5 (closures) → N6
(strings) → N7 (runtime polymorphism). N4 is the **heap threshold**: the first
node whose values do not all fit a machine register.

**Status.** Design drafted from the fork map recorded in `next-slice-decision.md`
(2026-09-02); awaiting review before an implementation plan.

---

## §0. Inherited ground rules

From 5b-1/5b-2/5b-3, not re-argued here:

- **Proof is execution.** No IR snapshot anywhere; every claim is proven by
  compiling, linking, running the binary, and reading exit status / stdout /
  stderr. No test may skip.
- **Semantic fidelity** (5b-1 §3.4, extended 5b-2 §4.1). Native codegen must be
  neither *more*- nor *less*-undefined than the tree evaluator. A clean
  `Unsupported` refusal removes a program from *both* back ends' accepted sets,
  so it never violates the rule; silently mis-compiling does.
- **Refused by name, never mis-compiled.** Every rejection is a `CodegenError`
  carrying a specific message.
- **Gate shape.** `cargo fmt --all` (write mode) before every gate; the gate is
  the five-stage `scripts/check.ps1` in both configurations; `CARGO_INCREMENTAL=0`
  on every cargo invocation.

---

## §1. The value widening: a pointer type joins the two widths

The back end today represents exactly two value widths — `Int → i64` and
`Bool → i1` — and `repr_ty` refuses everything else, `Ty::Con` included. N4
widens the representation to three:

| Elya type | LLVM representation |
|---|---|
| `Int` | `i64` (unchanged) |
| `Bool` | `i1` (unchanged) |
| `Ty::Con(..)` | **`ptr`** (a pointer to a heap object) |

`repr_ty(Ty::Con(..))` therefore changes from "unrepresentable type" to
`ctx.ptr_type(AddressSpace::default())`. Since the two integer widths and the
pointer are distinct `BasicValueEnum`s, no wrapper enum is needed; the existing
`lower_expr -> IntValue` shape widens to `BasicValueEnum` (a pointer is not an
`IntValue`), which is the mechanical heart of the slice.

**Field storage rule.** Every constructor field occupies one 8-byte word:

- an `Int` field stores its `i64` bits;
- a `Bool` field stores `0`/`1` (one word, zero-extended — `repr_ty` still makes
  `Bool` an `i1`, widened only at the store);
- a field of ADT type stores a **pointer** to another object. Recursive types
  (`type List = Nil | Cons(Int, List)`) force this — an inline `List` field is
  infinite — so boxing is not a choice here, it is the only representation.

---

## §2. The heap and the object header (2-ii)

### §2.1 `elya_alloc`

A small runtime library provides exactly one allocation symbol:

```
ptr elya_alloc(i64 words)     // allocates words*8 bytes, zeroed, returns a ptr
```

It is an **external symbol** declared in the module and resolved by the same
`clang` invocation that already resolves `printf` — mirroring the print shim
(5b-1 §3.5). The runtime is `malloc`-backed and **never frees**: allocate-don't-
collect. A collector cannot be exercised without recursion that crosses function
boundaries carrying live heap values, which N4's shallow corpus does not do; and
a header bolted on *after* a collector exists would be a rewrite, so the header
is designed now (§2.2) even though no collector reads it.

### §2.2 Object layout and the header (designed now)

Every ADT value is a pointer to a block of `1 + arity` words:

```
[ tag ] [ field 0 ] [ field 1 ] ...   (the ptr points AT the tag word)
```

- **`tag`** is the constructor's **index** within its type's declaration
  (`Zero`/`Succ` → `0`/`1`), stored as one `i64` word.
- **The pointer points at the tag**, not past it, so a future collector can
  reach the header from a bare object pointer.
- **Forward-compatibility.** The tag word uniquely identifies a (type,
  constructor) pair, because constructor names are globally unique. A future
  mark-sweep collector therefore has everything it needs by keying a *type
  table* off the tag — size (arity), plus which fields are pointers (the field
  types of §1). No mark bit or size slot is reserved today; the *decision* that
  the object is self-describing via its tag is what makes a later collector
  additive rather than a layout rewrite.

**Nullary constructors** (`None`, `Nil`) allocate a one-word `[ tag ]` object,
uniformly — no sentinel-pointer optimization in this slice.

---

## §3. The core reach

### §3.1 `CoreModule` gains `types`

```rust
pub struct CoreType {
    pub name: String,
    /// Constructors in declaration order — the index IS the tag (§2.2).
    pub ctors: Vec<CoreCtor>,
}
pub struct CoreCtor {
    pub name: String,
    /// One type per field; len == arity. Drives the store/load kind (§1) and
    /// the allocation size (1 + len).
    pub fields: Vec<Ty>,
}
```

`CoreModule` becomes `{ fns, types }`. Field types reuse the existing `Ty` form
verbatim — a `Cons` field of type `List` is `Ty::Con("List", [])` — assembled
from the ADT declaration's elaborated field signatures. Because the corpus is
ground (§6), each field type is a concrete `Int`/`Bool`/`Con` and no `Ty::Var`
reaches the back end here.

### §3.2 `CoreKind::Ctor`

```rust
CoreKind::Ctor(String, Rc<[CoreExpr]>),   // name + field expressions
```

Construction is now **syntactically distinct from application**. Before, the
lowerer produced `App(Var("Some"), [x])` for `Some(x)`, and the back end could
not tell a saturated constructor call from a function call — the ambiguity the
slice kills. After, a constructor application lowers to `Ctor("Some", [x])`, and
a bare nullary constructor lowers to `Ctor("None", [])` (previously `Var`).

### §3.3 The lowerer change

`src/core.rs::lower_expr` builds (from `Decl::Type`) a set of constructor names
and, on seeing a `Var`/`Call` whose callee is a constructor, emits `Ctor` with
the field expressions lowered. The module's `types` vector is assembled from the
same declarations. This is the slice's one genuine reach into the front end; the
`Ctor` node's span already carries `Ty::Con(name, args)` through the 5a-1
recorder, so no inference or recorder change is needed.

**Ripple:** `tests/core_lowering.rs` snapshots move (`App(Var(..))` → `Ctor`),
and every existing match over `CoreKind`/`CoreModule` gains the new arms.

---

## §4. The back end

### §4.1 Construction

`lower_expr` on `Ctor(name, fields)`:

1. size = `1 + fields.len()`;
2. `p = call elya_alloc(size)`;
3. `store tag` at `p + 0` (the constructor's index, looked up in `types`);
4. for each field in order, lower it and `store` at `p + 8*(i+1)` — an `Int`
   stores its `i64`, a `Bool` zero-extends to a word, an ADT field stores the
   pointer produced by lowering the nested `Ctor`/expression;
5. return `p` as the value.

### §4.2 `match` — the sequential chain

`lower_expr` on `Match(scrutinee, arms)` lowers the **scrutinee once** to a
pointer `s`, then builds a decision chain in arm order, generalising N3's
diamond (5b-2) from one condition to a sequence:

- A shared **join block** owns a single `phi` of the scrutinee's result type.
- For a constructor arm `Ctor(c, _)`: emit a test block that `icmp eq` the loaded
  `tag` (`load i64 from s`) against `c`'s index; branch to the arm body on
  equal, to the *next* test on not-equal. The arm body unpacks each field with
  `getelementptr` + `load` (the field kind from §1), binds the pattern variables
  in the environment, lowers the body, and branches to the join.
- A `Wild`/`Var` arm needs no test (it always matches) and is the chain's
  terminal arm.

After each arm body is lowered, the builder sits at the end of *that* block —
not the join — so the `phi` incoming block is read back with
`get_insert_block()` (the exact 5b-2 §3.2 trap, now with N incoming edges
rather than two). After the last constructor arm's not-equal branch, the chain
falls into the **trapping default block** (§5).

### §4.3 `require_int` on `main` is unchanged

The §5.5 rule — `require_int(&main.body.ty)` — is untouched, so an ADT-valued
`main` is already refused. This is what confines the corpus to "construct →
match → extract an Int" and keeps a native `main`'s print convention (`%lld`)
intact: the shrink never has to decide how to print an ADT.

---

## §5. The default block traps — the reflex error to guard

The chain's fall-through block (reached only if the scrutinee's tag matches no
declared constructor) must **never be `unreachable`**. It traps deterministically:
a call to a runtime `elya_match_fail()` that writes a fixed message to stderr and
exits non-zero.

The reason is the §3.4 reflex, stated as a rule N4 must not break: the **tree
evaluator returns a defined runtime error on a failed match**, and `unreachable`
is LLVM UB, so an `unreachable` default would make native *more*-undefined than
the evaluator. Exhaustiveness checking (4a) already makes the fall-through
unreachable *for well-typed programs* — it is `E0430` at compile time — but the
back end does not get to lean on that and de-optimise into UB. The trap is cheap,
defined, and makes a future mismatch between the checker and codegen surface as a
deterministic non-zero exit rather than a crash that could be anything.

---

## §6. Scoping and deferrals

- **N7 (runtime polymorphism) — deferred, and committed by construction.** A
  single ground `main` presents no `Ty::Var`, so `repr_ty`'s existing
  "unrepresentable type" refusal keeps firing on any polymorphic node. ADTs alone
  therefore commit nothing about the monomorphize-vs-box question; it stays open.
- **Parametric ADTs deferred — the corpus is monomorphic-ADT-only.** `List(a)`
  used at `List(Int)` is explicitly out of scope: a ground `List(Int)` would force
  field-type substitution (declaration `List(a)` → use-site `List(Int)`) into the
  lowerer — a second mechanism landing alongside the heap, the tag encoding, the
  pointer representation, the fold widening, and the match compiler. That is
  stacking. It lands with parametric-ADTs-in-codegen, likely alongside N7.
  Nothing structural is lost: `Nat = Zero | Succ(Nat)` still proves the
  recursive-occurrence-is-a-pointer case, which is the load-bearing
  representation fact.
- **Maranget / `exhaust.rs` reuse — deferred as a later optimisation.** The
  sequential chain is correct for every well-typed match; the compiled arms are
  a source-order linear scan. Re-using exhaustiveness to reorder arms, drop the
  trap, or build a jump table is a later pass, not this slice.
- **GC collection — deferred.** §2.1 is allocate-don't-collect; the header is
  the only GC-facing decision made now.
- **N6 (strings) out entirely.** The corpus never constructs a `Str`; the
  existing "unrepresentable type" refusal stands.
- **Literal-pattern `match` (on `Int`/`Bool` scrutinees)** is folded into the
  same chain where it costs nothing (an `icmp` against the literal instead of
  against a tag), but is **not a focus**; the corpus is ADT matches.

---

## §7. Corpus and the differential harness

The proof is the existing direct + differential pair, over ADT programs shaped
**construct → match → extract an Int**. `require_int` (§4.3) already rules out
an ADT-typed `main`, so every program prints an `Int`. Representative programs:

- option/result style: build `Some(n)`/`None`, match, extract the payload or a
  fallback;
- a recursive list (`Nil`/`Cons`), fold to an `Int` by tail recursion;
- a multi-constructor type with arity ≥ 2, proving the tag dispatch and the
  N-armed `phi` (not just a Bool-style two-way).

Each is run natively and against `elya run`; the answers must agree. Native exit
status and stdout are read literally (per §0), and the `STATUS_STACK_OVERFLOW`
diagnosis from 5b-3 is retained so a missing tail-call guarantee stays named.

---

## §8. Obligations

Carried over, still open: **T1** (≤5 arity cap), **T2** (whole-module
representability), **T3** (lambda parameters carry no types in Core). New in N4:

- **T4 — the header is a promise to future GC.** The tag-word design (§2.2) is
  the layout decision a collector will inherit; re-validate it against the
  language-design memory model (§2) before any mark-sweep work.
- **T5 — the sequential chain is an unoptimised correctness floor.** Maranget
  reuse (§6) is the optimisation path; do not fold it into this thin cut.
- **T6 — literal-pattern match is present but unfocused**; assert its behaviour
  if it is enabled, or refuse it by name rather than leave it half-tested.

---

## §9. Completion checklist

- [x] `CoreModule.types` + `CoreKind::Ctor` land; lowering emits `Ctor`, never `App(Var(ctor, ..))`
- [x] `repr_ty(Ty::Con)` → `ptr`; `lower_expr` returns `BasicValueEnum`
- [x] Construction allocates via `elya_alloc` and stores tag + fields per §4.1
- [x] `match` lowers to a sequential chain; the join `phi` carries N incoming edges, read back with `get_insert_block()`
- [x] The default block traps via `elya_match_fail`; no `unreachable`
- [x] The runtime library provides `elya_alloc` (and `elya_match_fail`), linked by clang
- [x] ADT corpus passes the direct and the differential harness (construct → match → extract Int)
- [x] `require_int` on `main` unchanged; an ADT-valued `main` stays refused
- [x] The 5b-1/5b-2/5b-3 corpora pass unchanged
- [x] No IR snapshot anywhere; no test skips
- [x] Full five-stage gate green, both configurations