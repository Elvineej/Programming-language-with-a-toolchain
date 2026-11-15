# Self-hosting, part 4: the parser in Elya

**Status:** designed (2026-11-15, on `main`, an interactive session). HANDOFF
step 1, part 4 (spec 2026-11-14-elya-self-hosting R2, R3). Design choices are
Claude's (rule 2); the parser is a Codex job to this spec, checked here.

## 0. Measured first

- `src/parse.rs` is 2,297 lines: a recursive-descent parser with precedence
  climbing (binding powers per operator), statement and block recovery, and
  about thirty diagnostics (E0100 to E0130).
- Its only printer, `ast::pretty`, drops what a parser must be checked on:
  imports, annotations, effect rows, `pub`, `linear`, `multi`, `any`,
  `borrow`, every span. Two parsers could disagree on all of those and print
  the same.
- Part 3's lexer in Elya (`selfhost/lex.elya`) prints a dump of its tokens; a
  parser needs the tokens as values, read by position: until part 2b, a list
  read by position was quadratic. Arrays (part 2b) and maps (`elya/map`) are
  in.

## 1. Decisions

**D1. The parser is `selfhost/parse.elya`,** a program like the lexer:
it reads a source file (`io.read_file` of its first argument), lexes it, parses
it, and writes the dump (D2) on standard output. The lexer it uses is
part 3's, changed to answer its tokens as values: `selfhost/lex.elya` keeps
its dump as one user of a function that answers an `Array(Token)`, `type
Token { Token(Int, Int, Int, String) }` (kind as a number, start, end,
payload). The parser reads tokens by index through `borrow` parameters
(part 2b), the first real user of arrays.

