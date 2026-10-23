# Parser error recovery inside a body

**Status:** in progress (2026-10-23, branch `auto/elya`, a scheduled run). HANDOFF
step 1. The design choices are Claude's under the maintainer's delegation (rule
2), recorded here.

## 0. Measured first (at 1a9003c, gate 1199 / 117)

`elya check` on seven one-mistake programs (each also has a genuine later error,
an unresolved name, so recovery is visible):

| program | mistake | diagnostics today | what is wrong |
|---|---|---|---|
| p1 | `let x = (a + b` (no `)`), then two more lines in `add` | E0100 `)`, **E0200 `add`**, E0200 `undefined_name` | `add` dropped: its caller is reported |
| p2 | `let = a + 1` | E0100, **E0200 `f`**, E0200 `oops` | the same |
| p3 | `match o { Some(+) -> 1 ...}` | E0100 pattern, **E0200 `get`**, E0200 `nope` | the same |
| p4 | a handler clause `E.e( -> resume(1)` | E0100, **E0200 `run`**, E0200 `missing` | the whole function dropped for one clause |
| p5 | the file ends in `pub fn g(` | **E0200 `g` only** | no parse error at all: the truncation is silent |
| p6 | the file ends in `fn h() -> Int { x +` | **none: `check` passes** | a truncated declaration is silently DROPPED |
| p7 | `let p = 1 +` then `let q = 2`, `p + q + zz` | E0100, **E0200 `a`**, E0200 `yy` | `zz`, a real error after the mistake in the same body, is never reported |

Causes:

- A failed expression or `let` fails its block (`?`), so the function fails and
  the module level skips to the next declaration keyword. The function is gone,
  so every call of it is a second error (E0200), and nothing after the mistake
  in its body is checked.
- `recover_to_decl` stops at any `fn`, a lambda's included, so it can resume in
  the middle of a body.
- Running out of tokens returns `None` without a diagnostic (`self.peek()?`), and
  a block, effect or type whose `}` is missing at the end of the file is
  accepted (`self.eat(&RBrace)` unchecked).

## 1. Decisions

- **D1. The end of the file inside a declaration is an error.** A declaration
  that fails without a new diagnostic gets E0100 "unexpected end of file"
  (labelled "the file ends here"); a block, effect or type whose closing `}` is
  missing is E0100 "expected `}`". p5 and p6 now fail `check`.
- **D2. Recovery to the next statement.** A failed statement in a block (an
  expression or a `let`) becomes a new AST node `Expr::Error`, and parsing goes
  on at the next statement: the next token, at the statement's own bracket
  depth, that starts a line; or the block's `}` (not consumed); or the end of
  the file. The depth is counted from the statement's first token, so a
  mistake inside an unclosed `(` or a `match`'s braces skips to its close. A
  line starting with `let` also ends the skip when no `{` was opened since the
  mistake (an unclosed `(` before a `let`, p1). A bad `let` whose name was read
  still binds it (to `Expr::Error`), so its uses are not reported. The
  innermost block recovers, so a mistake in a lambda or a clause body costs
  only its statement.
  - `Expr::Error` exists only in a module with a parse error, and `lib.rs`
    runs inference only on a module without diagnostics, so no later phase
    sees it: resolve skips it, inference types it `Ty::Error` (poison),
    evaluation and Core lowering treat it as an internal error.
  - *Rejected:* recovering only to the next declaration at column 0, keeping
    the function as a stub. It still hides p7's `zz`.
- **D3. A declaration whose header fails is still a known name.** When `fn`
  and its name were read, the declaration is kept as a stub (the parameters
  read so far, a body of `Expr::Error`), so callers are not reported (p5).
  Inference never runs on it (D2's reason).
- **D4. Module-level recovery stops at a `fn` only at the start of a line in
  column 0** (a top-level declaration), and at `pub`, `import`, `effect` or
  `type` anywhere as before: a lambda's `fn` no longer resumes the parse in
  the middle of a body.
- *Not in this step* (left in HANDOFF): a label after non-ASCII text (byte
  offsets where `ariadne` counts characters); E0209 across files labelling only
  the second declaration.

## 2. Evidence (planned)

`tests/parse_recovery.rs` (root crate, counted twice): each of p1-p7 gives exactly
the expected diagnostics (codes and messages, in order):

| program | expected |
|---|---|
| p1 | E0100 `expected `)``, E0200 `undefined_name` |
| p2 | E0100 `expected binding name`, E0200 `oops` |
| p3 | E0100 `expected a pattern`, E0200 `nope` |
| p4 | E0100 `expected parameter name`, E0200 `missing` |
| p5 | E0100 `unexpected end of file` |
| p6 | E0100 `unexpected end of file` (D1: the block's missing `}` is reached through the failed expression) |
| p7 | E0100 `expected an expression`, E0200 `zz`, E0200 `yy` |

plus a recovered lambda body and a valid program with a column-0 `fn` after a
mistake (D4). Negative controls: D2 off (statements fail their block again):
p1-p4 and p7 report the cascade E0200s; D4 off: a lambda `fn` after a mistake
gives a second E0100; D1 off: p6 checks clean.
