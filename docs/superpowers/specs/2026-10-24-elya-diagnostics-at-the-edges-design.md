# Diagnostics at the edges

**Status:** in progress (2026-10-24, branch `auto/elya`, a scheduled run).
HANDOFF step 1. The design choices are Claude's under the maintainer's
delegation (rule 2), recorded here.

## 0. Measured first (at 0635bb1, gate 1241 / 125, debug build, 8 MiB main stack)

Four edges the parser-recovery slice left (its spec §3), measured with
`elya check | run | build`:

| # | program | today | what is wrong |
|---|---|---|---|
| e1 | `let s = "héllo wörld"` then `1 + true` on the next line | E0200 at `d1.elya:3:9`, the label under `ue` and `}` | `ariadne` counts characters, the spans are bytes: every label after a non-ASCII character is shifted (by 2 here) |
| e2 | `type T { A }` in `b.elya`, `type T { B }` in the root | E0209, one label (the root's) | the first declaration is not shown, and it is in another file; E0202 (an operation declared twice) is the same |
| e3 | `fn main() -> Int { if true { 1 }` at the end of the file | E0100 "expected `else`" **and** E0100 "expected `}`", one span | one mistake, two reports (the recovery rule "the end of the file is reported once" covers `}` only) |
| e4 | 5000 nested `(`…`)` | `thread 'main' has overflowed its stack`, abort (134), `check` and `run` | no diagnostic at all |

e4 is wider than brackets. The depth at which `check` dies (bisected):

| shape | `check`/`run` die at | `build` dies at |
|---|---|---|
| `((…1…))` | ~1576 | > 600 |
| `1 + 1 + … + 1` (a chain: no parser recursion) | ~1097 | 256 < n < 400 |
| `{ { … 1 … } }` | ~985 | 256 < n < 400 |
| `fn() { fn() { … } }` | ~888 | 256 < n < 400 |
| `match 1 { _ -> match … }` | ~1097 | 256 < n < 400 |
| `if c { if c { … } else {0} } else {0}` | ~927 | 256 < n < 400 |
| nested patterns, `fn() -> fn() -> …` types, calls `i(i(…))` | under 5000, 5000, 1000 | |
| **a `main` of n `io.println` statements** (no nesting) | fine at 10 000 | **~700** |

The last row is the important one: `elya build` of an ordinary 700-line
`main` overflows. A block of statements is a chain of `let`s in Core and every
Core pass (`mono::rewrite_let` first, by `gdb`) recurses down it. The
expression rows are adversarial or generated code; this one is not.

A test of the suite already relies on a 258-term chain (`specialize`'s
`the_clone_budget_is_refused_by_name`, run on a 256 MiB thread for that reason).

## 1. Decisions

- **D1. The renderer counts bytes.** `diag::render` configures `ariadne` with
  `IndexType::Byte`; every span in the compiler is a byte offset. e1's label is
  under `true` at `3:7`.
  - *Rejected:* converting spans to character offsets before rendering: a
    second index for the same thing, and `ariadne` already does the lookup.

- **D2. A duplicate names both declarations.** E0209 and E0202 keep their
  first label (the second declaration, "a second declaration of this name") and
  gain one at the first, "first declared here", in its own file. Their
  messages and codes are unchanged. E0209's help says where a name is unique:
  a function within its file (functions are qualified across files), a type,
  constructor or effect across the program, imported files included (modules
  part 1 D3).
  - *Rejected:* labelling only the first: the second is the one to rename.

- **D3. One parser diagnostic per position.** A parser error whose label is at
  exactly the span of the parser's previous error is not reported. e3 keeps
  "expected `else`". This generalises the end-of-file rule (parser recovery
  D1): a second error at the same token is always a consequence of the first.
  - *Rejected:* special-casing `if`: the same cascade exists for any construct
    that needs one more token at the end of the file.

- **D4. A nesting limit, E0102.** An expression may be nested at most
  **1024 levels**. A level is an expression node on the path from the
  statement's root to a leaf, a parenthesised expression counting as one
  (`(((1)))` is 4 levels; a chain `1 + 1 + 1` is 3; `{ 1 }` is 2). Patterns and
  type annotations share the parser's recursion count with expressions. The
  first construct past the limit is E0102 "expression nested too deeply"
  (labelled at it, with a help: bind an inner part with `let` or move it into
  a function); the parser then reads nothing more of that declaration (no
  follow-on errors) and resumes at the next declaration.
  - The height is counted while parsing, bottom up: each `expr` call reports
    the height of what it returned, an atom is one more than the expressions
    inside it, a chain step (an operator or a call) one more than both sides.
    So a chain (built by a loop, not by recursion) is limited as much as
    nesting is, and no later pass ever sees a tree taller than the limit.
  - The limit is a property of the language, the same on every build and
    host, rather than "whatever the stack allows".
  - Why 1024: C requires 63 levels of parenthesised nesting, Python's parser
    stops at 200 brackets; 1024 is deeper than any of these and above the
    suite's own 258-term chain, while far below what every pass survives on
    the stack of D5 (debug `build` needs ~25 KB a level; 1 GiB is ~40 000).
  - *Rejected:* an explicit stack in the parser (or `stacker`): the parser
    would survive and every later pass (resolve, inference, Core, the codegen
    passes) would overflow instead; making all of them iterative is a project
    of its own.
  - *Rejected:* a limit only on brackets: the chain row overflows without any.

- **D5. The CLI runs the compiler on its own 1 GiB stack.** `elya check | run
  | build` run their work on a thread with a 1 GiB stack (reserved, not
  committed), the size native programs already run on (`elya_start`). This
  makes D4's limit reachable as a diagnostic on every build, and lifts the
  statement-chain overflow (the table's last row) from ~700 statements to
  about 90 000 in a debug build.
  - Sequence length is NOT limited: a block of a million statements is
    legitimate code; its bound is this stack. Making the Core passes iterative
    over `let` chains (so no such bound exists) is a new HANDOFF step.
  - The library entry points (`check_source`, ...) run on their caller's stack,
    as before: a caller (a test) that compiles deep programs gives them a big
    thread, as `crosscheck` does.
  - A panic on the thread is re-raised on the main thread (`resume_unwind`),
    so the exit status of a compiler bug is unchanged (101).

## 2. Evidence (planned)

`tests/diagnostics_edges.rs` (root crate, counted twice): e1 at `3:7`; e2 for
types across files and for an operation, two labels, the first in `b.elya`;
e3 one diagnostic; each deep shape (brackets, a chain, blocks, lambdas,
`match`, `if`, patterns, types, calls) exactly one E0102 on a 256 MiB thread;
the edge (1024 levels run, 1025 is E0102, for a chain and for brackets); and
recovery (a later declaration's unresolved name is still reported, nothing
else). `crates/cli/tests/run_cli.rs` (twice): `elya check` on 5000 brackets is
E0102 and exit 1; `elya run` on 1024 nested blocks prints its value.
`crates/cli/tests/build_cli.rs` (codegen, once): `elya build` of a 2000-line
`main` runs.

Negative controls: D1 off (the default `Char` index) puts e1 back at `3:9`; D3
off brings e3's second report back; D4's height off (recursion counted only)
lets the chain through the parser (E0102 missing, a crash or a value); D5 off
(no thread) aborts the CLI tests.
