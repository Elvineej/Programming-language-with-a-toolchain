# Plan: the parser in Elya (self-hosting, part 4)

Spec: `docs/superpowers/specs/2026-11-15-elya-parser-in-elya-design.md`.

## Steps

1. **The print form, here (done first):** `src/ast_dump.rs` (`dump`,
   `dump_diagnostics`), `elya parse --dump <file>`; `tests/ast_dump.rs` pins
   it on a program using every node; a CLI test in `run_cli.rs`. Control K1
   (a parameter's `borrow` flag not printed) fails the pinned test.
   Prediction: gate 1614 + 2 x (2 + 1) = **1620 passed, 182 suites**;
   configuration C unchanged (8, 2).
2. **The lexer's tokens as values, done** (ChatGPT's, Codex being out of
   usage; checked here): `selfhost/lex.elya` answers `lex(src)` (a `Lexed`
   of an `Array(Token)` and an `Array(LexDiag)`) and `tokens(src)`, kinds
   numbered in `src/lex.rs`'s order; its dump is printed from the arrays and
   `tests/selfhost_lex.rs` agrees unchanged. Natively 1.5 MB lexes in 1.29 s
   (0.82 s before): a `Token` per token and the pair rebuilt each time; the
   remedy waits for a "never allocates" fact per function (step 3's roots).
   A parser in `selfhost/` imports it as `import lex / {IO}` (the module
   reader resolves from the root file's directory; the lexer keeps its `main`,
   which performs `IO`); a test's reader must map `lex` to its source.
3. **4a, the parser** (a Codex job, the large one): `selfhost/parse.elya`;
   `tests/selfhost_parse.rs` comparing its output with `elya parse --dump`'s
   on every file of the spec's D3 list, in the evaluator and natively; the
   native time against `src/parse.rs` measured and written in the spec.
4. **4b, errors and recovery:** implemented against `42d13c5`. The retained
   tree and complete diagnostics agree in the evaluator and native suites.
   `parse_tree(src: String) -> Parsed(Module)` and the checker's existing
   constructors remain compatible; the checker and its tests are unchanged.
   Measurements, controls and the remaining debug runtime budget are recorded
   in the spec's section 3.

## 4a verification prediction

One new root-crate evaluator test runs in configurations A and B; one new
codegen native test runs in B. The shared corpus helper adds no test suite.
Prediction against the recorded baseline: **1623 + 2 + 1 = 1626 passed,
183 + 2 + 1 = 186 suites** in A/B; configuration C remains **8 passed,
2 suites**. No existing expected values or compiler source files change.
The before/after gate measurements and native timings are recorded in the
spec's section 3 once all three configurations finish.

Observed completed coverage matches the prediction: A/B **1626 passed,
186 suites**; C **8 passed, 2 suites**. Both original `check.sh` invocations
failed an existing native-array timing ratio assertion. Complete unchanged
`--no-fail-fast` LLVM follow-ups and all C commands passed; the spec records
both original partial counts and complete configuration counts. The PR is a
draft pending a green original gate; no test threshold or compiler was changed.


## 4b verification (2026-11-23)

The existing evaluator and native parser suites now compare the shared full
corpus: **3,345 inputs**, including **93 distinct extracted malformed inputs**,
**208 focused malformed programs**, and **1,521 one-token deletions (N=1)**.
Every nonempty clean input produces a deletion; two tokenless inputs are counted
separately. The legacy clean corpus API is retained for the unchanged checker
suite. No protected compiler source or existing expected value changes.

All 55 parser message texts and all seven parser/lexer diagnostic codes have
focused coverage. Native execution covers every case and separately measures
both largest clean and malformed inputs against optimized Rust. K1 (one extra
recovery token), K2 (diagnostic end +1), and K3 (lexical diagnostics last) each
failed the native parity test and were restored byte for byte.

The pristine baseline on `42d13c5` is **1,666 passed / 196 suites** across A/B/C.
Extending the existing test functions predicts no change to those counts;
the additional proof is the expanded differential corpus. The unchanged checker
suite passed **203 comparisons**.

The final stable debug evaluator run passed in **74.560627689 seconds** on two
workers under a two-CPU quota. This exceeds the original one-minute budget; N=1
is the minimum positive choice, and all coverage is retained. The lexer scan and
parser dispatch optimizations are included, with unchanged standalone lexer
expectations. This budget limitation is explicit in the PR rather than hidden
by dropping cases or changing test profiles.

Measured final original gate: **1,666 passed / 196 suites**, zero failures and
zero ignored, exactly the baseline and prediction. A: **636 / 80**; B:
**1,022 / 114**; C: **8 / 2**. Both before/after scripts exited zero, including
QEMU. The array ratio passed under load without a pinned retry.

The complete optimized evaluator proof also passed in **18.295462491 seconds**;
this is reported separately from the default debug result. The final optimized
native measurement, pinned after the gate, is **47.071485 ms** on the largest
malformed input versus Rust **3.428314 ms** for lex + parse + dump (explicitly
different process/IO scopes). The spec contains all phase medians and scopes.
Functional parity and compatibility gates are complete; the debug one-minute
budget remains an explicit limitation, with all coverage retained.
