# Operation-generic handlers: a recorder for any effect (`with any`)

**Status:** in progress (2026-10-16). HANDOFF step 1 (ROADMAP priority 3, the replay
step before the CLI flags). The design choices are Claude's under the maintainer's
delegation (rule 2), recorded here.

## 0. Measured first (at 82e2245)

- **The recorder in `examples/05_replay.elya` is written for one effect.**
  - `record` (17 lines) and `replay` (25 lines) name `World` and each of its two
    operations, and the log type has one constructor per operation.
  - A second effect needs a second copy of both, and so does every new operation.
  - That is the definition of not generic: a handler covers exactly one effect, every
    operation of it, by name.
- **Nothing can be written today that intercepts "every operation of whatever the body
  performs".** The natural program,
  `handle { body() } with any { op(o) -> { let v = op.perform(o)  resume(v) } }`,
  is a parse error (E0100 "expected `{`" at `any`).
- **Row variables already describe "whatever the body performs".** An unannotated
  `fn record(body) { handle { body() } with { ... } }` infers `body : fn() / ρ -> a`.
  What is missing is a handler that handles `ρ` itself, and a typed way to talk about
  an operation whose name, arguments and answer type are unknown to the handler.

### Survey

- **Koka, Effekt, Frank, Eff, Links, Unison:** every handler names its effect(s) and
  each operation. Koka can `mask` an effect but not intercept an unknown one.
- **OCaml 5:** an `effc : 'b. 'b Effect.t -> (('b, 'a) continuation -> 'a) option`
  handler IS generic over every effect. But effects are untyped (an extensible
  variant), so logging an answer generically needs `Obj` or a printer per effect.
- **Haskell (polysemy, fused-effects):** `interpose`/`intercept` re-handle one known
  effect; generic tracing needs a `Show` instance per effect.

None types a handler over "every operation of an unknown set of effects" together with
a typed answer that can be logged and fed back. That is what this slice adds.

## 1. Decisions

- **D1. Syntax: `with any { op(o) -> body  return(x) -> body }`.**
  - `any` after `with` marks an operation-generic handler.
  - It has exactly one operation clause, written `op(o)`: "an operation, called `o`".
    The `return` clause is optional, as everywhere.
  - Any other clause in an `any` handler is a parse error (E0100), as is `with multi any`.
    These handlers are one-shot (D8).
- **D2. It handles everything the body performs, except `IO`.**
  - The handled set is the body's effect row. No new kind of variable is needed: the
    row variable that already says "whatever the body performs" is what gets handled.
  - `IO` is never dispatched to a handler (it is builtins), so a concrete `IO` label in
    the body stays in the residual. Every other label, and the row's tail, is
    discharged.
  - *Rejected:* an effect-label variable (`with e { ... }`). It needs a new kind of
    variable, and an "everything but these labels" set at run time. The body's row
    already is the set.
- **D3. The operation is a typed value `o : Op(t)`, where `t` is the operation's answer
  type, rigid and fresh per clause.**
  - `resume` takes a `t`. `op.perform(o)` answers a `t`. Nothing else is known about
    `t`, so an answer can only be given back, encoded, or decoded.
  - **E0435:** `t` must not escape its clause, into the handle's result type or into
    any binding from outside the clause. A clause runs once per operation, at a
    different answer type each time. If one run's `t` could reach another run, two
    different types would meet under one name. The state-passing shape
    `fn(prev: t) { resume(prev) }` would make that unsound.
  - *Rejected:* untyped answers (OCaml's `Obj`). They are unsound.
- **D4. Five builtins on an operation, namespaced like `time.now_ms`:**
  - `op.name(o) -> String`: `"World.roll"`.
  - `op.show(o) -> String`: `"World.roll(6)"`, its arguments in literal syntax (D7).
  - `op.perform(o) -> t`: performs the same operation, with the same arguments, at
    the clause. Since a clause runs outside its handler, that reaches the next handler
    out. The clause's row gains the handled row.
  - `op.encode(o, x: t) -> String`: an answer in literal syntax.
  - `op.decode(o, s: String) -> t`: parses a literal and checks it against the
    operation's declared answer type. It fails by name if `s` is not one: "op.decode:
    `...` is not a value of type `T`".
  - *Rejected:* `decode` answering an option. Elya has no built-in option type. A
    replay compares `op.show` first, and once the question matches, only a corrupt log
    can fail to decode.
- **D5. E0434: the handled effects must be data.**
  - Every label an `any` handler handles must be a user effect, not `IO`.
  - Every operation's parameter and answer types must be data: `Int`, `Float`,
    `Bool`, `String`, `Unit`, or a declared type whose fields are data, at its
    arguments. No functions.
  - **Where it is checked:**
    - concrete labels at the handle;
    - a row tail at every instantiation of a function whose scheme generalized it.
      The tail is a "generically handled" row variable, and a copy made by
      `instantiate` inherits the mark, so a caller that passes its own row variable
      becomes generic in turn.
    - Checks run when a binding group is generalized (callees are generalized before
      their callers).
  - A type variable left in an effect's argument, or in a handled operation's type, at
    check time is E0434 as well ("not known to be data"). That is the stricter option:
    a generalized `s` in `St(s)` could later be a function.
  - *Rejected:* a dictionary (type-class) for data. The structural check plus
    self-describing literals is enough for the evaluator, and native support will
    need per-operation type tables either way.
- **D6. The evaluator first; natively refused by name.** Core lowering refuses an `any`
  handler with "operation-generic handler (`with any`)". Native support (the runtime
  needs each operation's name, arity and answer type for `show`/`decode`) is a later
  step.
- **D7. Literal syntax is the encoding:** `42`, `-7`, `1.5`, `"a\"b"`, `True`, `False`,
  `()`, `Cons(1, Nil)`. Constructor names are unique per module, so a literal describes
  itself. `decode` checks it against the declared answer type, through constructors
  and their fields. A position that is an effect's type parameter accepts any data
  literal (the parameter is not known at run time).
- **D8. One-shot.** A `with any` handler resumes at most once, like a plain `with` (a
  `multi` effect handled by it is one-shot too).

## 2. Evidence (planned)

`tests/any_handler.rs`:
- parsing (accepted shapes, refused shapes);
- types: `Op(t)`, `resume`, the builtins, the residual (`IO` stays, the rest goes);
- E0435 escapes, E0434 on non-data and `IO` (concrete and through a generalized tail);
- the evaluator: forwarding, `show`/`encode`/`decode` round trips, and a named decode
  failure.

Native: a refusal by name. The example: `examples/09_record_any.elya`, one generic
`record` and `replay` used over two unrelated effects.
