# A repeatable differential sweep

**Status:** done (2026-10-16). HANDOFF step 1. Design choices are Claude's under
the maintainer's delegation (rule 2), recorded here.

## 0. Measured first

The 2026-10-15 sweep (about 700 programs) used review corpora that lived in a session's
scratch space and are gone; no script was kept. Two facts make a faithful comparison
impossible with today's CLI:

| Program | `elya run` | native |
|---|---|---|
| r1 `pub fn main() -> Int { io.println("before")  1 / z(3) }` (`z(x) = x - x`) | stdout **empty**, the diagnostic, rc 1 | `before`, `elya: division by zero`, rc 1 |
| r2 `pub fn main() -> Int { 42 }` | prints nothing (main's value is discarded) | `42` |

So `elya run` drops what a program printed before a runtime error (r1: the evaluator's
output is buffered and the error path returns only the diagnostic), and it never shows
main's value, which is all that most corpus programs produce (r2). The tests compare
values through `eval::run_module_value`; a script cannot.

## 1. Decisions

- **D1. `elya run` keeps its output on a runtime error.** What the program printed goes
  to stdout, then the diagnostic to stderr, exit 1 -- as a native binary does. Losing
  output that a program really produced is a bug, not a design. (`--record`/`--replay`
  already keep it but print it to stderr with the diagnostic; left as is, recorded.)
- **D2. `elya run --value <file>`** prints, after the program's output, main's value the
  way the native shim does: an `Int` in decimal, `Unit` as `0`, each on its own line.
  Any other main type is an error by name (exit 1), as natively (`require_int`). With
  it, an evaluator run and a native run of the same program can be compared byte for
  byte. Options rejected: rewriting each program's `main` in the script (changes the
  program under test), or a Rust test that walks the corpora (the sweep must be cheap to
  re-run at any commit without touching the gate).
- **D3. `scripts/sweep.py`.** Python 3, standard library only, a developer tool (not part
  of the gate; the gate scripts stay sh/ps1). It collects programs -- `examples/`,
  `tests/ui/`, and every Rust string literal in `tests/` and `crates/*/tests/` that
  declares `fn main(` (unescaped, with `{{ }}` collapsed for `format!` templates) --
  keeps those `elya check` accepts, runs each through `elya run --value` and `elya build`
  plus the binary (timeouts on both), and classifies: `same`, `both-fail` (same stdout,
  both non-zero), `refused` (a named native refusal, ranked by reason), `differ`,
  `native-crash` (a signal), `eval-timeout`, `native-timeout`, `build-error`. The report
  (counts, the ranked refusals, every `differ`/`crash` with its program) goes to the
  output directory.

## 2. Evidence

- `crates/cli/tests/run_cli.rs`: r1 keeps `before` on stdout (red: empty); `--value`
  prints an Int main and a Unit main as the native shim does, refuses a Bool main by
  name, and is a usage error in the wrong place (red: usage error today).
- The sweep at the tip: counts and the ranked refusals, recorded here (§3), and the
  top-ranked gap becomes the next HANDOFF step.

## 3. The sweep at the tip (2026-10-16, after 9bb6f5d)

`python3 scripts/sweep.py --jobs 2 --timeout 60`: 672 programs collected, of which 263
are fragments or deliberately ill-typed programs (`not-checked`), leaving 409:

| result | count |
|---|---|
| same (byte-identical stdout, both exit 0) | 382 |
| both-fail (same stdout, same status, the native message in the evaluator's) | 5 |
| both-fail-differently (same stdout, but not provably the same failure) | 4 |
| differ-timing (reads the real clock) | 2 |
| build-error | 11 |
| refused (named) | 4 |
| eval-timeout | 1 |
| differ / native-crash / native-timeout / any panic | 0 |

The four `both-fail-differently` are one known difference: a second resume of a one-shot
continuation is "continuation resumed more than once" in the evaluator and "a one-shot
continuation was resumed twice" natively (open since 2026-10-10).

The first run also had **one `build-panic` class (2 programs)**: `elya build` asserted
that inference returned no diagnostics at all, so a program with only a warning (E0426)
panicked the debug CLI. Fixed red first (`build_cli.rs::a_program_with_only_warnings_builds`).
Two timing programs differed on elapsed milliseconds (the script now classes a program
that reads `time.now_ms` as `differ-timing`, noise).

Ranked native gaps:

1. **`with any` (9 programs: example 09, the `any_handler` corpus, one Core test)** --
   refused by Core lowering. The next HANDOFF step.
2. A constructor of a parametric type used as a function value (`map(xs, Some)`):
   "parametric ADT" (1).
3. An effect-polymorphic closure reached through a `let` alias (`let mk2 = mk`): D16's
   refusal (1).
4. `Float` in Core (1); a user effect named `IO` (1, Core refuses "effect IO"); a `Bool`
   main (2, refused by design: the shim prints an `Int`).

The eval-timeout is unbounded recursion (`down(0)`): natively a named stack overflow,
in the evaluator a heap that grows until the timeout -- by design.

Limits of the corpus: programs assembled at run time from constants (`format!("{DECLS}...")`
with named arguments) are collected as fragments and fail `check`; the review corpora of
the 2026-10-15 sweep are gone. The script is the record now: re-run it at any commit.

## 4. The independent review

No bug in the Rust changes (`--value` byte-identical to native binaries on Unit, negative,
`i64::MIN`, multi-print, warning-only and failing programs; argument handling; plain
`run` unchanged on success). Fixed in `sweep.py`: a panic or timeout of `elya check` /
`elya run` was hidden as `not-checked` / `both-fail` (now `front-end-panic`,
`eval-panic`, `check-timeout`, all failing the sweep, as does `native-timeout`);
`both-fail` now needs the same status and the native message in the evaluator's
(else `both-fail-differently`, listed); `differ-timing` ignores `time.now_ms` in
comments; char literals and comments no longer derail the literal scan; one broken
program no longer kills the sweep. Not fixed, recorded: `--record`/`--replay` still print
a failing run's output to stderr with the diagnostic (HANDOFF "also open").