**D2. One print form, both sides: `elya::ast_dump::dump(&Module, &str)`,**
one node a line in pre-order: `<depth> <Kind> <start> <end>` and the node's
payload, children one deeper in source order. Complete: every field the
parser sets and every span; a node with no span of its own prints `- -`.
Text is quoted and escaped as the lexer's dump; a float literal prints its
source text (as part 3, Elya cannot yet read a float's value). The kinds and
payloads are the code's doc (`src/ast_dump.rs`); the CLI prints it, `elya
parse --dump <file>`, so a job can compare without the test harness.

**D3. In two steps, each byte for byte against `src/parse.rs`:**
- **4a, programs that parse clean:** every `.elya` file the repository holds
  (examples, `selfhost/`, the standard modules, every program in `tests/`
  that has no parse error), the dump identical.
- **4b, programs that do not:** the diagnostics, code, span and message, in
  order, and the dump of what recovery kept. Recovery is the parser's
  hardest half and comes second.

**D4. Speed is measured, not required yet:** the time of the Elya parser,
native, against `src/parse.rs` on the largest inputs; the switch (part 6)
needs it within a small factor, and the measurement says where it stands.

**D5. Not here:** name resolution, the checker (part 4's later halves, or
part 5's), and any change to the language's grammar: a grammar change from
now lands in both parsers (R4).

## 2. Tests (planned)

`tests/selfhost_parse.rs`: for every file in D3's list, the Elya parser's
output (evaluator) against `ast_dump::dump`; a native run on the largest;
4b's error programs once 4b lands. `tests/ast_dump.rs`: the dump on a program
that uses every node, pinned, so a change to the form is seen.

## 3. Measured after

4a is implemented in `selfhost/parse.elya`. Its immutable, spanned AST mirrors
`src/ast.rs`; a single token array stays in `parse_dump`, and all readers take
it by `borrow`. Cursors are returned with parsed values, and output is built
with `Buffer`. Strings retain the lexer's escaped payload and floats retain
their source spelling, as required by the dump. No grammar or Rust compiler
changes were needed. Errors detected by this port print only
`error <code> <start> <end>`; complete diagnostics and recovery remain 4b.

The differential test discovers every `.elya` file recursively in `examples/`,
`selfhost/`, `src/std/`, and `tests/`, and extracts raw and ordinary string
literals from `tests/*.rs` and `tests/programs/*.rs`. The Rust parser filters
out programs with errors; duplicate embedded strings are compared once. On
2026-11-15 it compared **39 repository files** (none skipped), **712 distinct
embedded programs**, and **80 additional focused programs**: **831 inputs,
241,784 bytes**. These include the pinned every-node program and all binary
operators at every precedence boundary. The parser also parses its own source.
Both back ends report the first differing dump line.

The native test builds the parser at O2 and compares its output on the largest
input, `selfhost/parse.elya` (**56,237 bytes**). The native runtime's final
`0` value line for `main` returning `Unit` is removed by the harness; every
byte printed by the Elya program itself is compared. Compilation is outside
the timings. The following medians were measured on the same Linux x86-64
machine with LLVM 18:

| Measurement | Median | Runs |
| --- | ---: | ---: |
| Elya native: process startup + read file + lex + parse + dump + capture stdout | 65.696 ms | 7 |
| Rust `src/parse.rs`: lex + parse, source already in memory | 1.603 ms | 101 |
| Rust `ast_dump::dump`, parsed module already in memory | 1.094 ms | 101 |

Command: `RUST_TEST_THREADS=1 taskset -c 1 cargo test --release -p elya-codegen --test selfhost_parse -- --nocapture`.
Rust is optimized too. These are explicitly different scopes, so the timing
does not isolate parsing speed; the native test prints each scope on every run.
This final measurement ran on CPU 1 after the gate suites completed. Speed has
no acceptance threshold in 4a (D4).

All three negative controls were actually run, failed, and were restored byte
for byte (`git diff --no-index --quiet` against the saved source and
`git diff --quiet` for tracked files):

- **K1:** change `+`'s binding power from 50 to 60. The evaluator comparison
  fails on `examples/06_structured.elya`, dump line 244 (`Binary *` versus `+`).
- **K2:** end a call's span at its callee. The evaluator comparison fails on
  `examples/01_hello.elya`, line 9 (`Call 41 51` versus `Call 41 67`). The
  native largest-input test also fails (line 263, the containing arm's span).
- **K3:** consume the `borrow` modifier but store an ordinary parameter. The
  evaluator comparison fails on `selfhost/lex.elya`, line 1807 (`Param -`
  versus `Param borrow`).

The evaluator test first failed while the parser file was absent. Independent
review of the complete parser and harness found no remaining defects; further
CLI probes covered contextual names, grouped calls, newline layout, skipped
grant arguments, empty matches/handlers, repeated returns, and row phantoms.

Gate measurements use `RUST_TEST_THREADS=1 taskset -c 1 sh scripts/check.sh`
after activating the environment's Rust/LLVM 18/board tools, with no board skip.
The baseline is the pristine `66b9077` worktree. Its original script stopped
at the existing `reading_an_array_by_index_is_linear_natively` timing assertion
(0.213 s versus 0.020 s), after 1,381 passes and one failure in 156 suites.
Other baseline attempts also failed that timing ratio. Independent investigation
found elevated kernel CPU time on the first large allocation; the exact cause
is unproven. The unchanged test passed four isolated CPU-1 runs and then passed
in the full native follow-up. No test, threshold, runtime, or compiler was changed.

To complete baseline coverage after that fail-fast stop, the LLVM native/CLI
suites were run with `--no-fail-fast`, followed by the frontend doctests and
all configuration-C clippy and QEMU commands. Each follow-up command exited
zero. Counting each suite once within its configuration, the measured baseline
is **A: 621 passed / 76 suites; B: 1,002 / 107; C: 8 / 2**: **1,631 passed /
185 suites** overall (**1,623 / 183** for A/B). This is the completed baseline
with unchanged reruns, not a claim that the original script exited zero.

The final original `check.sh` likewise stopped on the unchanged native-array
ratio (0.218 s versus 0.020 s), after **1,383 passes, one failure, 158 suites**.
The same complete follow-up commands all exited zero, including the native
parser test and both QEMU suites. The completed configuration counts are:

| Source | A passed / suites | B passed / suites | C passed / suites | Total passed / suites |
| --- | ---: | ---: | ---: | ---: |
| Before (`66b9077`) | 621 / 76 | 1,002 / 107 | 8 / 2 | 1,631 / 185 |
| After | 622 / 77 | 1,004 / 109 | 8 / 2 | 1,634 / 188 |

A/B after is **1,626 passed / 186 suites**, exactly the plan's prediction
(+2 evaluator instances, +1 native instance). These totals count each suite
once per configuration after unchanged reruns; they do not conceal the two
original script failures. The PR remains a draft because a complete original
`check.sh` run has not exited zero on this machine. No existing expected value
or protected compiler source was changed to make the gate pass.
