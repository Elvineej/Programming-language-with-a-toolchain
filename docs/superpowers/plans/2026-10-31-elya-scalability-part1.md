# Plan: compiler scalability, parts 1 and 2 (inference, the evaluators)

Spec: `docs/superpowers/specs/2026-10-31-elya-scalability-design.md` (D1-D8).
Base: `main` at c9405f1 (gate 1409 passed, 151 suites).

## Steps

1. Measures first, on the old code: `Infer::steps` and
   `elya::types::inference_steps` (D6); `Env::get_counting` and the lookups in
   `Interp::cost` (D8). No behaviour changes.
2. Red tests:
   - `tests/infer_complexity.rs` (new): seven shapes, steps and allocations at
     `n` and `2n`. Predicted red on the old code: many functions, lets under a
     parameter, closures calling a value (linear, ratio near 4), a chain, nested
     performs (allocations only), two towers (in proportion to their types). A
     long block passes (a control: already linear).
   - `tests/eval_complexity.rs`: a name down a long block, the machine and the
     tree oracle (red, ratio near 4); a binding made in place is seen by no one
     else (passes before and after: the guard D7 must keep).
3. D1-D5 in `src/types.rs`, D7 in `src/eval.rs`; the counters carried into
   the new code.
4. Green: the whole gate.
5. Negative controls, each reverted with `cmp` after:
   - K1, `free_vars` resolving at every node again: the chain's test fails;
   - K2, the environment's free variables read from every scheme again: many
     functions and lets under a parameter fail;
   - K3, `unify` resolving both sides whole at every level: two towers fail;
   - K4, a flush taking every inclusion: closures calling a value fail;
   - K5, no path compression: closures calling a value fail;
   - K6, `resolve_row` cloning a label before resolving it: nested performs fail
     (allocations);
   - K7, `open_covariant` resolving at every arrow: the chain fails;
   - K8, `Env::bind` always in place: the in-place guard fails (both programs);
     K9, `Env::bind` never in place: both lookup tests fail.
6. Independent review; findings fixed test-first.

## Predictions

- Gate: 1409 + 2 x (7 + 3) = **1429 passed, 153 suites** (one new root test
  file: +2 suites). No existing test's expectation changes.
- `gc_mark` unchanged (637 bytes); nothing native changes.
- Times at ten times the HANDOFF's sizes (debug CLI): the chain at 1000 links
  about 10 s (its types are n^2: 0.09 s at 100), a name down 50 000 statements
  about 2 s, nested performs at 500 (the parser's nesting limit is 1024 levels)
  about 5 s; many functions, lets under a parameter and closures at 40 000 under
  10 s each.
