# Plan: a linear buffer (self-hosting, part 2a)

Spec: `docs/superpowers/specs/2026-11-14-elya-buffer-design.md` (D1-D6).
Base: `main` after the lexer in Elya lands (part 3, begun early).

## Steps

1. The front end: `Buffer` a builtin type name (no arguments, E0400), linear
   from the start of inference, not data (E0436 "this compares buffers", and a
   declared type holding one); `buffer.new/push/finish` typed, pure.
2. The evaluator: `Value::Buffer(Rc<RefCell<String>>)`; `buffer_builtin`; both
   machines; `literal::show` refuses a buffer (it is never an answer).
3. Native: `elya_buffer_new/push/finish` in `runtime.c` (the string tag, whose
   row traces nothing); `buffer_builtin` in both emitters, operands rooted.
4. `selfhost/lex.elya` writes its dump with a buffer instead of joining by
   halves; part 3's comparison must still agree.
5. Controls, each reverted and compared byte for byte:
   - K1, a native push that always grows: the native linearity test fails
     (800,000 appends took 374 s).
   - K2, an evaluator push that copies the text each time: it first passed
     the bound of ten at 4x the appends (it was 8.3x: a copy of a few hundred
     kilobytes is fast), so the test now takes 8x the appends with a bound of
     fourteen. Linear measures 8; K2 then fails at 49.
   - K3, `finish` not rooting the buffer: not caught, and it cannot be from
     Elya code: both emitters root every operand across the call, so the
     runtime's own root is a second one (kept for a caller in C). The first
     note here blamed the stress mode for never reusing a freed slot; that was
     wrong about this control, though the gap it named was real in another
     form (a write through a stale pointer went unseen) and is closed since
     (`gc_check_poison`, 2026-11-14).
   - K4, `Buffer` not linear: the E0428 cases check, so their test fails.
6. Review; the gate.

## Tests

- `tests/buffer.rs` (new, 3): the text pushed (empty, one push, across the
  first growth, non-ASCII, through a handler); used once and not data (E0428
  twice and in a closure, E0429 across a `multi` perform, E0436 for a buffer
  and for a type holding one, E0400, a `String` where a `Buffer` is wanted);
  linear time in the evaluator (200,000 appends within fourteen times
  25,000's).
- `crates/codegen/tests/native_buffer.rs` (new, 3): the evaluator's output
  natively, also with `ELYA_GC_STRESS=1`; a buffer as main's value stopping
  by name, the same words on both back ends (found while checking: natively
  it was an internal error, so the reflection table gained `KIND_BUFFER`);
  linear natively (800,000 appends under 2 s, within ten times 200,000's).
- `crates/codegen/tests/native_boards.rs`: the same program on both boards (a
  twelfth program in an existing test).

## Predictions

- Gate, configurations A and B: 1570 + 2 x 3 + 3 = **1579 passed, 174
  suites** (171 + 2 + 1). Configuration C: **8 passed, 2 suites** (unchanged:
  one more program inside an existing test).
- No existing expectation changes.
