# Aliases that rebind their target: rename, do not keep

**Status:** done (2026-10-24, branch `auto/elya`, a scheduled run). HANDOFF
step 1. The design choices are Claude's under the maintainer's delegation (rule
2), recorded here.

## 0. Measured first (at c5c2a44, gate 1238 / 124)

`effect S { fn get() -> Int }`, `let mk = fn(n) { fn(g) { g() + n } }`,
`let mk2 = mk`, then `mk2` used so its result is called with an `S` closure:

| program | the alias's body | evaluator | native |
|---|---|---|---|
| a1 | rebinds `mk` in a block (`{ let mk = 0  mk2(1 + mk) }`) | 8 | refused: "effect-polymorphic function used at a user effect" |
| a2 | rebinds `mk` as a lambda's parameter | 8 | refused (the same) |
| a3 | rebinds `mk` in a `match` pattern | 8 | refused |
| a5 | rebinds it once, and uses `mk2` at two types | 113 | refused |
| a4 | rebinds nothing; a top-level function is also named `mk` | 8 | refused |

`anf`'s alias rule (spec 2026-10-22 D3) substitutes `mk` for `mk2` so that
row specialization sees each use of `mk` at its own instantiation. It keeps the
`let` (and the refusal follows) when the body binds `mk` again, since renaming
would capture; and when a top-level function shares `mk`'s name, a guard from
before captures were decided by scope (2026-10-23).

## 1. Decisions

- **D1. Capture-avoiding substitution.** When the body binds the target `y`
  again, those inner binders (a `let`, a lambda's parameter, a pattern's
  variable, a clause's parameter, a `return` binder) are renamed to fresh
  names (`y$alias.k`, a name no source can write) together with their uses,
  and then the alias is substituted. a1, a2, a3, a5 run.
  - *Rejected:* keeping the alias and cloning it as a generic local. It would
    duplicate the specializer's work for a second binder of the same value.
- **D2. A top-level function sharing the target's name no longer blocks it.**
  Closure conversion decides a lambda's captures by scope since 2026-10-23,
  and every later pass resolves a name to the innermost local first; the guard
  is dropped, and a4 runs. Measured with the review's original shape (a
  shadowing `y` planted in a lambda: 2004 for 1007 before scope captures).
  - *Rejected:* keeping the guard "conservatively": a refusal with no failure
    behind it.
- The refusal stays where it is right: an alias of a top-level function, and
  of a closed-row local (spec 2026-10-22 D3's other exclusions, each found by a
  wrong value). `an_alias_of_a_generic_local_used_at_a_user_effect_is_refused_by_name`
  no longer has a program that reaches the guard through the pipeline (none was
  found: data, `if`, `id(mk)`, a pattern binder all run); its program now runs
  natively (CHANGED EXPECTATION, by D1), and the guard is pinned by a unit test
  of the prepass without the pass (`the_alias_guard_refuses_by_name_without_the_pass`).
  `anf`'s unit test `an_alias_whose_target_is_rebound_is_kept` now asserts the
  renaming (CHANGED EXPECTATION, by D1).

## 2. Evidence (planned)

`crates/codegen/tests/native_aliases.rs` (differential, counted once): a1-a3,
a5 and a4, plus the 2004-for-1007 shape. Negative controls: D1 without renaming
the uses (only the binder): a wrong value or a refusal; D2 with the old guard:
a4 refused again.

## 3. Outcome and review

Red as predicted (a1, a4 refused); gate 1241 / 125 as predicted. Controls K1 (a
binder renamed without its uses: a compiler panic) and K2 (the old guard: a4
refused) in `native_aliases.rs`'s header.

The independent review ran ~30 programs (every binder kind, nested and repeated
rebinding, chains, the alias itself rebound, parameter and pattern targets, a
top-level function of the same name called and used as a value, `with any`
operation aliases, multi-shot) on the new build and the base: all agree; the
base refused most of them. Sweep: 982 programs, 504 same, 0 failing; the base's
six extra refusals ("effect-polymorphic function used at a user effect") are
gone. The remaining four native refusals are older: two `main`s of a non-`Int`
value, two "too many type instances".
