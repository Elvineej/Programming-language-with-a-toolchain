# Row variables you can write, in signatures and declarations

**Status:** part 1 done (394a0b8, branch `auto/elya`); part 2 in progress (2026-10-30, on `main`, an interactive session). HANDOFF step 1, ahead of `elya/async` (step 2), which
needs it. The design choices are Claude's under the maintainer's delegation (rule 2).

## 0. Measured first

What a task-holding declaration can say today (debug CLI, `elya run --value`):

| program | result |
|---|---|
| `fn twice(f: fn() -> Int) -> Int { f() + f() }` with a task that performs `Log` | 2: a signature's unwritten row is open |
| `effect Async { fn spawn(f: fn() -> Int) -> Int }`, a task that performs `Log` | E0423: an operation's unwritten row is pure |
| `type Task { Task(fn() -> Int) }`, a task that performs `Log` | E0423: a field's unwritten row is pure |
| `type Task { Task(fn() / {Log} -> Int) }` | 2: only a row the declaration names |
| `type Task(t) { Task(t) }`, `t` a task that performs `Log` | 2: a TYPE parameter can carry a task |
| `effect Async(t) { fn spawn(f: t) -> Int }`, a task that performs `Log` | 5 |
| the same, a task that spawns | E0401 (since the occurs-in-rows fix; a crash before): `t = fn() / {Async(t)} -> Int` |
| `fn run(f: fn() / {e} -> Int)` | E0100 "expected effect name" |
| `fn run(f: fn() / {Log | e} -> Int)` | E0001 "unexpected character" |
| `/ {State(Int)}` in a signature | accepted (a label's arguments in a signature's row) |

- The checker's printer already writes rows as `{a}` and `{Log | a}`
  (`fn h(f) { say("x")  f() }` prints `f : fn() / {Log | a} -> b`); the lexer has no
  `|` token (hence E0001), and `parse::effect_row` reads capitalised names only.
- `types::elaborate_adt_ty`: "a declaration has no row variables, so its rows are
  exactly what is written". Operation signatures use the same elaborator.
- A constructor's scheme already has `row_vars` (empty for every constructor today);
  an operation's signature quantifies only its effect's TYPE parameters.
- Core's generic types number a parameter as `Ty::Var(i)` in field types; the native
  instance builder substitutes those. Instance keys leave rows out (mono's `canon`
  with `rows == None`).

So the async scheduler needs a type whose argument is a ROW: the task's row is
`{Async(e) | e}` with `e` the other effects, a finite type (a type parameter holding
the whole task type makes `Async` mention itself).

## 1. Decisions

Part 1 (this slice): row variables in a function's own annotations.
- **D1. A row may end in a row variable, written as the printer writes it:**
  `/ {e}` (any effects, named) and `/ {Log, State | e}` (those and whatever `e` is).
  The variable is a lowercase name after `|`, or alone; at most one per row, and
  last. A grant list (`import m / {..}`) names effects only.
