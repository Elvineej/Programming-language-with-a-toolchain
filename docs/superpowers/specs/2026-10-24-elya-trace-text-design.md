# Strings to read a trace: byte strings, one line format, `replay.parse`

**Status:** in progress (2026-10-24, branch `auto/elya`, a scheduled run; continued
2026-10-30 by an interactive session, which revised D2's trimming, below).
HANDOFF step 1, ROADMAP priority 3. The design choices are Claude's under the
maintainer's delegation (rule 2), recorded here.

## 0. Measured first (at bac4834, gate 1284 / 132)

- An Elya program cannot look inside a `String`: the only operations are `<>`,
  `==`/`!=` and the `to_string`s. `string.length(s)` is E0201 "unknown builtin
  `string.length`".
- Natively a string is one heap block `[tag, length, bytes..., NUL]`
  (`elya_str_lit`): its byte length and any byte are one load each; a
  character index would need a walk from the start. The evaluator holds a Rust
  `String` (UTF-8, byte-indexed the same way).
- `elya/replay`'s `text` writes the host trace's lines, and a `String` answer
  or argument with ` = ` in it is written as is, inside its quotes:

  ```
  elya trace 1
  Ask.ask("x = y") = "p = \"q\"\né"
  ```

  The host's reader (`replay_source_with`) splits each line at its LAST ` = `,
  which cuts that line inside the answer (`Ask.ask("x = y") = "p` / `\"q\"\né"`).
  Splitting at the first ` = ` would cut it inside the question instead. Today
  the host records only `time.*` (Int and Unit answers), so no host trace has
  such a line yet; a recording made by `elya/replay` does.
- A question is `op.show`'s text (`Effect.op(literal, ...)`) and an answer is
  `op.encode`'s (a literal). Outside a string literal neither ever contains
  ` = `, and inside one every `"` and `\` is escaped and a newline is `\n`; so
  the first ` = ` outside quotes is always the separator, and a line never
  holds a raw newline.
- The host reader ignores a leading byte-order mark, trims each line with
  Rust's `str::trim` (Unicode whitespace), skips blank lines, and numbers lines
  from 1 (the header).
- No program can read text from outside (a file, stdin): the only way to give
  `replay.parse` a trace is a literal or a string built in the program.

## 1. Decisions

- **D1. Strings are indexed by byte, three builtins in a new builtin module
  `string`.**
  - `string.length(s: String) -> Int`: the number of bytes.
  - `string.byte_at(s: String, i: Int) -> Int`: the byte at `i`, `0..=255`.
    Outside `0..length` it fails by name:
    `string.byte_at: index 7 is outside a string of 3 bytes`.
  - `string.slice(s: String, from: Int, to: Int) -> String`: the bytes
    `from..to`, a fresh string. It fails by name unless `0 <= from <= to <=
    length` (`string.slice: 2..9 is not a range in a string of 3 bytes`), and
    when either end falls inside a UTF-8 character (`string.slice: 1..2 splits
    a UTF-8 character`): a `String` is always valid UTF-8, as both back ends
    assume when they print it.
  - All three are pure: a failure is not an effect, as with checked `Int`
    arithmetic and `float.truncate` (part 2 D4).
  - `string` joins the builtin modules (`elya/string` imports it like
    `elya/int`; a user import named `string` is E0604, as `int` is).
  - *Rejected: characters (code points).* O(n) per index natively, and still
    not what a reader of text sees (graphemes). Bytes are what both back ends
    hold, and every format Elya reads (traces, literals) is ASCII where it
    matters.
  - *Rejected: an `Option` answer instead of a failure.* No builtin can name a
    user type, and a prelude type is a bigger decision than this slice;
    failing by name is the existing rule for a partial builtin.
  - *Rejected: `find`, `split`, `trim` as builtins.* They are ordinary Elya
    over these three (`elya/replay` writes the ones it needs). The builtins are
    the minimum that cannot be written in Elya.
- **D2. One line format, read the same way by the host and by Elya.** A trace
  is the text `elya trace 1`, then one `question = answer` line per answer.
  Reading it:
  - lines end at `\n`; a leading byte-order mark (EF BB BF) is skipped;
  - each line is trimmed of Rust's `str::trim` whitespace (Unicode `White_Space`,
    as both host readers trim today), and a blank line is skipped;
  - the first line must be `elya trace 1`;
  - an entry splits at the first ` = ` outside a string literal, where a
    string literal runs from a `"` to the next unescaped `"` (`\` escapes the
    byte after it); the question and the answer are trimmed;
  - errors, by line number (from 1): `expected \`question = answer\`` (no
    separator) and `a string is not closed` (the line ends inside quotes
    before any separator).
  - The host readers (the evaluator's and the native runtime's) move to this
    split, from the last ` = `. Every trace either accepted before means the same
    now, since the host's own questions and answers hold no quotes and no ` = `.
  - **Revised 2026-10-30:** the first draft also moved the trim to ASCII
    whitespace. That would change what a host reader accepts: a line padded with
    U+00A0 or U+3000 was trimmed, and `native_trace.rs`'s review test (M2) pins
    that both readers trim it. So the host keeps `str::trim`, and `replay.parse`
    trims the same set: 25 code points, one to three bytes each in UTF-8. Changing
    the host would have meant a changed test and a format change; trimming the
    same set in Elya is a table. *Rejected:* a new quoting layer (`"question" = "answer"`) or a
    version 2 header: the literals are already self-delimiting, so the first
    unquoted separator is unambiguous and the format stays the one already
    written.
- **D3. `replay.parse(text: String) -> ParsedTrace`** in `elya/replay`:
  `type ParsedTrace { TraceParsed(Trace), TraceUnreadable(Int, String) }` (the
  line and the reason, in D2's words; the header's reason is `the first line
  must be \`elya trace 1\``). `parse(text(t))` is `TraceParsed(t)` for every
  trace `t` a recording makes. Written in Elya over D1, tail-recursive (a trace
  of any length parses in constant stack).
  - Names prefixed as in std-replay D2 (`ParsedTrace`, `TraceParsed`,
    `TraceUnreadable`).
- **D4. Out of scope, recorded.** Reading a file or stdin (an input effect the
  host must also record) is HANDOFF's new step: until then a program parses a
  trace it holds as a string. The std-replay review's other two limitations
  (`record` under a multi-shot handler; decoding an answer typed by the
  effect's own parameter) stay under "Also open": both are `with any`
  behaviour, not this format's.

## 2. Evidence (planned; the 2026-10-30 additions marked)

- `tests/string_bytes.rs` (root, counted twice): the three builtins on ASCII
  and UTF-8 text, each failure by name, the type errors, purity (usable in a
  pure function), `import elya/string`, a user import named `string`.
- `tests/std_replay.rs`: `parse(text(t))` round trips the game, the shop and a
  string answer with ` = `, `"` and `\n` in it; the replay of a parsed trace
  replays; each `TraceUnreadable` (header, no separator, an unclosed string)
  with its line; the host's lenient trace (BOM, `\r\n`, blank lines, spaces)
  parses the same in Elya.
- `tests/trace.rs`: the host reader splits at the first unquoted ` = ` (an
  answer `"a = b"` to `time.now_ms()` is refused as not an `Int`, not as a
  question `time.now_ms() = "a`).
- `crates/codegen/tests/native_string_bytes.rs`: every program above natively,
  the same output and the same failure in the same words. Added 2026-10-30: the
  CPS emitter (the builtins in a function that performs), and slices through many
  collections.
- Added 2026-10-30: `native_trace.rs`, the native host reader splitting at the
  first unquoted ` = ` with the evaluator's words.
- Negative controls: K1 `slice` without the boundary check (native); K2 the
  Elya splitter ignoring quotes; K3 `byte_at`'s range check off (native).

## 3. Measured (2026-10-30)

Each new test was red first, as the plan predicted:
- `tests/string_bytes.rs` (6): E0201 "unknown builtin `string.length`", and E0600
  "no module `elya/string`".
- `tests/std_replay.rs` (+3): E0603 "module `elya/replay` has no function `parse`".
  The plan said E0201; the module system answers first.
- `tests/trace.rs` (+1): "replay diverged at entry 1: ... the trace holds
  `time.now_ms() = "a`" (the last ` = ` had split inside the answer).
- `native_string_bytes.rs` (3): the front end's E0201 and E0603.
- `native_trace.rs` (+1): the native reader's same divergence.

Found while writing `parse`: Elya has no `else if`. An `else` branch must be a block,
so `parse` nests `else { if .. }`. Recorded in HANDOFF's open list.

Controls (each reverted, `cmp` after):
- **K1**, `elya_str_slice` without its UTF-8 check: natively 1 where the evaluator
  fails ("1..2 splits a UTF-8 character").
- **K2**, `parse`'s separator ignoring quotes: two `std_replay.rs` tests fail.
- **K3**, `elya_str_byte_at` without its range check: natively 0, a padding byte.

The collection row slices a 3 000-byte string at every offset: 4.5 MB of slices and
17 collections natively, 498929 on both sides.

## 4. Review (2026-10-30)

An independent review ran:
- 10 programs over every valid byte index and slice of strings with 1- to 4-byte
  characters, and 250 failure programs (`byte_at` at -1, at the length, past it, at
  `i64::MIN` and `i64::MAX`; every invalid or splitting range). These ran on the
  evaluator, natively, and natively under a stress runtime that collects at every
  allocation;
- CPS, `with multi` and `with any` programs with slices live across resumes;
- the two host readers against each other on 2 500 random and 3 601 exhaustive
  texts;
- 420 random recorded programs round-tripped through `text` and `parse` (120 also
  natively).

Agreement held everywhere except two findings, both fixed before the commit:
- **F1 (high: introduced here).** `parse` split the TRIMMED line, the host readers
  the whole one. A ` = ` touching a trimmed edge read differently: `A.a() = ` then
  the end of the line is an empty answer to the host, "expected `question =
  answer`" to `parse`; ` =  = 1` gave other halves.
  - `parse` now searches the whole line and trims the halves.
  - `read_trace` (new, `src/lib.rs`) is the host's reader as a function, and
    `replay_source_with` uses it.
  - `parse_reads_every_short_trace_as_the_host_does` compares the two on 1 041
    texts: every line of up to three symbols over {space, `=`, `"`, `\`, `a`,
    U+3000}, in three placements. Red first: 21 differed.
- **F2 (medium: older than the slice, exposed by it).** The evaluator's
  `Value::Str` held a `String`, cloned at every variable read and argument pass.
  So a byte loop over a string copied it at every step: `parse` of a 20 384-line
  trace took 89 s in the debug evaluator (0.06 s natively).
  - `Value::Str` is an `Rc<str>` now. 2 500, 5 000, 10 000 and 20 000 lines take
    3.4, 6.9, 13.4 and 27 s: linear.
  - No test pins the time: a timing bound would be flaky, and nothing in the
    evaluator counts a copy.

Also confirmed: `parse` runs in constant stack on both sides (natively with
`ELYA_STACK_MIB=1` on 262 144 entries). The tree-walking oracle overflows on a deep
loop, as it always has; it is a test-only evaluator without tail calls.
