# Slice 5c-1 — handler clause resolution (unqualified clauses, unique op names)

**Status:** approved direction (2026-10-04). Front end, evaluator and Core lowering; native
codegen inherits the result unchanged.

## 0. The cut

PARKED's "unqualified handler clauses" entry: `ask() -> resume(2)` (no effect name) passes
`elya check`, fails `elya run` with E0300 "unhandled effect reached the machine", and is
refused by `elya build`. Three tools, three answers to one program.

Measured at `23c719f` (2026-10-04), every program a module of one handler over `one()`:

| # | Program | check | run | build |
|---|---|---|---|---|
| m1 | `ask() -> resume(2)` (unqualified) | ok | **E0300** | refused "unqualified handler clause" |
| m2 | `effect A { fn ping }` and `effect B { fn ping }`, clause `A.ping` | **E0420 "B is never handled"** | same | — |
| m3 | clause `Other.ask()` where `ask` belongs to `Ask` | E0420 "Ask is never handled" | same | — |
| m4 | clause `Nope.ask()`, no effect `Nope` | E0420 "Ask is never handled" | same | — |
| m5 | perform written `A.ping()` | E0100 parse error | — | — |
| m6 | extra clause `Ask.nope()`, no op `nope` anywhere | **ok** (silently) | ok | — |
| m7 | one effect declaring `ask` twice (`-> Int`, `-> Bool`) | E0400 type mismatch | — | — |
| m8 | two clauses `Ask.ask()` in one handler | ok (silently) | ok | — |

The finding that settles the design is m2 + m5. **A perform cannot be qualified**, so a
perform names a bare op, and every op index in the toolchain — the checker's `Infer.ops`,
the evaluator's `op_table`, lowering's `ast::op_effects` — is keyed by op name with the
last declaration silently winning. A module where two effects share an op name is already
broken at every perform, not only in handlers. PARKED's recorded direction ("an op name
shared by two handled effects gets rejected as ambiguous by the checker") would have left
those performs silently resolving to the last-declared effect.

**Decision (2026-10-04, maintainer): op names are unique per module.** With that rule an
op name determines its effect, so an unqualified clause has exactly one meaning.

The language design spec's grammar (§3, `handler_arm = qualified "(" …`) only shows the
qualified form; the bare form is a parser extension the maintainer chose to keep (PARKED,
2026-10-02). This slice makes it a supported form and records it here.

## 1. Rules

1. **E0202 — an operation name declared more than once.** Across two effects (m2) or
   within one (m7). Reported at the second declaration, naming the first effect. Checked
   in `resolve`, where op names are already collected.
2. **E0203 — a clause that names no declared operation.** Three shapes, one code,
   distinct messages:
   - (a) the op exists nowhere (m6): "no effect declares an operation `nope`";
   - (b) the qualifier is not a declared effect (m4): "`Nope` is not a declared effect";
   - (c) the qualifier is an effect, but not the op's (m3): "`ask` is an operation of
     `Ask`, not `Other`", help: write `Ask.ask(…)` or `ask(…)`.
   Checked in `resolve`'s `Handle` arm. This is slice 3's stated rule ("Handler Op clauses
   must name operations of a single declared effect", slice-3 spec §7.2), never enforced
   until now.
3. **An unqualified clause means the op's effect.** Under rule 1 that is well defined.
   - Evaluator: a clause matches a perform of `(effect, op)` iff `clause.op == op` and its
     qualifier, if present, is `effect`. No table is threaded: under rules 1-2 the op
     name alone decides. (PARKED proposed that inference record the resolved effect and
     both consumers read it; with unique names, the declaration index gives the same
     answer with no new state, so that machinery is not built.)
   - Core lowering: the clause's effect is `c.effect` or `op_effects[c.op]`; the
     "unqualified handler clause" refusal is lifted. Native codegen consumes Core clauses,
     which always carry an effect, and needs no change.
4. Resolve errors stop the front end before inference (`lib.rs` `front_end`), so E0202
   and E0203 arrive alone, without the misleading E0420s in the table above.

5. **Amendments during the build (2026-10-04)**, each red first:
   - *Native first-wins.* m8 was found to DIVERGE: the evaluator takes the first of two
     clauses for one op (`find`), native took the last (each clause overwrote its op's
     clause-table slot) and printed 5 for 2. Native now keeps the first. This is the back
     end matching the reference, not a language rule; §1.3's "native needs no change"
     held for unqualified clauses only.
   - *E0203 also covers clause arity* (independent review). A clause binding a different
     number of parameters than its op declares checked clean, then failed at run time
     ("unbound variable") or segfaulted natively -- pre-existing, but the bare form now
     reaches it too. Message: "`ask` takes 0 arguments, but this clause binds 1".
   - *No E0203 after E0202* for the same op name (review): its owner is ambiguous, and the
     cascade said, falsely, that `B.ping` was not B's.

## 2. Non-goals

- m8 (two clauses for one op): whether it should be an ERROR stays parked. Native now
  matches the evaluator's first-wins (§1.5).
- A qualified perform syntax (`A.ping()`), and op names shared between effects: excluded
  by the decision.
- The fn-vs-op name collision (PARKED, its own entry): unchanged.
- `ast` pretty-printer prints an unqualified clause as `.ask` — cosmetic, unchanged.

## 3. Acceptance

| # | Criterion | Kind |
|---|---|---|
| B1 | m2 and m7 are each rejected with exactly `[E0202]`. | execution |
| B2 | m3, m4, m6 are each rejected with exactly `[E0203]`, each with its own message. | execution |
| B3 | m1 runs in the evaluator and returns 2; a handler mixing one qualified and one unqualified clause runs; an unqualified handler spanning two effects is still E0423. | execution |
| B4 | m1 lowers to Core and runs natively, matching the evaluator (handler-corpus differential). | differential execution |
| B5 | Nothing already accepted is newly rejected: the full gate's existing tests, examples and corpora pass unchanged. | execution |
| B6 | Negative controls, one per rule, each failing differently, reverted. | procedure |
| B7 | Windows gate (CI) green. | execution |
