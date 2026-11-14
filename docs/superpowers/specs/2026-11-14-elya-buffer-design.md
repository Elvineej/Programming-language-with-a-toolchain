# Self-hosting, part 2a: a linear buffer, text built in linear time

**Status:** built (2026-11-14, on `main`, an interactive session; the plan
`docs/superpowers/plans/2026-11-14-elya-buffer.md`). HANDOFF
step 1, part 2 (spec 2026-11-14-elya-self-hosting R3), its first half; arrays
are part 2b. Design choices are Claude's (rule 2); ChatGPT surveyed Clean,
Lean 4, Koka (Perceus, FBIP), Rust, linear Haskell and Roc for it (checked
against the sources it cites where they decide anything below) and ranked
"static affine, consuming updates, no reference counts" first, as this does.

## 0. Measured first

- Building a string by appends is quadratic: `<>` copies both sides. The same
  program, 10 bytes appended `n` times (`release` build, this container):

  | n | native | evaluator |
  |---|---|---|
  | 20,000 | 0.107 s | 0.168 s |
  | 80,000 | 1.667 s | 2.468 s |

  Four times the appends, fifteen times the time. A compiler's output (part 5
  writes LLVM IR as text) is megabytes of such appends.
- The lexer in Elya (`selfhost/lex.elya`, part 3, begun early) avoids it by
  joining a list of pieces by halves: `O(m log k)` for `m` bytes in `k`
  pieces, and every program that writes text would have to do the same.
- Elya already has what makes an in-place update safe: a `linear type`'s value
  is used at most once (E0428), across functions, closures and generic code,
  and is never held across a point that may resume twice (E0429) (spec
  2026-11-06-elya-affine-across-functions). A function given the only
  reference to a value may change it: no one can see the old one.
- The native collector does not move objects and has no write barrier (it
  stops the world and marks everything); a byte buffer holds no pointers, so
  it needs neither.

## 1. Decisions

**D1. `Buffer`, a builtin linear type, opaque:** no constructors, no
patterns; `Buffer` names it in an annotation. It is not duplicable (E0428,
E0429 apply as to any `linear type`), not comparable (`==` refuses it, as a
function), and `any` cannot carry it (`op.encode` refuses it by name).

**D2. Three builtins, pure (no effect):**

| builtin | type | what |
|---|---|---|
| `buffer.new()` | `fn() -> Buffer` | an empty buffer |
| `buffer.push(b, s)` | `fn(Buffer, String) -> Buffer` | `b` with `s` after it |
| `buffer.finish(b)` | `fn(Buffer) -> String` | the text |

The update consumes the buffer and answers its successor, as a function on an
immutable value would: the program reads as if `push` made a new buffer. A
`push` is amortised `O(|s|)`; a `finish` is `O(length)` once. There is no
`buffer.length` or reading back: a buffer is for building text, and a
program that needs to read it finishes it.

**D3. In place, by representation, not by a check.** Natively a buffer is one
heap object, `[tag][length][capacity][bytes]`, holding no pointers. `push`
copies `s` in when it fits and answers the same object; when it does not, it
allocates one of twice the capacity (at least what is needed), copies, and
answers the new one (the old is garbage). `finish` allocates the string and
copies once. In the evaluator a buffer is `Value::Buffer(Rc<RefCell<Vec<u8>>>)`
and `push` appends through the `RefCell` whatever the count: the machine's
own copies of the `Rc` (an environment keeps a binding after its last use)
are not aliases a program can use, because the affine guarantee forbids the
use. No reference count is read; `Rc::make_mut` would copy on every push
there.

**D4. What the guarantee does not cover is refused.** A `Buffer` captured by a
closure that may run twice, used in an operation clause, held across a
`multi` point: E0428 and E0429 already. A data value or a tuple holding a
`Buffer` is not duplicable either: the checker already counts a type's
fields (`decl_not_dup`) and a tuple's parts, so the rules reach it. A
`Buffer` in a trace (`time.*`, `io.*` answers are
never buffers) and in `any`: refused by type.

**D5. The test is the measurement.** The 80,000 appends with a buffer, both
back ends, within ten times the 20,000's time (linear: four times, with
room); the lexer's join by halves replaced by a buffer (part 3's comparison
still agreeing); a buffer under `ELYA_GC_STRESS` (a collection at every
allocation, the growth path included).

**D6. Not here:** arrays and maps (part 2b: a `linear` `Array(a)` with the
same consuming updates, and its reading, which is the hard question there);
a buffer of bytes that need not be UTF-8 (every `push` adds a `String`,
which is UTF-8, so the text is).

## 2. Tests (planned)

Both back ends, the same bytes: empty, one push, pushes across the growth
boundary, non-ASCII text; the 80,000-append timing ratio; E0428 for a buffer
used twice, E0429 for one held across a `multi` point, `==` and `op.encode`
refused; the stress run.

## 3. Measured after

The 80,000 appends: 8 ms natively (1.667 s with `<>`), 0.77 s in a debug
evaluator (2.47 s release with `<>`). The lexer in Elya, its output now in two
buffers: 700 KB lexed in 0.66 s natively (1.02 s joining by halves), the same
bytes; part 3's comparison with `src/lex.rs` still agrees everywhere.
