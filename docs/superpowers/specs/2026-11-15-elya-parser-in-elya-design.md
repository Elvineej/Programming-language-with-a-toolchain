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
