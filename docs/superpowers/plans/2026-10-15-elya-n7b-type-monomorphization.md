# N7 part 2: type monomorphization -- plan

Spec: `docs/superpowers/specs/2026-10-15-elya-n7b-type-monomorphization-design.md`. Branch
`auto/elya` at `0e92180`. Baseline Linux gate: 886 passed, 85 suites.

1. **Front end.** `CoreModule::generic_types` (`CoreGenericType`: name, parameter count,
   constructors whose fields carry `Ty::Var(i)`); `types` unchanged (snapshots unchanged).
   Red first: `tests/core_lowering.rs::parametric_declarations_are_recorded_in_generic_types`
   (predicted red: no such field -- a compile error, then the field check). Root crate:
   counts twice, +2.
2. **`mono.rs` unit tests** (codegen lib, once), each red first against a stub `run` that
   returns its input:
   - `no_type_variable_survives` (q1, q5, q9): predicted red, `Ty::Var` found.
   - `one_clone_per_instantiation_shared_across_sites`: `id` at `Int` from two call
     sites and at `Bool` -> 2 clones, no original. Predicted red: 0 clones.
   - `a_generic_local_nullary_ctor_is_cloned_per_use` (r11): 2 `let` clones of `n`.
   - `ambiguous_variables_default_to_unit_once` (r19): one clone of `seven`, parameter
     `List(Unit)`.
   - `a_row_only_type_variable_is_refused_by_name` (r10): `"type variable only in an
     effect row"`. Predicted red: `Ok`.
   - `type_clone_budget`: a chain of 300 instantiations -> `"too many type
     specializations"`.
   - `instances_are_lazy_for_a_non_regular_type` (r18): exactly 2 instances built
     (`Nest(Int)`, `Nest(List(Int))`) plus `List(Int)`, i.e. 3; the field type
     `Nest(List(List(Int)))` named, not built.
   - `type_instance_budget`: 300 distinct instances -> `"too many type instances"`.
   +8.
3. **Native** (`native_codegen.rs`), red first (every program refused today):
   `TYPE_VARS` corpus, 19 programs (q1-q9, r11, r13, r15, r17, r18, r19, r21, r22, r23,
   r25), value and differential: +2. `a_parametric_list_survives_collection` (r20:
   5000050000 with `collections > 0`): +1.
4. **Changed expectations (called out in the commit):** `lib.rs`
   `a_polymorphic_function_refuses_the_whole_module` and
   `a3_an_unconstrained_polymorphic_effect_meets_the_ty_var_refusal` assert the refusal
   this slice lifts; both now assert the module compiles (the generic original is not
   emitted). Count unchanged.

Predicted gate: 886 + 2 + 8 + 3 = **899 passed, 85 suites**.

Negative controls (to record in the test file header):
- K1, references not redirected to clones: the corpus is refused ("unrepresentable
  type"), by name.
- K2, a parametric field's mask computed before substitution (`Ty::Var` counted as
  not a pointer): r20 fails (a collected `Some` read back), distinct from K1.
- K3, ambiguous variables not defaulted: r19 refused "unrepresentable type".
- K4, the effect-position check off: r10 compiles (r24's string typed as `Unit`).
- K5, instances built eagerly (fields instantiated transitively): r18 hits the
  instance budget.

Measured: **899 passed, 85 suites** (as predicted). r26 joined `TYPE_VARS` after K3
(no count change).

Review: F1 (row tails in instance keys) and F2 (phantom effect parameters) fixed red
first -- four corpus programs (no count change), one unit test (+1), and
`effect_params_used` checked in the existing new lowering test. Final gate predicted 899 + 1
= **900 passed, 85 suites**; measured 900.
