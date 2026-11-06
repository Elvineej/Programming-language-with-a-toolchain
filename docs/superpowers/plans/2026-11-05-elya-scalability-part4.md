# Plan: compiler scalability, part 4 (generalization by levels)

Spec: `docs/superpowers/specs/2026-10-31-elya-scalability-design.md`, section 1b
(D14-D16). Base: `main` after part 3b. Found by part 3a's review (aliases of a
row-polymorphic local, 26.7 s at 800 in a debug front end); then measured.

## Steps

1. Measure (release CLI, `elya check`, 1000 / 2000 / 4000): closures over a
   function parameter 1.65 / 18.1 s / killed; aliases of one 2.1 / 14.4 /
   110 s; non-value `let`s beside generalized ones 0.46 / 2.7 / 15.3 s; a
   recursive group with local `let`s 2.7 / 21.5 / over 120 s. Callgrind: 62%
   in `resolve` under `generalize`, reading every open entry, each through a
   row chain a link longer per call (traced: `bind_row 2 := 4`, `3 := 4`,
   `4 := 6`, .. from `infer_call`'s unification of the callee's row with the
   call's).
2. Red tests: `tests/infer_complexity.rs` +4 (closures over a function
   parameter, aliases of one, open lets beside generalized ones, a recursive
   group with local lets), steps and allocations at n and 2n, linear. Before:
   7.55, 7.35 (cubic), and the two D2 shapes quadratic.
3. D15: `unify_tails` binds the younger row variable to the older. The first
   two to 3.82 (quadratic: the open entries remain).
4. D14: levels on every variable (`fresh`, `fresh_row`), lowered by `bind`,
   `bind_row` and a non-value `let`; `generalize` by level, with the kept
   sets read as before; `check_levels` (and `ELYA_CHECK_LEVELS`) computing
   both ways. `env_free_*` deduplicate by a set. The first three linear; the
   group 3.29.
5. D16: a flush reads the group's parameters only for a member inclusion.
   The group linear.
6. `tests/levels_agree.rs` (new, 3): the examples, ui tests and row-parameter
   program; ten programs written for levels; the open shapes, small; all
   under `check_levels`, the written ones checking clean.
7. The root and codegen suites under `ELYA_CHECK_LEVELS=1`: no difference
   anywhere (only the four new complexity tests fail, by design).
8. Controls K23-K27 (in the spec's evidence), each reverted and compared.
9. An independent review (a generator of 1000 programs: lets of lambdas,
   aliases, non-value lets, shadowing, lets in lambdas, arms and clauses,
   `with any`, recursion, annotations): 42 of its programs generalized
   differently by level. Two causes, fixed test-first:
   - a variable kept monomorphic above the `let`'s level stayed there, and a
     later `let` quantified it: what a `let` leaves unquantified is lowered
     to its level (control K30);
   - an annotation variable's rows were not kept (older than levels: D2
     quantified them too, against its own assertion): kept (K31), and three
     programs that checked clean are refused (E0420), as the rule that an
     annotation variable is one type says.
   `tests/levels_agree.rs` +3 (the review's programs; the annotation row;
   a hidden `Log` still refused). After: the 1000 programs agree, and check
   as before levels.

## Predictions

- Gate: part 3b's count + 7 root tests, counted twice = **+14**, and **+2
  suites** (`levels_agree` in both configurations); with the review's 3,
  **1443 + 20 = 1463, 157 suites**.
- No existing test's expectation changes: the schemes D14 computes are D2's
  (checked over both suites), and D15 changes which row variable represents
  a class, not what it resolves to.
- `gc_mark` untouched (no runtime change).
