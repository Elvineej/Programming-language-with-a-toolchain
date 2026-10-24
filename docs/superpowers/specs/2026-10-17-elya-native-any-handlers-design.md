# `with any` natively: operation-generic handlers by expansion

**Status:** in progress (2026-10-17, branch `auto/elya`). HANDOFF step 1. The design
choices are Claude's under the maintainer's delegation (rule 2), recorded here.
Builds on `2026-10-16-elya-any-handlers-design.md` (the evaluator's `with any`, D1-D8).

## 0. Measured first (at 1cf9bb5)

The 2026-10-17 sweep (`python3 scripts/sweep.py`, 672 programs): 369 identical, no
disagreement, no native crash. `with any` is the top native gap: 9 programs fail to
build, all with `core lowering failed: Unsupported("operation-generic handler (`with
any`)")`.

| program | what it needs natively |
|---|---|
| `examples/09_record_any.elya` | forwarding (`op.perform`), `op.show`, `op.encode`, `op.decode`, two unrelated effects, the recorder inside a state-passing handler |
| any_handler 0395 | a `with any` handler in a local `let` lambda, used at two effects |
| any_handler 0409 | a 100 000-cell list encoded and decoded (iterative) |
| any_handler 0412 | decoding at an effect's own parameter (`St(s).get`): a run-time failure by name in the evaluator |
| any_handler 0419 | `o == o` |
| core_lowering 0434 | `op.decode` of an `Int` |
| any_handler 0410 | decoding at a non-regular type (`Nest(a) { Z(a), S(Nest(Opt(a))) }`) |
| any_handler 0391, 0399 | `Float` answers (Core refuses `Float` for every program) |

What no corpus program does: pass an operation value `o` to a function, store it in
data, or return it. Every use of `o` is the first argument of an `op.*` builtin or an
operand of `==`.

## 1. Decisions

- **D1. Compile by expansion, after row specialization.** By the time rows are
  specialized (N7 part 1), the row a `with any` handler handles is a closed set of
  labels: the clause binder's type is `$Op(t, fn() / H -> t)` and `H` has been
  substituted with the labels its row variable was instantiated at. A new Core-to-Core
  pass (`crates/codegen/src/any.rs`, between `specialize::run` and
  `mono::instantiate_types`) turns the handle into a PLAIN handle with one clause per
  operation of every label of `H` except `IO` (D2 of the earlier spec). Each clause is
  a copy of the generic clause body with the rigid answer type `t` replaced by that
  operation's answer type (the effect's arguments substituted). An open tail left in
  `H` stands for no user effect (specialization clones every reference that
  instantiates a row variable at one), so it contributes no clause.
  - This is monomorphization of the clause's rank-2 type: a clause "runs at a
    different answer type each time" (earlier D3), so natively it is one copy per
    operation, exactly as a generic function is one copy per instantiation.
  - *Rejected:* a run-time operation value (a descriptor id plus boxed arguments) and
    one generic clause. Native values are not uniform (`Bool` is `i1`, `Int` an `i64`,
    the rest pointers), so a clause generic in `t` would need boxing of every answer,
    a second calling convention for resumes, and a dynamic perform by operation id.
  - *Rejected:* a dictionary of closures per operation (`show`, `perform`, ...): the
    same boxing problem, and an effectful closure per perform.
