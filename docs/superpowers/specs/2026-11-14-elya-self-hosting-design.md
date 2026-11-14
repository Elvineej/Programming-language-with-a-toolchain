# Elya written in Elya: the road

**Status:** the road, decided (2026-11-14, on `main`, an interactive session);
each part gets a spec of its own. The maintainer, 2026-11-14: "Why are we still
using rust instead of our own language?" and, told what stands in the way,
"I want self-hosting to be the priority". Design choices are Claude's (rule 2).

## 0. Measured first

The compiler is Rust: the front end (`src/`, 17,230 lines: lexer, parser, name
resolution, inference, affine checks, Core, the evaluator) and the back end
(`crates/codegen/`, about 16,000 lines over LLVM through inkwell). What a
compiler written in Elya would need, and what Elya has:

| a compiler needs | Elya today |
|---|---|
| its source files | nothing reads a file or standard input (HANDOFF's old step 3) |
| its command line | nothing reads the arguments |
| to write its output | `io.println` only |
| to run the linker | nothing runs a program |
| text by byte | `string.length`, `string.byte_at`, `string.slice` (2026-10-30) |
| to build a large text | `<>` copies both sides: a string built by 80,000 appends of 10 bytes takes 1.44 s natively, 0.18 s at 20,000 (quadratic) |
| tables (names, types) | lists and trees of immutable data; no array, no map |
| data structures | ADTs, closures, effects: enough for a lexer, a parser and a checker |
| to be fast enough | native code at `default<O2>`; the collector paged |

So the language can already express a lexer: it reads bytes of a `String` and
builds a list of tokens. It cannot get the `String` from a file.

## 1. Decisions

**R1. Rust stays the bootstrap compiler** until an Elya compiler compiles
itself and every test agrees; then the Elya one is built by the Rust one, then
by itself, and the two must give the same output (the usual fixed point).

**R2. One piece at a time, each checked against the Rust one, byte for byte.**
The house's oldest check is that native code writes what the evaluator writes;
the self-hosted pieces get the same: each Elya piece and its Rust twin run on
every program the tests hold and their outputs compare equal. A piece lands
when it agrees everywhere, and is wired into `elya` only when it is also fast
enough (measured against its twin).

**R3. The parts, in order:**
1. **The world a compiler needs:** reading a file and standard input, the
   command line, writing a file, as builtins that perform `IO` with
   capabilities over them (`elya/files`, handlers in `elya/host`), recorded
   and replayed like `time.*`, on both back ends, and failing by name on a
   board that has none (spec 2026-11-14-elya-files-and-arguments). Running a
   program (`io.run`, `elya/process`) comes with part 5, which needs it.
2. **Text and tables in proportion:** a buffer that builds a string in linear
   time, and arrays, both `linear`: the affine guarantee (E0428, E0437) means
   no one else holds a value used once, so a "functional" update
   (`buffer.push(b, s)` answers the buffer) can be done in place, safely. Maps
   in Elya over arrays. The 80,000-append measurement is the test.
3. **The lexer in Elya** (`selfhost/lex.elya`, first written by ChatGPT to
   Claude's brief while part 1 is built; checked here): every `.elya` file in
   the repository and every test program, lexed by both, the same tokens and
   diagnostics.
4. **The parser, then name resolution, inference and the affine checks**, each
   the same way (the AST printed in one agreed form).
5. **Core and a back end that writes LLVM IR as text** and runs `clang` on it
   (no foreign-function interface needed), checked against the Rust back end by
   running both programs.
6. **The switch:** `elya` built by the Rust compiler from the Elya sources,
   then by itself.

**R4. The language keeps moving meanwhile.** A change to the language lands in
the Rust compiler first, as now; its Elya twin follows in the same commit once
it exists, so the comparison tests keep both honest.

## 2. What is not decided here

The shape of each part's builtins (part 1's spec), whether arrays hold any
type or words only (part 2's), the AST's printed form (part 4's), and when the
Rust front end is retired (after R1's fixed point, and only then).