- **D2. A named row variable is one row for its whole function** (its signature
  and the annotations in its body), like a lowercase type name: flexible ("the
  same row here and there"), not rigid. So
  `fn run(f: fn() / {e} -> Int) / {e} -> Int { f() }` says `run`'s row is `f`'s,
  and `fn logged(f: fn() / {e} -> Int) / {Log | e} -> Int` adds `Log`. Being
  flexible, what a body performs beyond the declared labels joins the row (and so
  `f`'s, which its callers then handle); the declared labels are still checked. An
  unwritten row keeps its meaning (any effects, unnamed). A lowercase name spelled
  like an effect (`log` for `Log`) is a typo, and one name is a type or a row, not
  both.
- Representation: an annotation's row stays a list of names; a lowercase one, last,
  is the tail (`ast::row_tail` splits them), so every consumer of effect names sees
  effects only.

Part 2: row parameters on declarations (settled in section 3 below).

## 2. Evidence (part 1)

- `tests/row_variables.rs`: D1's syntax and its errors; D2 shared across a
  signature, adding a label, and a call that the shared variable makes wrong.

Measured (part 1, 2026-10-30):
- Red first as planned: E0100 and E0001.
- Two drafts of the third test were wrong, each found with the CLI before the code:
  a call of a parameter relays its row into the caller's, and E0421 counts a
  function's own labels only (`fn run(f: fn() -> Int) / {} -> Int { f() }` checks
  clean). So naming does not change those; what it changes is sharing between
  positions (`first`, E0420 where unnamed runs).
- A declaration's row variable read as "unknown effect `e`": refused by name until
  part 2 (a fifth test).
- The new forms natively: the evaluator's and the binary's values agree (148).
- No other root test changed (509 tests, 61 suites). Controls K1, K2 (the test's
  header): each failed distinctly.
- README: the annotation syntax.
- **Review (an independent agent), each finding fixed test-first:**
  - F1, high: row unification dropped a label when both rows ended in one variable
    (`{Log | e}` against `{e}`): the first absorb bound `e`, then the stale tail was
    rebound. Older than this slice (`absorb`'s missing side condition, noted in the
    occurs-in-rows review), but names made it writable: a program checked clean and
    failed at run time ("unhandled effect `say` reached the machine"; natively "the
    handler has no clause"), or E0424 with the arguments swapped. Each tail is read
    again after the other's absorb, and `bind_row` never overwrites a bound variable
    (it unifies). Rows are sets, so `e := {Log | f}` solves it. Test
    `a_shared_tail_keeps_its_labels`; control K3.
  - F2, high: a named declared row was unified after the ambient closed (2.5), so
    `e` was forced pure wherever else it is written (three correct programs
    refused). F3, high: in a recursive group the last member's names were read.
    Now each member's names are kept from its body, and its named row is unified
    at 2.48, before anything closes. Test `a_declared_named_row_is_its_own_functions`;
    control K4.
  - F5, medium: the parser's row errors cascaded or misled (`{e, Log}`, `{Log, e}`,
    `{e |}`), `{_}` was a variable, a typo `/ {log}` became one silently, and one
    name could be a type and a row. One named error per row now, and two checks.
    Tests `a_row_variable_is_written_once_and_last` (exact messages),
    `a_row_variable_is_not_a_misspelled_effect_or_a_type`.
  - F6, medium: only one test depended on sharing. Test
    `a_name_is_shared_by_the_function_row_and_the_body`; K1 now fails two.
  - F7, low: README and D2 said "exactly what `f` does"; flexible says less (above).
- Natively, after the review: the new forms agree with the evaluator, except a
  function whose named row says more than its body performs
  (`fn h(g: fn() / {e} -> Int) / {e} -> fn() / {e} -> Int { g }`): its convention is
  direct (its body performs nothing, D16) and its calls' CPS (its type says `e`), so
  it is refused by name, "calling convention disagrees with the callee". A
  refusal, not a wrong answer; HANDOFF step 5's (native limits).
- The root crate after the fixes: 513 tests, 61 suites, all pass.

## 3. Part 2: row parameters on declarations

### Measured (2026-10-30, debug CLI at 394a0b8)

| program | result |
|---|---|
| `type Task(t) { Task(t) }`, `run(Task(fn() { 1 }))` and a `Log` task in one program | 3 in both back ends: a type parameter at two rows is two instances already |
| `type Task { Task(fn() / {Log} -> Int) }`, a pure and a `Log` task | 3 in both back ends |
| `type T { T(fn() / {e} -> Int) }` | E0432 "a declaration's row cannot name a row variable (`e`) yet" (part 1) |
| `effect E(t) { fn op(k: fn() / {E} -> Int) -> Int }`, and with `{E(t)}` | E0432 "effect `E` takes type arguments, which a declaration row cannot write yet" |
| `fn f() / {State} -> Int { 1 }`, `effect State(s)` | E0423 (a bare parameterized label in a signature) |
| `fn f() / {State(Int)} -> Int { get() }` | 3: but the label's arguments are parsed and DROPPED (`parse::effect_row` skips them); each is a fresh variable |
| `effect Async(t) { fn spawn(f: t) -> Int }`, a task that spawns | E0401 (`t = fn() / {Async(t)} -> Int`) |

- Two programs in the corpus write a label's arguments in a signature
  (`/ {St(Int)}`, `tests/any_handler.rs`, `native_any.rs`), both correct.
- The instance key renders a function row as its convention alone (`mono::canon`,
  since the 2026-10-17 review's f3), so a phantom argument keys an instance by
  convention with no change: `Task({Log})` and `Task({})` are two instances,
  `Task({Log})` and `Task({Ask})` one. Core's generic field types are elaborated
  from the AST again (`core::ann_to_ty_in`), with rows closed: a row parameter's
  tail would be dropped there, and a task stored CPS read back direct.
- A `$Op` type already carries a row in a phantom function type (`with any`'s
  operation, `Op(t / {E})`), and the printer writes it as a row.

### Decisions (Claude's, under the maintainer's delegation)

- **D3. A declaration's parameter is a row parameter when the declaration uses it
  where a row goes:** as a row's tail (`fn() / {e} -> Int`, `{Log | e}`), or as the
  argument at another declaration's row parameter (`type Queue(e) { Empty,
  Push(Task(e), Queue(e)) }`). Kinds are found by a fixpoint over every declaration
  (a parameter only ever turns from type to row, so it ends). A row parameter used
  where a type goes is E0400 by name; a parameter used in neither way stays a type
  parameter. In its own declaration an effect's bare name is itself at its own
  parameters: `effect Async(e) { fn spawn(f: fn() / {Async | e} -> Int) -> Int }`.
  Elsewhere in a declaration a parameterized effect's arguments are written
  (`{Async(e) | e}`): a declaration's rows are exactly what is written. A row
  variable a declaration does not declare stays E0432 by name.
  Rejected: a marked parameter (`type Task({e})`): one more form for what the
  declaration's uses already say.
- **D4. Encoding: a row argument is a type argument holding a phantom function
  type, `fn() / R -> $Row`** (`$Row` is a type no program can name). Unification,
  generalization and instantiation need no new case: a constructor's scheme
  quantifies its type's row parameters' row variables; an operation instantiates its
  effect's row parameters fresh, as it does its type parameters, and a handle fixes
  them for its clauses. The printer writes the phantom as the row (`Task({Log | a})`).
  (`with any`'s data check, E0434, would read a phantom as a function; it never meets
  one alone: a row parameter is only ever some function's row, so a type with one
  holds a function and is not data either way.)
- **D5. A use writes a row argument as a row:** `Task({Log})`, `Task({})`, or a row
  variable alone, `Task(e)` (`Task({e})`). In a row, an effect's arguments are
  written in parentheses, `{Async(e) | e}`, `{State(Int)}`, and are now checked (they
  were parsed and dropped: every label's arguments were fresh). Unwritten in a
  function's annotations (`{State}`), they are inferred as before. A type at a row
  parameter, or a row at a type parameter, is E0400 by name.
- **D6. Natively: Core's generic field types write row parameter `i`'s tail as
  `Open(i)`,** as they write type parameter `i` as `Var(i)`; the instance builder
  substitutes the rows (from the arguments' phantoms) before the types. Instance keys
  are unchanged (by convention, above).
- **D7. The AST:** a row is `RowAnn { labels: [LabelAnn { name, args }], tail }`
  (part 1's list of names with the variable last goes); a row written as a type
  argument is parsed into its phantom's annotation, `fn() / {..} -> $Row`, so every
  walker of annotations (the module checks, Core's elaboration) sees a function type.
- What a clause that calls a task may perform needs no rule of its own: it performs
  the task's row, which the handle's outer ambient receives. A scheduler handles it
  by calling itself (`Async.spawn(f) -> resume(sched(f))`), the shape that was
  E0401 with a type parameter.

### Evidence (part 2)

- `tests/row_parameters.rs` (8): a type's row parameter, a row argument written
  as a row (and a `Log` task refused as a `Task({})`, E0423), kinds through
  another declaration (the round robin of `examples/04_async.elya` written once,
  `tests/row_parameters/round_robin.elya`: the example's output), an effect's row
  parameter (a task that spawns, annotated and not: 6; and one scheduler at two
  rows), D3/D5's five named errors, written label arguments checked, the printer.
- `crates/codegen/tests/native_row_parameters.rs` (3): `Task({Log})` and
  `Task({})` in one program, the spawning scheduler, the round robin; each binary
  prints what the evaluator prints.
- Part 1's `a_declarations_row_variable_waits_for_part_2` is renamed
  `a_declarations_row_variable_must_be_its_parameter`; its assertion holds.

Measured (part 2, 2026-10-30):
- Red first as planned: all 7 root tests (the table in the plan).
- Found while making them pass, each a case the design did not name:
  - A call's row was poured into the ambient label by label, then the ambient
    equated with the call's tail. For a task's row, `{Async(e) | e}` inside a
    handle for `Async(e)`, that put `Async(e)` inside `e`: E0424 "cyclic effect
    row" for both schedulers. A row whose labels' arguments mention its own tail
    is now unified whole (`Async` meets `Async`, `e` the rest); every other row
    is poured as before, so no other program changes.
  - Two performs of `spawn` each bring a phantom, concrete from the start, and
    `unify_effect_args` reported two concrete unequal arguments as E0423 without
    unifying them. Phantoms are unified (their rows' conflict is reported there).
  - Natively, the first build refused `Queue(e)` holding `Task(e)` by name
    ("direct function used where an effectful one is expected"): the
    specializer's pattern binder types (`mono::generic_fields`) substituted the
    type parameters only, so `t` read `Task({b})` (the raw `Open(0)`) where the
    instance builder had `Task({Log})`. Both now use one helper,
    `ctor_fields_at`, rows first.
- The spawning scheduler ran natively with no codegen change: an effect's row
  parameter is instantiated in the checker, and typed Core's rows are ground
  after specialization.
- Gate at 7 + 3 tests: 1372 passed, 148 suites, as predicted. Controls (the
  test files' headers): K1, K3, K4 failed distinctly; K2 (an effect's row
  parameter shared, not instantiated per use) failed nothing, so an eighth root
  test uses one scheduler at two rows (`an_effects_row_parameter_is_instantiated_per_use`).
- **Review (an independent agent), each finding fixed test-first:**
  - F1, high: `unify_row` unified the labels both rows share, which can bind
    either row's tail (a phantom binds the other row's own `e`), and then split
    the rows as they were before: a label was lost (`St(String)` beside `St(Int)`:
    a program checked clean, the evaluator failed on `==` and the native binary
    crashed), or never matched (`{Async({Log}), Log}` handed to a task parameter,
    E0423). Both rows are now read again until no label is newly shared. Test
    `a_rows_labels_are_read_again_after_its_arguments_unify`.
  - F2, medium: checking written label arguments refused programs that ran
    before (`/ {St(Box(Int))}` against an inferred `Box(a)`; any function type):
    `unify_effect_args` read two concrete, unequal arguments as a conflict. A
    conflict is now two different heads or two different ground types; the rest
    unify. The same fix took the inclusion fixpoint's relay through `add_row`
    (a state holding a function, called: `w` held `St(.. w ..)`). Test
    `a_written_label_argument_unifies_structurally`.
  - F3, medium: a scheduler that runs a task in its clause, annotated with its
    own inferred type, was E0424: the handle's residual `e` was equated with an
    ambient holding `Async(e)`. `add_row` keeps a label whose arguments mention
    the relayed tail out of it, whether the label comes with the row or is
    already in the ambient. Test `a_scheduler_that_runs_a_task_in_its_clause_checks`.
    **One existing expectation changes** (a decision, under the delegation):
    `occurs_in_rows.rs`'s `fn g(k) { op(k)  k() }` (`effect E(t)`) was E0424 and
    now checks, `g : fn(fn() / {a} -> b) / {E(fn() / {a} -> b) | a} -> b`, a
    finite type; a `k` that performs `E` itself is refused at the call.
  - Found checking F3's fix: such a kept-out label can reach the variable later
    by another path, and `resolve_row` keeps the first of two same-named labels.
    `g(self_op)` with `self_op` performing `E(Int)` checked clean. Same-named
    labels along a row's chain now have their arguments unified where rows meet
    (`settle_duplicates`): E0423. Test `a_label_met_twice_along_a_row_is_unified`;
    control K5.
  - F4, low: `fn bad(t: fn() / {Async(e) | e} -> Int) / {e} -> Int { t() }` is
    E0423 naming `Async` (it was "cyclic effect row", naming nothing); a bare
    row-parameterized effect in a declaration is told its argument is a row.
    Test `a_task_rows_mistakes_are_named`.
  - Natively: a ground type with row arguments in its fields and a label's
    argument, handed to a scheduler (F1's refusal), runs; every one of the
    review's 82 probes that checks prints the same natively.
