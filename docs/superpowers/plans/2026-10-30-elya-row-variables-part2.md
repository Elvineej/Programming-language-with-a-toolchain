# Plan: row parameters on declarations (row variables, part 2)

Spec: `docs/superpowers/specs/2026-10-30-elya-row-variables-design.md`, section 3
(D3-D7). HANDOFF step 1. On `main` (the maintainer, 2026-10-30).

## Red first (`tests/row_parameters.rs`, 7 tests; all 7 red at 2ab5833)

| test | red today | green |
|---|---|---|
| `a_type_with_a_row_parameter_holds_a_task` | E0432, E0400, E0423 | none, 42 |
| `a_row_argument_is_written_as_a_row` | E0100 "expected a type" at `{` | 2, 3; a `Log` task as `Task({})` is E0423 |
| `a_row_parameter_is_found_through_another_declaration` | 6 E0432, 2 E0400, 3 E0423, E0420 | the example's 9 lines, 3 |
| `an_effect_with_a_row_parameter_spawns_tasks_that_spawn` | E0432 x2, E0423, E0420 | 6, annotated and not |
| `a_parameter_is_a_type_or_a_row_by_name` | part 1's message | five named messages |
| `a_labels_written_arguments_are_checked` | `[]` (the arguments dropped) | E0423 |
| `the_printer_writes_a_row_argument_as_a_row` | does not check | `Task({Log})`, `forall a. fn(Task({a})) / {a} -> Int` |

Natively (`crates/codegen/tests/native_row_parameters.rs`, 3 tests, written after
the checker is green, each red then by name or by a wrong answer): the first test's
program, the spawning scheduler, the round robin; each binary's output and value
equal the evaluator's.

## Steps

1. AST (D7): `RowAnn { labels: Vec<LabelAnn>, tail }`, `LabelAnn { name, args }`;
   `TypeAnn.row` and `FnDecl.effect_row` hold it; `ast::row_tail` goes. The parser
   reads a label's arguments (a type, or a row `{..}`), and a row as a type
   argument, into the phantom's annotation `fn() / {..} -> $Row`. A grant still
   names effects only (its arguments skipped, as before).
2. Every walker of rows: `modules.rs` (three), `core.rs` (one), `types.rs` (four).
3. Kinds (D3): `types::decl_kinds(module)`, the fixpoint, shared by inference and
   Core's lowering.
4. Inference: declarations' elaboration with row parameters, own-effect labels and
   written label arguments; annotations' elaboration with label and row arguments;
   constructor schemes quantify row parameters; `OpInfo` carries its effect's row
   parameters, instantiated fresh by a perform, a handle and `with any`; the
   printer writes a phantom as its row.
5. Core (D6): generic field types and operation types write row parameter `i` as
   `Open(i)`, label arguments elaborated; mono's instance builder substitutes rows
   first.

## Predictions

- Gate: 1355 + 2 x 7 (root) + 3 (native) = **1372 passed, 148 suites** (145 + 2 + 1).
- No existing test's expectation changes (`fn_fields.rs`'s "takes type arguments"
  holds for the new message, after the first gate run caught a draft's wording).
  One test is renamed:
  `row_variables.rs`'s `a_declarations_row_variable_waits_for_part_2` becomes
  `a_declarations_row_variable_must_be_its_parameter` (its assertion, "cannot name
  a row variable", holds for the new message).
- `gc_mark` unchanged (637 bytes).

## Negative controls (each reverted, the file compared byte for byte)

- K1, kinds without the fixpoint (row tails only): the round robin fails
  (`Queue`'s parameter read as a type).
- K2, a perform's row parameters not instantiated fresh (the declaration's row
  variable shared): the spawning scheduler or the round robin fails.
- K3, mono without the row substitution: the native tests fail (a task stored CPS
  read back direct, or refused by name).
- K4, label arguments dropped again: `a_labels_written_arguments_are_checked`.

Outcomes: K1, K3, K4 failed as predicted (K4 also fails both schedulers, E0424).
K2 failed NOTHING: no test used an effect's row parameter at two rows. Added
`an_effects_row_parameter_is_instantiated_per_use` (red under K2, E0420), so the
root suite has 8 tests and the gate prediction moves to **1374 passed, 148
suites** (measured: 1374, 148).

After the review (spec, section 3's review list): five root tests (one per
finding, and the duplicate-label case found checking F3's fix) and one native
test, each red first; `occurs_in_rows.rs`'s `fn g(k) { op(k)  k() }` changes
from E0424 to clean (named in the commit). Gate prediction: 1374 + 2 x 5 + 1 =
**1385 passed, 148 suites**. K5 (`settle_duplicates` off) fails the
duplicate-label test alone.
