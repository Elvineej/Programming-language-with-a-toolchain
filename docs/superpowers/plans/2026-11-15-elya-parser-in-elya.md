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