- **D2. No operation value at run time; every other use is refused by name.** In
  a clause copy the operation is known statically, so `o` itself is not needed:
  - `op.name(o)` is the literal `"E.op"`;
  - `op.show(o)` is `"E.op(" <> show(a1) <> ", " <> ... <> ")"` over the clause's
    parameters;
  - `op.perform(o)` is a perform of `E.op` with those parameters;
  - `op.encode(o, v)` is `show(v)` at the answer type, `op.decode(o, s)` a decode at it;
  - `o == o` (the only comparison that type-checks: two clauses' answer types are
    distinct rigid types) is `True`, unless the comparison reaches a function in an
    argument, which stops the evaluator's `==` by name; each argument is checked at
    run time for one (`any.check`, the review's R2). `!=` is its negation.
  - Any other occurrence of a binder (passed to a function, stored, returned, bound
    by `let`) is refused at build time: `operation value used other than by op.* or
    ==`. None of the measured programs does it. Supporting it needs functions over
    `Op(t)` cloned per operation, a later step (recorded in HANDOFF).
  - *Rejected:* a synthesized one-constructor type per operation carrying its
    arguments. It makes `o` a value, but a function taking `o` is still generic in the
    operation, so it would need the same per-operation cloning, and nothing measured
    needs it yet.
- **D3. Literals natively through a reflection table.** The literal syntax (earlier
  D7) is produced and read by the runtime (`runtime.c`), driven by a table the back end
  emits for exactly the types `show` and `decode` are used at: per type, its kind
  (`Int`, `Bool`, `Unit`, `String`, a function, a declared type) and, for a declared
  type, each constructor's tag, source name and field types.
  - `elya_any_show` writes into a growing buffer, walking the value and the type
    together with an explicit stack (a million-cell list does not recurse);
    reaching a function value fails by name, as the evaluator does when it reaches one.
  - `elya_any_decode` parses the whole text first (the evaluator's parser, the same
    error words), then checks for trailing input, then checks the parsed literal
    against the type, then builds the value bottom-up with every built heap value
    rooted until its parent holds it. Three phases in the evaluator's order, so the
    first failure named is the same.
  - *Rejected:* show and decode generated as Elya Core per type. Right-nested
    concatenation is quadratic in the length of a list, and a parser in Core needs
    string indexing builtins that do not exist natively.
- **D4. Failures have the evaluator's words.** A native failure prints `elya: ` and
  then the evaluator's message text, and exits 1:
  - `op.show: an argument of `E.op` is a function, which is not data`;
  - `op.encode: the answer of `E.op` is a function, which is not data`;
  - `op.decode: <why>, for an answer of `E.op``, with `<why>` exactly the evaluator's
    (`` `x` is not a literal (...) ``, `` `x` has input after its value ``,
    `` `x` is not a value of type `T` `` with `T` the declared annotation).
  - **Decoding at an effect's own parameter stays a failure natively** (the
    evaluator's S2 refusal: "the answer type `s` mentions the effect's parameter
    `s`, which is not known at run time"), raised at the same point, after the text
    is evaluated. Natively the instantiation IS known after specialization, so the
    native side could decode it; doing so would make native binaries more defined
    than `elya run` (the semantic-fidelity rule in `crates/codegen/src/lib.rs`).
    Lifting it is a joint change of both sides, recorded as open.
- **D5. A decode target's types are instantiated in full.** Instances of parametric
  types are built lazily, on a constructor or a pattern; a decode can build values of
  any type reachable from its target, so `instantiate_types` demands the whole
  reachable closure of every decode target. A non-regular type (0410) has no finite
  closure and stops at the instance budget, refused by name (`too many type
  instances`). The evaluator decodes it; that stays a native gap, recorded.
- **D6. Effect declarations reach Core.** `CoreModule::effects`: per effect, its
  parameters, and per operation its parameter and answer types (parameters as
  `Ty::Var(i)`), whether it is `multi`-declared, the declared answer annotation as
  the evaluator renders it, and the effect parameter it mentions, if any. Core
  lowering no longer refuses `with any`; `CoreHandle::any` marks it.
  - **Changed expectation by this decision:**
    `tests/core_lowering.rs::an_operation_generic_handler_is_refused_at_lowering`
    becomes `an_operation_generic_handler_lowers_marked_any` (the refusal moved to
    the back end, where only D2's and D5's remain).
- **D7. One-shot, as before.** The expanded handle is a plain `with` (never `multi`);
  `is_multi_declared` is stamped from the handled operations' declarations as Core
  lowering stamps it for a written handle.

## 2. Evidence (planned)

- `crates/codegen/src/any.rs` unit tests (LLVM-free): one clause per handled operation,
  `IO` excluded, the refusal of D2.
- `crates/codegen/tests/native_any.rs` (execution, differential against `elya run`'s
  evaluator): the example; forwarding at `Int`, `Bool`, `String`, `Unit` and declared
  answers; `op.name`/`op.show` with zero to three arguments and escapes; encode/decode
  round trips through ground and parametric declared types; a 100 000-cell list;
  every decode failure class with the evaluator's words; `show` of a function by
  name; operation equality; nested `with any` handlers; a local `let` lambda handler;
  the refusals by name (D2, D5, `Float`).
- Negative controls (recorded in the test file's header): break the expansion's type
  substitution, the clause-per-operation enumeration, decode's phase order, and the
  build phase's rooting, each observed failing a distinct test.

## 3. Results

Gate after the slice: 1069 passed, 102 suites (predicted 1067 before the review;
the review added two native tests).

Negative controls K1-K5 are recorded in `crates/codegen/tests/native_any.rs`'s header;
each failed a distinct test, and the files were restored byte for byte.

### The review (2026-10-17)

An independent review ran about 110 differential programs and the full sweep (no
`differ`). Findings, fixed test-first (`native_any.rs`, "the review"; both red first):

- **R1, high.** A generic function called at the clause's answer type
  (`resume(id(op.perform(o)))`) was cloned once by `mono::run`, at the rigid type,
  before the expansion: the build panicked in inkwell, or (a pointer-sized answer) a
  `Box('t)` instance's tags made a match fail. **Fix:** the expansion clones such a
  function per operation, at its answer type, as it does the clause (a function whose
  signature mentions a rigid type is redirected in each copy, transitively, and the
  rigid original dropped). Budgeted by the clone budget, refused by name past it.
- **R2, high, and R3, medium.** `o == o` was compiled as the arguments' own
  equalities: an argument holding a function inside data compared equal natively
  where the evaluator fails (`` `==` reached a value that is not data ``), and a
  function-typed argument refused the whole program at build time
  (`Eq on an unrepresentable operand type`) even when that operation never ran.
  **Fix:** D2 revised: each argument is walked at run time for a function
  (`elya_any_check`), with the evaluator's words.
- Not of this slice, recorded: the one-shot trap's wording (HANDOFF step 5); `type
  variable only in an effect row` refusals of programs with no `with any` at all.
