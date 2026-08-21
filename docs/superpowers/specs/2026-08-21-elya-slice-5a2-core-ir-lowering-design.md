# Elya — Slice 5a-2 Design Specification: Core IR Datatype & Type-Directed Lowering

**Goal:** Introduce the first real Core IR datatype (`src/core.rs`) and a
type-directed lowering that consumes 5a-1's frozen `Span → Ty` table and produces a
Core tree with the **type inline on each node (Shape C)** — proven by a typed-Core
snapshot over the reused 5a-1 corpus that asserts the type materialized on each AST
node in 5a-1 is the same type now carried on the corresponding Core node. That
equality *is* lowering-preserves-types.

**Arc:** Core IR (Slice 5) — the compilation substrate under *both* native codegen
and precise linear ownership. This is sub-slice **5a-2** ("slice 1b"): the Core
datatype and lowering, built on the typed-inference feeder proven in **5a-1**
([spec](2026-08-21-elya-slice-5a1-core-ir-typed-inference-design.md), commit
`33baacc`).

**Non-negotiable boundary:** 5a-2 builds Core and lowers *the corpus subset* into it,
and snapshots the result. It does **not** execute Core (no evaluator, no CEK
re-point), does **not** wire lowering into `front_end`, does **not** re-platform
exhaustiveness or `affine`, and does **not** reach into `unify`/`unify_row`/`bind`/
`instantiate`/`generalize`/`resolve` — the table it consumes is already zonked and
frozen. See §6.

---

## 0. How to read this document

5a-1 stopped the compiler throwing types away: inference now persists a zonked
`Span → Ty` table. 5a-2 gives those types a home — a Core node — and proves the
type *rode across the lowering* unchanged.

The one seam that carries real risk is **where lowering meets the polymorphic-var
form** (§3): a node inside `fn id(x) { x }` has type `Ty::Var(_)`, and a lowering
written under an unexamined "types are ground by now" assumption would monomorphize
it, substitute `Error`, or panic. §5 makes that exact case an explicitly asserted
tooth. Everything else in the slice is a mechanical structural fold.

