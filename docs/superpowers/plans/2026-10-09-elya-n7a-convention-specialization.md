# N7 part 1 — convention specialization and the upcast adapter: plan

Spec: `docs/superpowers/specs/2026-10-09-elya-n7a-convention-specialization-design.md`.
Branch `auto/elya` from `main` at `5799ba9`. Baseline Linux gate: 783 passed, 77 suites.
Root-crate tests count twice; codegen-crate tests (unit and native) once.

## Task 1 — `specialize.rs`: rows (red first)

Unit tests in `crates/codegen/src/specialize.rs` (LLVM-free; they count once), today's
result in brackets (the module does not exist, so all are red by compilation):
- s1 `a_pure_instantiation_makes_no_clone`: `apply` used only at a pure lambda -> 0 clones.
- s2 `one_clone_per_distinct_instantiation`: p13 -> 3 clones of `apply` (`{L}`, `{M}`,
  `{L, M}`), each referenced once.
- s3 `recursion_reuses_its_own_clone`: p5 -> exactly 1 clone of `mapl`, and the clone's
  recursive call names the clone.
- s4 `a_generic_callee_of_a_clone_is_specialized_too`: p12 -> 2 clones (`apply2.1`,
  `apply.1`).
- s5 `a_local_generic_lambda_gets_a_local_clone`: p6 -> one extra `let app.1`.
- s7 `the_clone_budget_is_refused_by_name`: a generated program instantiating `apply` at
  257 distinct effects -> `"too many convention specializations"`.

Native: the `CONVENTIONS` corpus (p1, p3, p5, p6, p9, p11, p12, p13, p14, p15, p19) with
values and differentially. [refused: "effect-polymorphic function used at a user effect"]
-> the evaluator's values (15, 842, 55110, 20, 18311, 5015, 22077, 111715, 126, 11046,
2001000).

Expectation changes the step requires (each replaced red-first, named in the commit):
- unit `an_effect_polymorphic_function_used_at_a_user_effect_is_refused` ->
  `..._compiles`;
- unit `a_let_bound_effect_polymorphic_lambda_used_at_a_user_effect_is_refused` ->
  `..._compiles`.

## Task 2 — the adapter and the deep guard

- s5b `an_upcast_becomes_an_eta_expansion`: p4 -> the `Var f` at `fn(Int) / {T} -> Int`
  is a `Lambda` whose body applies `f` at its own type.
- s6 `the_guard_compares_every_covariant_layer`: the prepass alone on p8 -> refused,
  `"direct function used where an effectful one is expected"` [today: accepted].
- native `a_direct_closure_returned_where_an_effectful_one_is_expected_runs` (p8):
  [2] -> 3306.
- `CONVENTIONS` gains p4 (23), p7 (11102), p17 (6365), p18 (271702111), p8 (3306).
- Replaced: native `a_direct_closure_used_where_an_effectful_one_is_expected_is_refused_by_name`
  -> `..._runs_through_an_adapter` (13); `a_direct_closure_upcast_through_a_return_binder_is_refused_by_name`
  -> `..._runs_through_an_adapter` (102).

Predicted final gate: new unit tests s1, s2, s3, s4, s5, s5b, s6, s7 = 8; new native
tests = 3 (`CONVENTIONS` value, `CONVENTIONS` differential, p8); 4 replaced, net 0.
**783 + 8 + 3 = 794 passed, 77 suites** (no new test binary).

## Task 3 — negative controls, each reverted (`git diff --quiet`)

- K1 references not rewritten (clones made, callers unchanged): the corpus is refused by
  the D16 guard again, by name.
- K2 adapter off: p4/p7/p17/p18 refused by name ("direct function used where an effectful
  one is expected"), and p8 too (the deep guard).
- K2b adapter off AND the guard back to the outermost layer: p8 prints 2 (the miscompile).
- K3 the clone key ignores the substitution (one clone per function): s2 fails (1 clone),
  and p13 natively is wrong or refused.

## Task 4 — close-out

Independent review of the diff; fix its findings test-first. Update HANDOFF (rolling
five) and `AUTO_RUN_LOG.md`.
