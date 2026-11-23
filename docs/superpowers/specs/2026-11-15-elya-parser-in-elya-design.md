# Self-hosting, part 4: the parser in Elya

**Status:** 4a and 4b implemented; 4b verification recorded 2026-11-23 in
section 3, including the remaining debug evaluator runtime budget. The design
was made on 2026-11-15 on `main` in an interactive session: HANDOFF step 1,
part 4 (spec 2026-11-14-elya-self-hosting R2, R3). Design choices are Claude's
(rule 2); the parser is a Codex job to this spec.

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

**D6. The checker's verdict has a print form too, `elya::check_dump`**
(`elya check --dump <file>`, added 2026-11-15 ahead of the checker's port):
`scheme <name> <scheme>` for every top-level function, sorted by name (the
order inference visits its groups in is not part of the verdict); `type
<start> <end> <type>` for every node, its variables named afresh on each
line, so two checkers that number variables differently agree; then the
diagnostics, as the parser's. Errors before the checker (parse, names,
modules) are printed alone. Whether the Elya checker must match the
diagnostics' words or only their codes and spans is the maintainer's call
(asked 2026-11-15; until answered, words too). Pinned in
`tests/check_dump.rs`.

**D5. Not here:** name resolution, the checker (part 4's later halves, or
part 5's), and any change to the language's grammar: a grammar change from
now lands in both parsers (R4).

## 2. Tests (planned)

`tests/selfhost_parse.rs`: for every file in D3's list, the Elya parser's
output (evaluator) against `ast_dump::dump`; a native run on the largest;
4b's error programs once 4b lands. `tests/ast_dump.rs`: the dump on a program
that uses every node, pinned, so a change to the form is seen.

## 3. Measured after

### 4a (historical record)

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


### 4b: diagnostics and recovery (2026-11-23)

The 4b implementation is based on `42d13c5`. `parse_dump` now writes the
complete retained tree followed by the lexical diagnostics and then the parser
diagnostics, exactly as `elya parse --dump` does. Its immutable state carries the
cursor, diagnostics, raised-error count, EOF suppression, declaration resume
point, shared nesting depth and expression height. Recovery retains bad binding
names and function names, synthesizes the same `Error` nodes, and skips the same
tokens as `src/parse.rs`.

`pub fn parse_tree(src: String) -> Parsed(Module)` remains available to the
checker, with the original `Parsed(a, Int)` and `ParseError(String, Int, Int)`
constructors. Internal recovery uses a separate `Read` type. The unchanged
`tests/selfhost_check.rs` passed: **203 comparisons**. Neither the checker nor
its tests, the Rust compiler sources, or existing expected values were changed.

The imported lexer still has its established standalone dump contract. The
parser supplements its E0001/E0003 diagnostics with lexical float E0002,
interleaved at the original scan positions (E0003 first at an identical span).
Exact decimal halfway comparisons reproduce binary64 overflow and underflow;
large exponents follow Rust's actual exponent-accumulation rule. Independent
checks agreed with Rust on **1,001 evaluator payloads** and **1,083 native
payloads**, including exact halfway values, randomized spellings and
100,000-digit mantissas. Integer E0002 belongs to the parser and remains after
all lexical diagnostics.

The shared `parity_corpus()` adds malformed and token-deleted inputs while
retaining the original clean `corpus()` API for the unchanged checker test.
Both parser suites compare every input. Native execution uses the shipped
`main`; the evaluator batches file reads through a test-only IO entry point,
keeping every shipped parsing function unchanged. Each evaluator worker owns
its module and files, and the harness checks that every case is assigned and
compared exactly once. Both back ends report the first differing dump line.

Rust literal discovery includes root tests, crate tests, other test-support
fixtures and inline test modules. It decodes raw and escaped strings, skips
comments, and filters assertion text, locator strings, bare tags and
uninstantiated format templates. Duplicate source texts are counted once across
the embedded scopes. A paired manifest proof established identical inputs and
Rust dumps in the evaluator and native suites.

The focused malformed set asserts coverage of **all 55 parser diagnostic
messages**, all seven diagnostic codes (E0001–E0003, E0100–E0103), every operator
precedence level, delimiter recovery, rows, declarations, handlers, match arms,
multiple errors, and nesting depth/height in every composite expression context.
Clean cases also pin both sides of the nesting boundary. Generated cases delete
one middle token from **every nonempty clean input**, including the focused
clean inputs. **N = 1**; two empty/comment-only inputs have no token to delete.

Final full-corpus inventory (the same **3,345 inputs, 1,102,488 bytes** in both
back ends):

| Source | Clean | Malformed |
| --- | ---: | ---: |
| Repository `.elya` files | 40 | 0 |
| Embedded root-test inputs | 722 | 73 |
| Embedded crate/support/inline-test inputs | 676 | 20 |
| Additional focused inputs | 85 | 208 |

These 1,824 base cases are followed by 1,521 generated variants, which are
compared whether they parse clean or produce diagnostics. There are
**93 distinct malformed extracted inputs**. N=1 is the minimum
positive deletion count; all 1,521 deletions are retained. The largest clean
input is `selfhost/parse.elya` (**83,893 bytes**). The largest malformed input
removes the token at zero-based index 12,313 from that source (**83,892 bytes**).

The complete stable evaluator run passed in **74.560627689 seconds**, with two
workers under a two-CPU quota, including front ends, corpus construction, input
files and batched IO. This **exceeds the original one-minute budget**. No input,
focused family or discovery scope was dropped to improve that number. The
initial reconstructed run took 84.843762834 seconds before the final lexer
optimizations. The native parser has no speed acceptance threshold (D4).

The final lexer fast paths cache ASCII run bounds, append whole numeric/string
runs, and recognize punctuation that cannot start a longer operator without
probing every operator spelling. Token kinds, spans, regex consumption,
payloads and diagnostics are unchanged. The existing standalone lexer tests
passed unchanged, including **8,991 independent edge cases**. Parser arm
ordering and pure lookahead ordering reduce evaluator dispatch overhead without
changing grammar or recovery.

All three negative controls were run against
`largest_input_agrees_natively_and_is_measured`, each exited **101** with a byte
parity failure, and each was restored byte for byte. The restored parser SHA-256
was `811e5f53b657849b7d89cd5dee43c00eadeed0efac6d7f1e04b12eed7d655d28`:

| Control | Mutation | Input that caught it | First difference |
| --- | --- | --- | --- |
| K1 | Recovery consumes one extra token at a statement boundary | `tests/diagnostics_edges.rs`, byte 5197 | line 2: retained function ends at 81 instead of 34 |
| K2 | E0100's diagnostic end is one byte too far | `tests/any_handler.rs`, byte 2479 | line 9: span `57 59` instead of `57 58` |
| K3 | Parser diagnostics precede lexer diagnostics | focused `module-garbage` | line 14: E0100 precedes E0001 instead of following it |

Optimized measurements were taken after the full gate finished, with the native
comparison pinned to CPU 1 and no other test workload running. Compilation is
excluded. Both native and Rust are optimized; Elya native is O2. Medians:

| Input | Bytes | Native process + file + lex + parse + dump + capture (7 runs) | Rust lex + parse (101 runs) | Rust tree + diagnostics dump (101 runs) | Rust lex + parse + dump (101 runs) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Largest clean: `selfhost/parse.elya` | 83,893 | 49.630710 ms | 2.227088 ms | 1.559628 ms | 3.851725 ms |
| Largest malformed: token-12,313 deletion | 83,892 | 47.071485 ms | 2.015538 ms | 1.388219 ms | 3.428314 ms |

Command: `RUST_TEST_THREADS=1 taskset -c 1 cargo test --release -p elya-codegen --test selfhost_parse -- --nocapture`.
Rust's source is already in memory; its dump timing starts from the parsed
module, and its combined timing includes both phases. Native includes process
startup, file IO and captured stdout. These are stated scopes, not an isolated
comparison of parsing speed. The native runtime's final Unit value line alone
is removed, as in 4a; every byte printed by the parser is compared.

The same complete corpus also passed in the **optimized Rust evaluator in
18.295462491 seconds**, with two workers, including front ends, corpus and files:
`cargo test --release -p elya --test selfhost_parse -- --nocapture`. This remains
an evaluator run, not a native Elya execution. The default debug evaluator
measurement above is retained: no repository build profile was changed to hide
its one-minute-budget miss.

The pristine baseline `sh scripts/check.sh` on `42d13c5` exited zero, including
configuration C and both QEMU boards. No board skip or changed threshold was
used. Counts include every suite once per configuration, including zero-test
library/doctest suites:

| Source | A passed / suites | B passed / suites | C passed / suites | Total passed / suites |
| --- | ---: | ---: | ---: | ---: |
| Before, pristine `42d13c5` | 636 / 80 | 1,022 / 114 | 8 / 2 | 1,666 / 196 |
| After 4b | 636 / 80 | 1,022 / 114 | 8 / 2 | 1,666 / 196 |

Both original scripts exited **zero**, with **zero failures and zero ignored
tests**. The native array test, using its existing faster-of-three measurement,
passed under the full gate's load; no pinned rerun or workaround was needed.
The unchanged checker test passed explicitly and within both gate configurations.
The parser proof expands existing test functions, so the number of test
functions/suites is unchanged while differential input coverage increases.
Protected-source and checker diffs were checked empty. Independent source and
corpus reviews found no remaining functional defect or suspicious Rust recovery.
The remaining limitation is the debug evaluator's runtime budget, stated above.
