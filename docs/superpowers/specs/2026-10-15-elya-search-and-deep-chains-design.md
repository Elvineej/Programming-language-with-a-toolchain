# A backtracking example, and multi-shot copies of deep handler chains

**Status:** done (2026-10-15). HANDOFF step 2. Choices are Claude's under the
maintainer's delegation (rule 2).

## 0. Measured first

- `examples/07_search.elya` (N-queens over `effect multi Choose { choose, fail }`, three
  handlers) ran on the first try at `511d6ed`: the evaluator prints the three lines and
  13215863724 (4 solutions of 6 queens, 92 of 8, the first 8-queens solution 15863724);
  natively the same bytes and value. Nothing in the language or the back end changed.
- The shape the multi-shot review could not build -- a continuation captured under MANY
  handler frames -- is buildable since recursion through a handle body is accepted:
  `nest(n)` handles `S` around `nest(n - 1)` and flips at the bottom, under a `with multi`
  handler that resumes twice. Natively, wall time of the whole program:

  | handler frames | time |
  |---|---|
  | 1 000 | 0.004 s |
  | 10 000 | 0.014 s |
  | 100 000 | 0.138 s |

  Linear, as `elya_cont_copy`'s parent map predicts. Values agree with the formula
  `(1 + n) * 10^6 + (2 + n)` and, at 1000, with the evaluator.

## 1. Choices

- **The example's handlers take the choice loop as an ordinary function** (`every`,
  `total`, `earliest` receive `fn(k) { resume(k) }`): an escaped multi-shot `resume`,
  which shows that a handler is just code -- the first-solution handler stops trying
  columns as soon as one `resume` answers `Some`.
- **`fail` is an operation of `Choose`**, not a separate effect: a handler covers every
  operation of its effect (E0207), and failure only means something to the explorer.
- **No timing in tests.** The deep chain is pinned by value: `MULTI_SHOT` at 1000 frames
  (against the evaluator) and a native-only test at 100 000; the table above is the
  linearity evidence.

## 2. Evidence

`tests/examples.rs::search` (snapshot), `crosscheck`'s effectful list,
`native_codegen.rs::the_search_example_runs_natively`, the `m-deep-handler-chain` row of
`MULTI_SHOT`, `a_multi_shot_copy_of_100000_handler_frames`. Plan:
`docs/superpowers/plans/2026-10-15-elya-search-and-deep-chains.md`.
