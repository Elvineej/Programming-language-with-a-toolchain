# Elya — Slice 5a-1 Design Specification: Typed-Inference Output (Core IR arc, foundation)

**Goal:** Make inference *persist* the type it already computes for every
expression node — a zonked `Span → Ty` side-table — and prove it directly with
snapshots, isolating the one architectural pivot of the Core IR arc (materialized
per-node types) into its own reviewable step, before any Core datatype or lowering
depends on it.

**Arc:** Core IR (Slice 5) — the compilation substrate under *both* native codegen
and precise linear ownership. This is sub-slice **5a-1** ("slice 1a" in the
brainstorm): the typed-inference *feeder*. **5a-2** ("1b") builds the Core datatype
and the type-directed lowering on top of the table this slice produces.

**Non-negotiable boundary:** 5a-1 adds an inference *output*. It does **not** add a
Core module, does **not** touch `eval`/the CEK machine, and does **not** reach into
`unify`/`unify_row`/`bind`/`instantiate`/`generalize` — it only *reads* their result
via `resolve`. See §6.

---

## 0. How to read this document

The payoff of the whole Core IR arc is that the compiler stops throwing types away
(`infer_expr` computes a `Ty` per node and discards it — [types.rs](../../../src/types.rs)).
The *risk* of the arc is concentrated entirely in this feeder: zonking, generalization,
and row materialization. 5a-1 does exactly that risky part and nothing else, with the
snapshot corpus (§5) as the direct consumer that fails loudly when materialization is
wrong. §2 is a **hard prerequisite gate** — read it first; if the span audit fails, the
whole shape of §3 changes.

## 1. Scope

**In:** a per-node `Span → Ty` table, populated during inference and zonked once at the
end; a public entry that renders it deterministically; the bug-surface corpus (§5) that
pins every way materialization can go wrong; the span-uniqueness audit (§2) that decides
the table's keying.

**Out (5a-2 and later):** the Core IR datatype, AST→Core lowering, re-platforming
exhaustiveness onto Core, re-platforming the CEK machine onto Core, native codegen,
re-platforming `affine` onto Core. None of these are touched here.

**The honesty line (must hold, mirroring 4d-1/4d-2):** 5a-1 proves the *feeder*, not the
substrate. A green 5a-1 means "inference now materializes correct per-node types," **not**
"Core IR exists." Exit criteria (§12) may not claim Core, lowering, or execution over Core.

## 2. Prerequisite gate — the span-uniqueness audit (Task 1, hard)

The table is **Shape A**: keyed on `Span{start,end}` (nodes carry only a span — no
`NodeId` — and there is a `Span::EMPTY` sentinel; see [span.rs](../../../src/span.rs)).
A span-keyed table is only sound if **no two distinct typed nodes share a span** and **no
typed node has `Span::EMPTY`** — otherwise a collision silently hands a node the wrong
type, which is exactly the class of latent bug this slice exists to prevent.

**The audit is the first task and a hard prerequisite. Do not build the table until spans
are proven unique.**

- **What it checks:** over a representative corpus (`examples/*.elya` plus the §5 corpus and
  a sampling of the existing test programs), walk every **expression** node (every
  `Spanned<Expr>` that `infer_expr` visits — the exact node class the table keys) and
  collect its span. Assert: (a) no span occurs twice; (b) no collected span equals
  `Span::EMPTY`. Suspect sources: bare-constructor-as-function-value lifting (4b-1) and any
  synthetic node the parser emits without a real source span.
- **Retained:** the audit ships as a permanent regression test, so a future synthetic node
  that breaks uniqueness fails CI rather than silently corrupting the table.

**Decision gate:**
- **Audit passes → Shape A** (span-keyed table) is the plan for §3, as the *transient
  bridge*: produced by inference, consumed once by 5a-2's lowering, then discarded — the
  types end up **inline on Core nodes (Shape C)** in 5a-2, so the AST-keyed map never
  outlives lowering.
- **Audit fails → Shape B fallback:** add `NodeId: u32` to AST nodes and key on it. This is
  a pervasive AST change (every node construction in [parse.rs](../../../src/parse.rs) and
  every walker), so it reshapes Tasks 2–3. The spec plans for Shape A as the expected path;
  a failed audit triggers a re-plan, it does not get worked around.