**The decoy we are *not* building (read this first).** 5a-1's §11 pre-listed
"exhaustiveness re-platformed onto Core" as a 5a-2 item. **That is dropped.**
Exhaustiveness ([exhaust.rs:12-14](../../../src/exhaust.rs#L12-L14)) reads only
pattern structure and constructor arity and **ignores every type field**. Running it
on Core would validate the one dimension that is *not* at risk (patterns) and say
nothing about the dimension that is (types riding across lowering). It is a decoy
proof. The honest proof is the typed-Core snapshot (§5). Exhaustiveness-on-Core moves
to a later slice (§11), where it earns its keep as the first *consumer* re-pointed at
Core, not as a proof of this lowering.

## 1. Scope

**In:** `src/core.rs` — a minimal Core datatype (§2) covering exactly the constructs
the 5a-1 corpus exercises, with the mandatory type-variable form (§3); a type-directed
`lower` (§4) consuming the frozen table with types inline (Shape C); a raw-`Ty`
accessor on inference (§7); the typed-Core snapshot corpus (§5) as the deliverable,
reusing the 5a-1 programs.

**Out (later slices):** re-platforming exhaustiveness onto Core; re-pointing the CEK
machine at Core (executing Core); re-platforming `affine` onto Core; native codegen;
wiring lowering into `front_end`; lowering the full AST surface
(`If`/`Handle`/`Resume`/`Qualified`/`Unary`/`Float`). None are touched here (§11).

**The honesty line (must hold, mirroring 5a-1/4d-2):** a green 5a-2 means "Core exists,
carries per-node types, and lowering preserves them over the corpus subset." It does
**not** mean Core executes, that lowering is total over the language, or that any
existing consumer (eval, exhaustiveness, affine) runs on Core. Exit criteria (§12) may
not claim execution, full-surface lowering, or a re-platformed consumer.

## 2. The Core datatype (minimal, corpus-derived)

Core is defined by the *union of constructs the 5a-1 corpus actually contains* — no
more. Surveying the seven surfaces plus the two invariant programs
([tests/typed_inference.rs](../../../tests/typed_inference.rs)) yields: integer and
boolean literals; variable references (locals, top-level function names, and nullary
constructors like `Tok`); n-ary application; the `+` binary primitive; a lambda;
`let`-sequencing; and `match` with constructor/variable/wildcard/literal patterns.
Nothing in the corpus is an `If`, `Handle`, `Resume`, `Qualified`, `Unary`, or
`Float`, so none appear in Core.

```rust
// src/core.rs  — populates the reserved layer-5 slot (see §6).
use std::rc::Rc;
use crate::ast::BinOp;      // the primitive operator set is reused verbatim
use crate::span::Span;      // provenance only — NOT a type key (see below)
use crate::types::Ty;       // the type form is reused verbatim (§3)

/// One Core expression: its source provenance, its inline type (Shape C), and shape.
pub struct CoreExpr {
    pub span: Span,          // provenance (diagnostics + the §5 cross-check); see note
    pub ty: Ty,              // the type materialized in 5a-1, now carried inline
    pub kind: CoreKind,
}

pub enum CoreKind {
    Lit(CoreLit),
    Var(String),                              // local, top-level fn, or nullary ctor
    App(Rc<CoreExpr>, Rc<[CoreExpr]>),        // callee + args (function/ctor/effect-op)
    Prim(BinOp, Rc<[CoreExpr]>),              // `+` etc. — no separate ctor-app node
    Lambda(Rc<[String]>, Rc<CoreExpr>),       // uncurried, block flattened into body
    Let(String, Rc<CoreExpr>, Rc<CoreExpr>),  // binding spine from block-flattening
    Match(Rc<CoreExpr>, Rc<[CoreArm]>),
}

pub struct CoreArm { pub pat: CorePat, pub body: CoreExpr }

pub enum CorePat {
    Wild,
    Var(String),
    Ctor(String, Rc<[CorePat]>),
    Lit(CoreLit),
}

pub enum CoreLit { Int(i64), Bool(bool), Str(String), Unit }

pub struct CoreFn { pub name: String, pub params: Rc<[String]>, pub body: CoreExpr }
pub struct CoreModule { pub fns: Vec<CoreFn> }
```

**Structural sharing via `Rc` (confirmed against the codebase).** Every Core child
pointer is `Rc<CoreExpr>` / `Rc<[CoreExpr]>`, never `Box` or an owned `Vec` of nodes.
This is not a novel choice: the AST *already* Rc-wraps every child pointer —
`Rc<Spanned<Expr>>`, `Rc<[Spanned<Expr>]>`, `Rc<Spanned<Block>>`
([ast.rs:167-202](../../../src/ast.rs#L167-L202)). Core mirrors the established
pattern, so a subtree is a cheap refcount clone, not a deep copy. This matters because
**every later Core consumer inherits it**: a Core type-checker, native codegen, and the
eventual CEK-on-Core will all clone and re-root subtrees, and `Rc` keeps that O(1). The
decision is settled here so no later consumer bakes in deep clones.

**Two deliberate thinnesses, stated so they are choices, not omissions:**

- **No constructor-construction node.** In the corpus, constructors appear as values
  (`Tok`, nullary) or inside patterns — never as an *applied* construction expression.
  So `Tok` lowers to `Var("Tok")` (typed `Con("Tok", [])`), and an applied constructor
  would be plain `App`. A dedicated construction node waits for a program that needs it.
- **No effect-control nodes.** Surface 4 *performs* `State(Int)` but never handles it
  (it is a no-`main` program; `E0420` fires only at `main`). The effect rides on the
  node's `Ty::Fn(_, EffectRow, _)`, so Core needs zero `Perform`/`Handle`/`Resume`
  nodes. **Consequence to state plainly:** Core in 5a-2 *cannot represent handlers* and
  therefore cannot lower any effect-*handling* program end to end. That is acceptable
  because Core is non-executed here; it is a real expressiveness gap, flagged in §11,
  not papered over.

**`CoreLit` breadth.** The corpus uses only `Int`/`Bool`, but the four scalar literals
form one cohesive node and mirror the four scalar `TyCon`s; `Float` — the one the corpus
never touches and which the AST already treats as pattern-hostile
([ast.rs:139-146](../../../src/ast.rs#L139-L146)) — is deferred. Adding `Str`/`Unit`
costs nothing and avoids an artificial two-variant enum.

**`span` is provenance, not a type key.** Carrying a `Span` on a Core node does **not**
reintroduce the AST-keyed side-table: the *type* is inline (Shape C), and the
`Span → Ty` map is discarded after lowering (§4). The span is retained for the same
reason the AST keeps spans — future diagnostics — and, in this slice, to make the §5
cross-check ("each Core node's type equals what `node_types` said") a literal, testable
assertion rather than a tautology-by-construction. A node whose type is *derived* rather
than *looked up* (the `Let` spine, §4) carries the originating statement's span for
provenance and is excluded from the lookup cross-check.

**ADT declarations are not re-homed.** `CoreModule` carries only lowered functions.
The constructor sibling sets that exhaustiveness needs live in `Decl::Type` and are not
duplicated into Core — because exhaustiveness-on-Core is out of scope (the decoy, §0).
When a real Core consumer needs the type environment, that is its slice's work.

## 3. The type-variable form (stay-polymorphic — 5a-1 §4, now load-bearing)

5a-1 settled that Core **stays polymorphic**: a node inside a polymorphic body zonks to
a bare `Ty::Var`, and the table legitimately contains variables, not only ground types
([5a-1 §4](2026-08-21-elya-slice-5a1-core-ir-typed-inference-design.md)). 5a-2 is the
first place that decision is *acted on*.

**Core reuses `types::Ty` verbatim as its inline type.** No new type representation is
introduced. In particular the inline `ty` may be `Ty::Var(u32)`, and — because the value
comes straight from the frozen, already-zonked table — bound variables are already
collapsed to their solutions; only genuinely-generalized variables survive as
`Ty::Var`. The latent effect row rides for free inside `Ty::Fn(_, EffectRow, _)`, so an
effectful arrow (surface 4) carries its resolved `{State(Int)}` inline with no extra
Core machinery.

**No var canonicalization in 5a-2.** Raw `Ty::Var(n)` indices are inference-machine
artifacts, meaningful only relative to a shared render — exactly as in 5a-1. The §5
snapshot therefore renders the whole Core tree through **one shared `Names`**,
reproducing 5a-1's cross-node coherence. A standalone Core type-checker or codegen will
later want canonical per-scheme variables; staying-polymorphic says we do not pay that
here. The one property to *verify* (not assume) is snapshot determinism: we render, not
dump raw indices, and inference is deterministic with first-appearance letter
assignment, so the snapshot is stable.

## 4. Type-directed lowering (typed AST → Core)

`lower` is a structural fold over the typed AST. At each expression node it sets the
inline type from the frozen table and drops the span-keyed lookup:
`ty = table.get(&e.span)` — the type is placed inline, the map is never consulted again
after the tree is built.

**It performs no inference.** The table is frozen and zonked; `lower` clones `Ty` out
of it and never calls `resolve`, `unify`, `unify_row`, `instantiate`, or `generalize`.
"No Algorithm-J reach" therefore holds trivially — lowering has no access path into the
solver (§6).

**Direct lowering rules** (each result node's `ty = table[origin.span]`, `span =
origin.span`):

| AST node | Core node |
|---|---|
| `Expr::Int/Bool/Str/Unit` | `Lit(CoreLit::…)` |
| `Expr::Var(x)` | `Var(x)` (local, top-level fn, or nullary ctor) |
| `Expr::Call { callee, args }` | `App(lower(callee), args.map(lower))` |
| `Expr::Binary { op, lhs, rhs }` | `Prim(op, [lower(lhs), lower(rhs)])` |
| `Expr::Lambda { params, body }` | `Lambda(params.names, lower_block(body))` |
| `Expr::Match { scrutinee, arms }` | `Match(lower(scrutinee), arms.map(lower_arm))` |

**Pattern lowering** (`lower_pat`): `Wild → Wild`, `Var(x) → Var(x)`,
`Ctor{name,args} → Ctor(name, args.map(lower_pat))`, `Lit(l) → Lit(l)`. Patterns carry
no inline type (they need none; the corpus asserts on expression-node types only, and
binder types are not in the table — [5a-1 §3](2026-08-21-elya-slice-5a1-core-ir-typed-inference-design.md)).

**Block flattening (the one synthesized node).** A block `{ s₁ … sₙ tail }` becomes a
right-nested `Let` spine terminating in the lowered `tail`:
- `Stmt::Let { name, value }` → `Let(name, lower(value), rest)`
- `Stmt::Expr(e)` (non-tail) → `Let("_", lower(e), rest)` (discarded binding — no
  separate sequencing node is needed for the corpus)
- the block `tail` → the innermost body.

A fn body with no statements (e.g. `fn id(x) { x }`) has no spine: `CoreFn.body` is the
lowered tail directly. A `Let` node is **synthesized** — it has no single originating
AST *expression* — so its `ty` is **derived by propagation** (`ty = body.ty`), and its
`span` is the originating `let` statement's span (`Spanned<Stmt>`), used for provenance
only. Synthesized nodes are excluded from the §5 lookup cross-check; they are covered by
the whole-tree snapshot.

**The deferred surface is a typed boundary, not a panic.** `lower` returns
`Result<_, LowerError>`; the AST variants outside the 5a-2 subset
(`If`/`Handle`/`Resume`/`Qualified`/`Unary`/`Float`) map to a single arm returning
`Err(LowerError::Unsupported(&'static str))`, and a missing table entry returns
`Err(LowerError::Untyped(Span))`. Because lowering is test-only over the fixed corpus
(§6), neither error fires in practice — but encoding the subset boundary as a typed,
testable value keeps committed code panic-free and makes "this construct is not yet in
Core" an honest fact rather than a runtime landmine.

## 5. The deliverable — the typed-Core snapshot corpus

The snapshot **is** the proof of lowering-preserves-types, exactly as the 5a-1 corpus
was the proof of the feeder. It reuses the 5a-1 programs verbatim
([tests/typed_inference.rs](../../../tests/typed_inference.rs)), lowers each to Core,
and renders the Core tree — with each expression node's inline `ty` — through **one
shared `Names`**, then `insta`-snapshots it. New test file:
`tests/core_lowering.rs`; new snapshots under `tests/snapshots/`.

**Snapshot format (auditable, span-free-safe).** A typed S-expression mirroring
[`ast::pretty`](../../../src/ast.rs#L233), annotating each expression node with its
type: e.g. `fn id(x){x}` renders `(fn id (x) (var x : a))`, and a use site renders
`(app (var id : fn(Int) -> Int) (lit 5 : Int) : Int)`. Because the annotation is the
inline `Ty` rendered through the *same* `write_ty`/`Names` path 5a-1 uses, equal types
render to equal strings, so the snapshot is directly comparable to 5a-1's table lines.

**The headline asserted case (§3's risk, made a tooth).** Lower `fn id(x) { x }`, find
the Core body `Var("x")` node, and assert:

```rust
assert!(matches!(body.ty, Ty::Var(_)),
        "lowering monomorphized or errored a polymorphic node: {:?}", body.ty);
```

This is the explicit "lower a var-typed node → a Core node carrying the variable, not
monomorphized, not errored" case. It is where lowering meets the stay-polymorphic
decision, and where an assume-ground-types bug hides. It fails loudly on `Ty::Base(_)`
(wrongly monomorphized) and on `Ty::Error` (wrongly poisoned).

**The lookup cross-check (lowering-preserves-types, literally).** Walk every Core node
with a *direct* AST-expression origin (every kind except the synthesized `Let` spine)
and assert `node.ty == table[node.span]` — the type on the Core node is exactly the type
`node_types` assigned the AST node it came from. The provenance span (§2) makes this a
genuine regression assertion: a future refactor of `lower` that recomputes types instead
of copying them would drift and fail here.

**The derivation check (synthesized nodes, verified — not merely exempted).** Exempting
the `Let` spine from the lookup check must not leave it checked against *nothing*: a
derivation bug (a `Let` node given some type other than its body's) would otherwise
hide, since a synthesized node has no source node to look up. So each synthesized `Let`
node carries its own positive assertion — `let_node.ty == let_node.body.ty` — verifying
the propagation rule (§4) directly. Every Core node thus has exactly one origin-specific
proof: direct nodes are looked up against `node_types`, synthesized nodes are derived
from their child.

**Per-surface teeth (mirroring 5a-1, now on Core inline types):**

| # | Program | Assert on the Core tree |
|---|---|---|
| 1 | `fn add1(n) { n + 1 }` | no Core node carries a `Ty::Var` (fully monomorphic) |
| 2 | `fn id(x) { x }` | the body `Var("x")` node's `ty` is `Ty::Var(_)` (the headline tooth) |
| 3 | `id(5)`, `id(True)` | two `App` callee nodes carry distinct `fn(Int) -> Int` and `fn(Bool) -> Bool` |
| 4 | effectful `worker`/`use_it` | some node's rendered `ty` contains `State(Int)` — the row rode inline through `Ty::Fn`'s `EffectRow` |
| 5 | `let f = fn(x){x}` … | the lambda body node is `Ty::Var(_)`; the two applications are `Int`/`Bool` |
| 6 | `match` over `Option` | scrutinee, arm bodies, and the `Match` node each carry a type; the `Match` node is `Int` |
| 7 | `linear type Tok`, `let t = Tok` | the `Var("Tok")` node carries `Con("Tok", [])` |

**Cross-cutting invariants (mirroring 5a-1):** no internal inference token
(`%t`/`%r`/`%e`/…) may appear in any rendered type; two distinct variables in one
function render as two distinct letters through the shared `Names` (coherence).

## 6. Non-interference guarantees (explicitly confirmed, per request)

- **Core is built and snapshotted but non-executed → oracle green by
  non-interference.** 5a-2 adds no evaluator for Core and does not touch
  [eval.rs](../../../src/eval.rs) or the CEK machine. `eval` still runs the AST; the
  tree-walker ⇄ CEK cross-check is untouched, so it stays green *because it is
  unchanged*, not because anything was re-proven. Re-pointing the CEK at Core — where
  the oracle chain is genuinely re-proven — is a later slice (§11).
- **No Algorithm-J reach.** Lowering consumes a frozen, already-zonked table and clones
  `Ty` out of it. It never calls `resolve`, `unify`, `unify_row`, `bind`, `instantiate`,
  or `generalize`. Inference is byte-for-byte unchanged — the one addition (§7) is an
  additive raw accessor over the existing `infer_all(module, true)` path.
- **`front_end` untouched.** Lowering is *not* wired into the production pipeline
  ([lib.rs:30-42](../../../src/lib.rs#L30-L42)); it is exercised only by
  `tests/core_lowering.rs`, exactly as `infer_with_types` is a test-facing entry today.
  Production compilation and execution behave identically.
- **Exhaustiveness and `affine` untouched.** Neither is re-platformed onto Core (the
  decoy, §0). `exhaust::check(&module)` and `affine::check` continue to run on the AST.
- **Layering test does not churn — the slot is pre-reserved.** `core` is already mapped
  at layer 5 in [layering.rs:17](../../../tests/arch/layering.rs#L17). `src/core.rs`
  references only `ast` (layer 1), `types` (layer 4), and `span` (layer 0) — all
  strictly lower — and references neither equal-layer `exhaust`/`affine` nor higher
  `eval`, so `no_upward_module_references` passes with no edit to the test.

## 7. Pipeline & module changes

- **New module `src/core.rs`** — the datatype (§2), `lower_module(&Module,
  &BTreeMap<Span, Ty>) -> Result<CoreModule, LowerError>` and its expression/pattern
  helpers (§4), and `pretty_typed(&CoreModule, &mut Names) -> String` (or a
  free rendering fn) for the snapshot (§5). `pub mod core;` added to
  [lib.rs](../../../src/lib.rs) alongside the other modules.
- **New inference accessor (the raw table 5a-1 anticipated).**
  `pub fn infer_typed_table(session: &Session, module: &Module) -> (Vec<Diagnostic>,
  BTreeMap<Span, Ty>)` in [types.rs](../../../src/types.rs), a sibling of
  `infer_with_types` ([types.rs:1655](../../../src/types.rs#L1655)): it runs
  `infer_all(module, true)` and returns the zonked `BTreeMap<Span, Ty>` *raw* (not
  rendered to strings). `infer_with_types` and the two `want_types = false` callers are
  unchanged; no existing snapshot churns.
- **No `front_end` change** (§6). No CLI change.

## 8. Testing strategy

- **The typed-Core snapshot corpus** (§5) — the deliverable: one snapshot per reused
  5a-1 program plus the per-surface teeth, the headline var-carrying tooth, the lookup
  and derivation cross-checks, and the coherence/no-token-leak invariants. A lowering bug surfaces as a
  wrong snapshot line, a `Ty::Var` where a base type was expected (or vice-versa), or a
  failed cross-check.
- **Behavior-preservation** — the existing suite (76 lib + `typed_inference` + the
  effect/adt/affine integration corpora) is unchanged and green; `infer_with_types` and
  all existing snapshots are byte-for-byte identical; the layering test passes
  unmodified.
- **Determinism** — the snapshot is stable across runs (rendered through a shared
  `Names`, ordered by a deterministic Core pre-order).

## 9. Build order (tasks)

- **Task 1 — the raw table accessor.** Add `infer_typed_table` (§7); a unit test
  confirms it returns the same zonked types `infer_with_types` renders (parity with the
  string entry) and that the three existing entries and their snapshots are unchanged.
- **Task 2 — the Core datatype + lowering.** Add `src/core.rs` with the datatype (§2)
  and `lower_module`/`lower`/`lower_pat` (§4). A focused unit test lowers `fn id(x){x}`
  and asserts the body node's `ty` is `Ty::Var(_)` (the headline tooth) — the smallest
  program that exercises the stay-polymorphic seam. Confirm the layering test still
  passes (`core` → `ast`/`types`/`span` only).
- **Task 3 — the typed-Core snapshot corpus.** Add `tests/core_lowering.rs`: the reused
  5a-1 programs, the typed S-expression renderer, all per-surface teeth, the lookup
  and derivation cross-checks (§5), and the coherence/no-token-leak invariants. Every surface pinned by
  snapshot; surface 4's row and surface 2's variable are hard requirements.

## 10. Risks & mitigations

- **Assume-ground-types bug** (lowering monomorphizes/poisons a `Ty::Var`). *Mitigation:*
  the headline tooth (§5) on `fn id(x){x}` is written *first* (Task 2) and fails loudly
  on `Ty::Base`/`Ty::Error`. This is the one seam with real risk.
- **Row fails to ride inline** (surface 4 loses `State(Int)`). *Mitigation:* the table is
  already zonked with resolved rows (5a-1's surface 4 proved `resolve_row`'s arg path);
  lowering only clones the `Ty::Fn`, so the row cannot be dropped without dropping the
  whole type. The surface-4 tooth pins it regardless.
- **Snapshot instability** from raw var indices. *Mitigation:* render (never dump)
  through one shared `Names` (§3); order the Core walk deterministically.
- **Synthesized-node type confusion** (the `Let` spine). *Mitigation:* the lookup
  cross-check is scoped to direct-origin nodes; `Let` carries `body.ty` by propagation
  and is covered by the whole-tree snapshot (§5).
- **Silent scope creep into a consumer.** *Mitigation:* §6 forbids wiring `front_end`,
  and the deferred AST surface is a typed `LowerError`, not a stub that quietly succeeds.

## 11. Deferred / honestly-flagged

- **Exhaustiveness re-platformed onto Core** — demoted from a 5a-1-era 5a-2 item to its
  own later slice (§0). It is types-blind, so it validates patterns, not this lowering;
  it belongs where Core gains its first real *consumer*, not as a proof of the datatype.
- **CEK-on-Core (executing Core)** — the rung where the tree-walker ⇄ CEK oracle chain is
  genuinely re-proven over Core; its own slice.
- **Full-surface lowering** — `If`/`Handle`/`Resume`/`Qualified`/`Unary`/`Float`, a
  constructor-construction node, and effect-control nodes (`Perform`/`Handle`/`Resume`),
  added as programs demand them; until then they are a typed `LowerError` (§4).
- **Wiring `lower` into `front_end`** — waits until Core covers the full surface *and* a
  consumer runs on it; forcing it now would blow the minimal-Core budget (§6).
- **Var canonicalization; a standalone Core type environment/type-checker; native
  codegen; affine-on-Core** — the later rungs, each its own slice (§3, §2).

## 12. Milestone Checklist (Slice 5a-2)

- [ ] `infer_typed_table` returns the raw zonked `BTreeMap<Span, Ty>`; `infer_with_types`
  and the two `want_types = false` callers and all existing snapshots are unchanged.
- [ ] `src/core.rs` defines the minimal Core datatype — literals, `Var`, `App`, `Prim`,
  `Lambda`, `Let`, `Match`, and Core patterns — with all child pointers `Rc`-shared and
  the type carried inline (Shape C); it references only `ast`/`types`/`span`.
- [ ] `lower` is a structural fold consuming the frozen table, placing `table[span]`
  inline and never calling any solver primitive; the deferred AST surface is a typed
  `LowerError`, not a panic.
- [ ] The reused 5a-1 corpus lowers to Core and is pinned by a typed-Core snapshot
  rendered through one shared `Names`; surface 4's row shows `State(Int)` inline and no
  rendered type leaks an internal token.
- [ ] The headline case is asserted: `fn id(x){x}` lowers to a Core body node whose `ty`
  is `Ty::Var(_)` — carried, not monomorphized, not errored.
- [ ] Both origin proofs pass: every direct-origin Core node's inline `ty` equals
  `node_types[node.span]` (the lookup cross-check), and every synthesized `Let` node's
  `ty` equals its body's (the derivation check) — synthesized nodes are verified, not
  merely exempted.
- [ ] `eval`/CEK, `unify`/`unify_row`, `resolve`, `front_end`, `exhaust`, and `affine`
  are untouched; the layering test passes unmodified.
- [ ] Exit criteria claim only that Core **exists, carries per-node types, and lowering
  preserves them over the corpus subset** — and **not** that Core executes, that lowering
  is total, or that any consumer runs on Core.
