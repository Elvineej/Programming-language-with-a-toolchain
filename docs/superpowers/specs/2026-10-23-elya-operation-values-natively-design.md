# Operation values as values, natively

**Status:** in progress (2026-10-23, branch `auto/elya`, a scheduled run). HANDOFF step 1.
The design choices are Claude's under the maintainer's delegation (rule 2), recorded
here. Builds on `2026-10-17-elya-native-any-handlers-design.md` (D1-D7, R1-R3).

## 0. Measured first (at 95e1e08, gate 1106 / 106)

What a recorder library writes once its clause grows past a line: helpers that take
the operation. Six probe programs, each through `elya run --value` and `elya build`:

| program | what it does with `o` | evaluator | native |
|---|---|---|---|
| p1 | `fn describe(o) { op.name(o) <> " asked " <> op.show(o) }`, called in the clause | `W.roll asked W.roll(6)`, `W.now asked W.now()`, 13 | refused (D2) |
| p2 | `fn forward(o) { op.perform(o) }` and `fn logged(o, v) { op.show(o) <> " = " <> op.encode(o, v) }` | `W.word("a") = "ab"`, `W.roll(6) = 7`, 7 | refused |
| p3 | `o` stored in a list twice, captured by a lambda, taken back out by a generic `first` | `W.yes()`, `W.yes`, `W.roll(6)`, `W.roll`, 6 | refused |
| p4 | `fn pick(o, p)` returns one of two operations; `fn same(o, p) { o == p }` | `same`, 6 | refused |
| p5 | `fn keep(o) { fn() { o } }`: a closure returning `o` | 6 | refused |
| p6 | the clause answers `o` itself | E0435 (the answer type escapes) | (front end) |
| p7 | p4's `same` over an operation whose argument holds a function | `before`, then "`==` reached a value that is not data" | refused |

Every refusal is "operation value used other than by op.* or ==". p6 shows the
bound that makes a native answer possible: E0435 keeps every type mentioning the
clause's rigid answer type `t`, and so `Op(t)`, inside the clause.

## 1. Decisions

- **D1. An operation is a value of its own type in each clause copy.** The expansion
  (earlier D1) makes one clause per operation. In the copy for `E.op` at a label
  instance (`St(Int)`), `$Op(t, H)` is replaced by a synthesized ground type with one
  constructor whose fields are the operation's parameters at that instance
  (`$Op.St.get.N`, `N` numbering the instances), and the copy opens with
  `let o = <that constructor>(arg1, ..., argn)`. Then `t` is replaced by the answer
  type, as before.
  - So `o` is ordinary data natively: it can be passed, stored, captured, returned
    and aliased. Its representation is an operation's arguments, which `op.show`
    and `op.perform` need anyway.
  - *Rejected:* a run-time operation value (an operation id and boxed arguments)
    with functions generic over it. It is the same boxing and dynamic-perform problem
    the earlier D1 rejected for clauses.
- **D2. Functions over an operation are cloned per operation.** The earlier R1 clones
  a function `mono::run` made at a rigid type once per answer TYPE. Two operations
  with one answer type (`W.roll`, `W.now`: both `Int`) would then share a clone,
  and the clone could not know which operation it was given. The clone key becomes
  the operation instance: `(function, rigid, operation type)`.
  - *Finite:* a clone is requested from a clause copy or from another clone of the
    same copy; E0435 keeps the rigid inside its clause, so the set is bounded by the
    functions reachable from it times the handled operations (and by the existing
    clone budget, refused by name past it).
- **D3. `op.*` and `==` are rewritten by type, everywhere.** Any `op.*` whose operand
  has an operation type, in a clause copy or a clone, becomes a `match` on the value
  that binds its fields, then the earlier per-operation expansion over those fields.
  `p == q` and `p != q` over an operation type become the earlier `o == o`: `True`
  (or `False`) unless an argument holds a function, which fails at run time by the
  evaluator's words (`any.check`, the earlier R2), after both operands are evaluated.
  - *Why `p == q` is `o == o`:* a value of a clause copy's operation type is made
    only by that copy's `let o` (no source program can name the synthesized
    constructor), and no value of it can reach another run of the clause: the
    clause's free variables are bound outside it, `resume` takes the answer type,
    and E0435 forbids the rigid in the handle's result. So within one run every
    value of the type is the same operation with the same arguments, and the
    evaluator's `==` (equal effect, name and arguments) reduces to its walk of
    `o`'s own arguments.
  - The refusal "operation value used other than by op.* or ==" is removed: no use
    is left that the back end refuses.
  - **Changed expectations by this decision:** `any.rs`'s unit test
    `an_operation_value_used_as_a_value_is_refused` (its two programs now expand,
    so it becomes `an_operation_value_used_as_a_value_expands`), and the first half
    of `native_any.rs::what_is_not_compiled_is_refused_by_name` (the `describe(o)`
    program now runs, a row of the new test file; the non-regular half stays).
- **D4. Decoding at an effect's own parameter stays refused on both sides.** The
  HANDOFF step also asked whether both sides should decode at the instantiation.
  Natively the instance is known after specialization; the evaluator does not carry
  it. Lifting S2 is a change to the evaluator's semantics and to its operation value,
  independent of this slice, so it is not taken here; it stays recorded as open
  (HANDOFF). The non-regular decode gap (earlier D5) is unchanged too.

## 2. Evidence (planned)

- `any.rs` unit test: the two formerly refused clauses expand, with one
  operation type per handled operation.
- `crates/codegen/tests/native_op_values.rs` (execution, differential against the
  evaluator): p1-p5 and p7 as rows, plus two operations with one answer type through
  one helper (D2's case), a helper that performs (`forward`) under a state-passing
  replayer, an operation stored in a declared generic type and matched out, and
  nested `with any` handlers whose helpers take both operations.
- Negative controls (recorded in the test file's header): key clones by answer type
  again (D2), bind `o` with its fields in a different order (D1), and rewrite
  `op.*` only in clause copies, not in clones (D3), each observed failing a
  distinct test.

## 3. Results

(Filled in when the slice closes.)
