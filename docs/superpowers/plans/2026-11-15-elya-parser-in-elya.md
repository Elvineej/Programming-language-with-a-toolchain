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
4. **4b, errors and recovery**, after 4a lands.

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