## 3. The typed-inference output (Shape A)

**Where it lands.** `Infer` grows one field, `node_types: HashMap<Span, Ty>`. Every
expression node's inferred type is recorded at a **single record point** — the tail of
`infer_expr`, before it returns `ty`: `node_types.insert(e.span, ty.clone())`. Binders
(params, `let` names) are *not* recorded; a param's type is observed wherever the param is
*used* (a `Var` node), which is sufficient for the proof (§5). Recording is unconditional
(a cheap insert per node); it is *exposed* only through the new entry (§7), so
`infer`/`infer_schemes`/`infer_with_sites` are byte-for-byte unchanged.

**Record-then-zonk ordering (correctness rule).** The type recorded during the walk holds
in-progress unification variables that later constraints refine. The table is therefore
zonked in **one final pass, after the whole SCC loop in `infer_all` completes**, never at
record-time: `for t in node_types.values_mut() { *t = self.resolve(t); }`. `resolve`
([types.rs:241](../../../src/types.rs#L241)) deep-follows `subst`, and `resolve_row`
([types.rs:207](../../../src/types.rs#L207)) follows `row_subst` and re-resolves each
label's argument types. §5 includes a dedicated ordering test.

**No inference logic changes.** Recording is additive instrumentation of the existing walk.
Algorithm J has no backtracking (unification is destructive union-find, no rollback), and
each AST node is visited exactly once, so a single record-per-node is well-defined.

## 4. Decision — stay polymorphic (settled here; binds every later consumer)

A node inside a polymorphic body (the `x` in `fn id(x) { x }`) zonks to a bare `Ty::Var`
— one of the enclosing function's *generalized* scheme variables (generalization happens at
step 3 of each SCC group, [types.rs:1834-1842](../../../src/types.rs#L1834-L1842)). The
table legitimately contains type *variables*, not only ground types.

**Decision:** Core IR **stays polymorphic** (System-F-lite): a materialized type may contain
free variables bound by the enclosing function's scheme, and use sites carry their
instantiations (a use of `id` at `Int` zonks to the concrete `fn(Int) -> Int` because
`instantiate` made fresh vars later unified to `Int`). We do **not** monomorphize.

**Rationale.** Elya's polymorphism is parametric-only — no traits, no dictionaries — so it
is uniform and erasable; carrying quantified vars + per-call instantiations is cheap and
honest. Monomorphization is a hammer we do not need and would only ever consider at codegen.

**Consequence (why it's decided now, not in 1a's code):** although 5a-1 merely *records and
snapshots* variables without acting on them, this decision constrains every later consumer —
5a-2's Core datatype must have a type-variable form, and the eventual codegen and
affine-over-Core passes must handle type variables rather than assume ground types. Settling
it here prevents a later consumer from silently assuming monomorphic Core.

## 5. The deliverable — the bug-surface corpus (Task 3)

The corpus **is** the deliverable, not a test-nicety: the snapshot is the *direct consumer*
that validates the feeder. Each case is an `insta` snapshot of the rendered table (§7),
plus targeted assertions that **fail loudly** on lingering variables, wrong generalization,
or unresolved rows. Every surface below is mandatory.

| # | Surface | Program (sketch) | Snapshot must show / fail-loud on |
|---|---------|------------------|-----------------------------------|
| 1 | Monomorphic fn | `fn add1(n) { n + 1 }` | every node concrete (`Int`); **fail** if any type variable appears |
| 2 | Polymorphic fn (var-typed nodes) | `fn id(x) { x }` | the body `x` node shows a type variable; **fail** if it shows a base type (wrongly monomorphized) |
| 3 | Use-site instantiation | `id(5)` and `id(True)` in `main` | the two `id` use sites show `fn(Int) -> Int` and `fn(Bool) -> Bool`; **fail** if identical or variable |
| 4 | Effectful arrow — **row materialization (hard requirement)** | a function whose arrow carries `{State(Int)}` (or `{Log}`) | the arrow's row is fully resolved *including argument types* (`{State(Int)}`, never `{State(a)}` or a raw row token); **fail** on any unresolved row var or arg — this is 4c-2 `resolve_row` territory |
| 5 | `let`-generalized lambda (value restriction) | `let f = fn(x){x}  f(1)  f(True)` | `f`'s body node is a variable; its two applications instantiate to `Int`/`Bool`; **fail** if value restriction is mis-applied (f pinned to first use) |
| 6 | `match` | a `match` over an ADT | scrutinee, arm bodies, and the `match` node each carry a type |
| 7 | Linear binding (cross-check with 4d-2) | `linear type Tok { Tok }  let t = Tok` | `table[value.span] == Tok` **and** that same span is in `affine_sites` — the table and the affine sites agree |

**Additional fail-loud invariants (across all cases):**
- **No internal-token leak.** The rendered table must never contain raw inference tokens
  (`%t`, `%r`, `%e`, …) — the same UI invariant the `//~ ERROR` fixtures enforce.
- **Cross-node variable coherence.** A variable shared by two nodes of the same function must
  render as the *same* letter — the table is rendered through a **single shared `Names`**
  (§7), and a test asserts identical rendering for a shared variable. (`display_ty`
  [types.rs:607](../../../src/types.rs#L607) seeds a *fresh* `Names` per call, so per-node
  rendering would break this — hence the shared-`Names` requirement.)
- **Record-then-zonk ordering.** `fn f(x) { let y = x  y + 1 }`: `y` is a fresh variable when
  first recorded, then forced to `Int` by `y + 1`. The final table must show every node as
  `Int` — proving the zonk ran *after* inference, not at record-time.

## 6. Non-interference guarantees (explicitly confirmed, per request)

- **Touches neither Core nor eval → oracle green by non-interference.** 5a-1 adds no Core
  module and does not touch [eval.rs](../../../src/eval.rs) or the CEK machine. `eval` still
  runs the AST; the tree-walker ⇄ CEK cross-check is untouched, so it stays green *because it
  is unchanged*, not because anything was re-proven. (Re-pointing the CEK machine at Core is
  5a-2/later, and *that* is where the oracle chain is genuinely re-proven.)
- **No `unify`/`unify_row` reach.** Recording reads the result of inference via `resolve`;
  it does not modify `unify`, `unify_row`, `bind`, `instantiate`, or `generalize`. Algorithm
  J is byte-for-byte unchanged — consistent with every prior slice.
- **No layering change.** The table lives inside `types.rs` (a new `Infer` field + one public
  entry); the corpus is a new `tests/` file. No new `src` module, so
  [tests/arch/layering.rs](../../../tests/arch/layering.rs) is untouched.
- **Existing suite and snapshots unchanged.** The three existing entries return exactly what
  they did; no existing snapshot churns.

## 7. Pipeline & module changes

- **`Infer.node_types: HashMap<Span, Ty>`** — the durable artifact (reused by 5a-2's lowering
  via a raw accessor added then; not exposed as raw here — YAGNI).
- **New public entry:** `pub fn infer_with_types(session: &Session, module: &Module) ->
  (Vec<Diagnostic>, BTreeMap<Span, String>)`. It runs `infer_all`, zonks `node_types` (§3),
  and renders each entry through **one shared `Names`** (module-wide, for cross-node
  coherence) via the existing `write_ty` path, returning a `BTreeMap<Span, String>` (sorted
  by span for deterministic snapshots). Rendered strings are the right shape for 5a-1's
  consumer (the snapshot); 5a-2 promotes the surface to also carry raw `Ty` when lowering
  needs it.
- **Snapshot format (auditable):** one line per typed node, `"<src-substring>" @ start..end :
  <type>`, ordered by `(start, end)`. Including the source slice makes the snapshot a
  human-auditable "this syntax got this type" artifact.
- **`lib.rs`:** no wiring change — `front_end` is untouched. `infer_with_types` is a
  test-facing entry, exactly as `infer_schemes` is today.

## 8. Testing strategy

- **Span-uniqueness audit** (§2) — the gate, retained as a regression test.
- **The bug-surface corpus** (§5) — seven snapshots + the three cross-cutting fail-loud
  assertions (no-token-leak, cross-node coherence, record-then-zonk ordering). The snapshots
  are the consumer; a materialization bug shows up as a wrong line, a leaked token, or a
  failed targeted assertion.
- **Behavior-preservation** — a test (or the existing suite) confirms `infer`,
  `infer_schemes`, and `infer_with_sites` are unchanged and no existing snapshot churns.

## 9. Build order (tasks)

- **Task 1 — the span-uniqueness audit (hard gate).** Write and run the audit over
  `examples/` + a small dedicated corpus + sampled test programs. **Pass → Shape A; fail →
  re-plan for Shape B (NodeId).** No table code before this passes.
- **Task 2 — the typed table (Shape A).** Add `Infer.node_types`; record at `infer_expr`'s
  return; add the final zonk pass at the end of `infer_all`; add `infer_with_types` rendering
  through a shared `Names`. Confirm the three existing entries and all existing snapshots are
  unchanged (behavior-preserving gate).
- **Task 3 — the bug-surface corpus.** Add `tests/typed_inference.rs` with the seven snapshot
  cases and the three fail-loud invariants (§5). Every surface pinned; the row-materialization
  case (surface 4) is a hard requirement.

## 10. Risks & mitigations

- **Span audit fails.** *Mitigation:* it is Task 1 and a hard gate; failure triggers the
  Shape B (NodeId) re-plan before any table code exists — no rework of a built table.
- **`resolve_row` incompleteness leaks a row var into a materialized type** (the 4c-2 class of
  bug, now load-bearing for *all* node types, not just printed ones). *Mitigation:* surface 4
  is the tripwire. If it leaks, fixing `resolve_row` is **in scope for 5a-1** — it is a
  materialization-correctness bug, exactly what this slice exists to catch — and is a
  completeness fix to `resolve_row`, not a change to Algorithm J.
- **Cross-node variable-naming divergence** (fresh `Names` per call). *Mitigation:* the
  shared-`Names` requirement (§7) + the coherence assertion (§5).
- **Snapshot instability** from `HashMap` iteration order. *Mitigation:* the public return is a
  `BTreeMap` sorted by span.

## 11. Deferred / honestly-flagged

- **5a-2 ("1b"):** the Core IR datatype, type-directed AST→Core lowering consuming this table
  with types **inline on Core (Shape C)**, a typed-Core snapshot, and exhaustiveness
  re-platformed onto Core (proof that Core carries patterns — note exhaustiveness itself needs
  *no* types, [exhaust.rs:12-14](../../../src/exhaust.rs#L12-L14), so it can never validate the
  feeder; that is precisely why 5a-1 exists as its own slice). Promotes `infer_with_types` to
  surface raw `Ty`; records binder types if lowering needs them.
- **Monomorphization** — only if codegen ever demands it (§4 keeps Core polymorphic). Not now.
- **CEK-on-Core, native codegen, affine-on-Core** — the later rungs; each its own slice.

## 12. Milestone Checklist (Slice 5a-1)

- [ ] The span-uniqueness audit passes over `examples/` + corpus (no duplicate spans, no
  `Span::EMPTY` among typed nodes); it is retained as a regression test.
- [ ] `Infer.node_types` records every expression node's type at a single `infer_expr` record
  point; the table is zonked in one final pass after `infer_all`'s SCC loop.
- [ ] `infer_with_types` returns a span-sorted `BTreeMap<Span, String>` rendered through one
  shared `Names`.
- [ ] All seven bug-surfaces are pinned by snapshot; the row-materialization case shows a fully
  resolved row **including argument types**, with no leaked internal token.
- [ ] Record-then-zonk ordering is proven (a node refined after record-time shows its final
  type); cross-node variable coherence is proven.
- [ ] `infer`/`infer_schemes`/`infer_with_sites` unchanged; no existing snapshot churns; no new
  `src` module; `eval`/CEK and `unify`/`unify_row` untouched; layering test unchanged.
- [ ] Exit criteria claim only the **feeder** — correct per-node type materialization — and
  **not** that Core IR exists, lowers, or executes.
