# Row variables you can write, in signatures and declarations

**Status:** part 1 in progress (2026-10-30, branch `auto/elya`, an interactive session). HANDOFF step 1, ahead of `elya/async` (step 2), which
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

Part 2 (the next slice): row parameters on declarations.
- D3. A declaration's parameter written as a row's tail is a row parameter:
  `type Step(r) { Spawned(fn() / {r} -> Int, ..) }`,
  `effect Async(e) { fn spawn(f: fn() / {Async | e} -> Int) -> Int }`; in its own
  declaration an effect's name takes its own parameters (`Async` there is
  `Async(e)`).
- D4. Encoding: a row argument is a type argument holding a phantom function type,
  `fn() / R -> Unit`, so unification, generalization and instantiation need no new
  case; a constructor's scheme quantifies the parameter's row variable.
- D5. A row argument at a use is written as a row, `Step({Log})`; usually inferred.
- D6. Natively, instance keys include a row argument's row where a field's
  convention depends on it (a task field is CPS when its row names a user effect).
  Measured first.

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
