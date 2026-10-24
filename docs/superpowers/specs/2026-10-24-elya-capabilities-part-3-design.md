# Capabilities, part 3: attenuation in the standard library

**Status:** in progress (2026-10-24, branch `auto/elya`, a scheduled run).
HANDOFF step 1, ROADMAP priority 2's last part. The design choices are Claude's
under the maintainer's delegation (rule 2), recorded here.

## 0. Measured first (at dc22437, gate 1264 / 127)

What a program needs to give a dependency LESS than a builtin:

| program | dependency | today |
|---|---|---|
| c1 | `dep` prints with `io.println` | needs `import dep / {IO}`: all of the host (files, clock, output), nothing less |
| c2 | `dep` reads `time.now_ms()` to stamp a report | needs `/ {IO}`; no way to give it a fixed clock |
| c3 | `dep` performs `line(text)` of an effect `Out` declared in a user module `out.elya`; the root grants `/ {Out}` and handles it with its own `prefix` and `print` handlers | works, evaluator and native: `[chatty] working on 21`, 42 |
| c4 | two dependencies by different authors that both write lines | each declares its own effect (names are program-unique), so the root handles two vocabularies for one thing |

So attenuation already works mechanically (a handler over a user effect, c3):
what is missing is a shared vocabulary that dependencies can be written
against, and the handlers that attenuate it, so that every program does not
re-declare `Out` and re-write `prefix`. `IO` itself cannot be attenuated: no
handler can handle it (a builtin effect), by design.

Builtins need no import in a module (part 1). A module that uses `io.*` is
already refused at its importer unless granted `IO` (part 2, by its inferred
interface); eight programs in the suite's module tests use `io.*` in a module
without importing `elya/io`.

## 1. Decisions

- **D1. Two vocabulary modules, written in Elya.** `elya/console` declares
  `effect Out { fn out_line(text: String) -> Unit }`; `elya/clock` declares
  `effect Clock { fn clock_now_ms() -> Int }`. A dependency that writes lines or
  reads the time performs these, and is granted `/ {Out}` or `/ {Clock}`, never
  `IO`. Operation names carry their effect's name (`out_line`, not `line`):
  operation names are unique across a program, as `elya/replay`'s type names
  are prefixed for the same reason.
- **D2. Attenuating handlers beside them.** Each relays its body's operations
  outward changed, or answers them itself:
  - `console.prefixed(p, body)`, `console.limited(n, body)` (at most `n`
    lines get out), `console.counted(body)` (`Counted(value, lines)`),
    `console.captured(body)` (`Captured(value, text)`: nothing gets out),
    `console.silenced(body)`;
  - `clock.fixed(t, body)`, `clock.stepping(start, step, body)` (a
    deterministic clock), `clock.shifted(delta, body)`.
- **D3. The host's handlers are a module of their own, `elya/host`.**
  `host.console(body)` prints `body`'s lines; `host.clock(body)` answers from
  `time.now_ms()`. Its interface performs `IO`, so `import elya/host / {IO}`
  is required (part 2 D2): only a file that holds `IO` can turn `Out` or
  `Clock` into it, and `elya/console` and `elya/clock` need no `IO` at all.
  - *Rejected:* `console.print` inside `elya/console`. Every dependency that
    only writes lines would import a module whose interface performs `IO`, and
    would have to write `/ {IO}` on it to compile: the grant would claim
    authority the dependency does not want.
- **D4. Grants apply to the standard Elya modules unchanged.** A relaying
  handler (`prefixed`, `limited`, `counted`, `shifted`) performs its effect at
  a positive position, so `import elya/console / {Out}` is required as for
  any module performing `Out` (part 2 D1). The list says what the importing
  file lets the library do; the importer still decides where `Out` goes.
  - *Rejected:* exempting the standard Elya modules. A trusted-library
    exception would make the rule non-uniform, and would let `elya/host` hand
    out `IO` with no list.
- **D5. Builtins stay in scope without an import, in modules too.** The
  stricter-looking rule (a module must `import elya/io` to call `io.*`) adds no
  authority: an import in the module's own file is its claim, while the grant
  that bounds it is the importer's list, already checked against the module's
  inferred interface (part 2). It would change eight existing tests'
  expectations for no soundness gained. **For the maintainer's review.**

## 2. Evidence (planned)

`tests/std_capabilities.rs` (root crate, counted twice):
1. `a_dependency_gets_lines_and_a_clock_not_io`: `dep` (performs `out_line` and
   `clock_now_ms`) granted `/ {Out, Clock}`, run under `host.console`,
   `console.prefixed`, `clock.fixed`: its exact output and value.
2. `the_attenuating_handlers`: `limited`, `stepping`, `counted`, `captured`,
   `silenced`, `shifted` (over `fixed`): one program, its exact output and value.
3. `the_host_needs_io_granted`: `import elya/host` is E0606 for `IO`.
4. `a_dependency_cannot_reach_io_through_them`: a dependency granted
   `/ {Out}` that calls `io.println`, and one that imports `elya/host / {IO}`
   and calls `host.console` in a `pub` function, are both E0606 for `IO` at
   the root.
5. `a_relaying_handler_needs_its_effect_granted`: `import elya/console` with
   no list, using `console.prefixed`, is E0606 for `Out` (D4).

`crates/codegen/tests/native_std_capabilities.rs` (once): tests 1 and 2's
programs natively, their output equal to the evaluator's.

Negative control: `console.limited` without its count (every line out):
test 2 fails; `elya/host` declaring `Out` itself instead of importing it: E0209
where both are imported.
