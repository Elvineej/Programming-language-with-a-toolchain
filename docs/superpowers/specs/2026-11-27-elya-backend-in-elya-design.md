# The back end in Elya (self-hosting part 5)

2026-11-27. Status: decided, not built. Part of R3 item 5 of
`2026-11-14-elya-self-hosting-design.md`: "Core and a back end that writes LLVM
IR as text and runs `clang` on it, checked against the Rust back end by
running both programs". Design choices are Claude's (rule 2).

## 0. Measured first

The Rust back end (`crates/codegen`, about 18,800 lines over inkwell) compiles
the whole language. Most of its size goes to things the compiler's own
sources never do:

| the Rust back end has | lines | used by `selfhost/*.elya` (8,006 lines) |
|---|---|---|
| selective CPS for effects, handlers, resumes, multi-shot copies (`cps.rs`, `cps_emit.rs`, much of `lib.rs`) | ~6,000 | no `handle` at all |
| `with any`, operation values, reflection (`any.rs`, `reflect.rs`) | ~1,500 | no |
| row specialization and upcast adapters (`specialize.rs`) | 1,500 | no user effect, so no open row reaches code |
| boards: Cortex-M3, RISC-V, `rv_tail`, `arm_tail`, `boards.rs` | ~1,500 | no (the compiler runs hosted) |
| `Float`, `time.*`, traces | some | no |
| type monomorphization (`mono.rs`) | 1,479 | yes: `List(a)`, `Maybe(a)`, `Sp(a)` everywhere |
| pattern compilation (`patterns.rs`) | 571 | yes: nested patterns |
| closures (`closure.rs`, `anf.rs`) | ~1,700 | yes, a few (18 lambdas) |
| argument records past three parameters (`spill.rs`) | — | yes: functions of up to 9 parameters |
| direct-style emission, the shadow stack, tail calls | — | yes |

What the sources call: `int.to_string`, `int.wrapping_sub`/`_mul`,
`string.length`/`byte_at`/`slice`, `<>`, `==` on strings and ints,
`array.new`/`get`/`set`/`push`/`length`, `buffer.new`/`push`/`finish`,
`io.read_file`/`arg`/`arg_count`/`println`, and imports between the five
files.

The runtime (`runtime.c`, `gc.c`, `platform_hosted.c`, 3,800 lines of C) is
what both back ends call; the Elya one links the same files unchanged.

## 1. Decisions

**D1. The subset first: what a compiler is written in.** The back end in Elya
compiles *Elya-S*, the language the self-hosted sources use: `Int`, `Bool`,
`String`, `Unit`; ADTs, parametric ones included; top-level functions, mutual
recursion, lambdas with captures, functions as values; `let`, `if`, `match`
with nested patterns and literals; the builtins above and the rest of
`int.*`/`string.*` of the same shape; `Array` and `Buffer` with the affine
guarantee already checked by the front end; `IO` through builtins only;
imports (the module the front end merges). Anything else (a `handle`, a
user effect's operation, `resume`, `Float`, `time.*`, `any.*`, `op.*`) is
refused by name, before any IR is written: `elya: backend: unsupported
<construct>` on standard error, exit 1. The fixed point (part 6) needs only
Elya-S, as long as the compiler's sources stay in it; widening the subset is
later work and each widening gets its own stage (D6).

*Why not port the Rust back end.* Its passes exist for features Elya-S does
not have; porting 18,800 lines to compile what 8,000 lines use would be most
of the work for none of the goal. The check is behaviour (D2), not IR, so the
two back ends need not share a design: only what a program prints.

**D2. Checked by running both programs.** For every program of the corpus
that is in Elya-S (a classifier, as for the checker's stages), and for the
self-hosted sources run on their own corpus: the program built by the Rust
back end and the program built from the IR the Elya back end writes are run
on the same input; standard output, standard error and the exit code must be
equal. A runtime failure (overflow, division by zero, an index out of range,
a failed match) is the runtime's words on both sides, since both call the
same runtime. Out of memory and stack depth are the only behaviours allowed to
differ, and only past what the corpus reaches.

**D3. The pipeline, each piece in Elya, each checked on its own.**

1. **Core** (`selfhost/core.elya`): the typed AST lowered as `src/core.rs`
   lowers it, from the types `selfhost/check.elya` infers. Checked byte for
   byte against `core::pretty_typed` through a new `elya core --dump` (the
   printer exists; only the command is new).
2. **Monomorphization** (`selfhost/mono.elya`): every function and type at
   ground types, as `mono.rs` does for types (D4).
3. **Closure conversion and flattening** (`selfhost/lower.elya`): nested
   patterns to tests and binds in arm order, lambdas to a code pointer and an
   environment record, a top-level function used as a value wrapped, a
   computed callee named first (the evaluator's order of evaluation).
4. **Emission** (`selfhost/emit.elya`): LLVM IR as text, one module, built in
   a `Buffer`. `clang` compiles it with the runtime's C files.

Pieces 2-4 have no Rust twin to compare text with; they are checked through
D2, and by unit programs per piece where a fault would hide.

**D4. Values as the Rust back end lays them out, so the runtime is
unchanged.** `Int` an `i64`; `Bool` an `i1` in registers and an `i64` word in
a field; `Unit` an `i64` zero; a heap value a `ptr` to `[tag][fields...]`, a
field one `i64` word; a `String`, an `Array` and a `Buffer` the runtime's own
objects, made only by runtime calls. Each constructor of each ground type has
a tag and a descriptor row (arity and pointer mask) in the table
`elya_gc_init` takes; types are ground because of D3's piece 2, which is why
it exists: the collector is precise and must know which fields are pointers.
A closure is an object whose tag marks it as one, its first field the code
pointer, the rest the captures.

**D5. Direct style, `tailcc`, `musttail`, the shadow stack.** Every function
is `tailcc`; a call in tail position is `musttail`, so a loop written as
recursion runs in constant stack as it does natively today (the corpus
depends on it: lexers recurse once per byte). A heap value live across a call
that may allocate is pushed on the runtime's shadow stack before the call
and popped after; a function that never allocates (the Rust back end's
`noalloc` fixpoint) needs none. `&&` and `||` short-circuit as the front end
now lowers them (spec 2026-11-15-elya-short-circuit).

**D6. Built in stages, each a fragment of Elya-S, as the checker was.** A
stage lands when every corpus program in its fragment agrees under D2.

- **B1.** `Int`, `Bool`, `Unit`, arithmetic with the runtime's overflow and
  zero-divisor traps, comparisons, `if`, `let`, first-order top-level
  functions with tail calls, `main` returning `Int` or `Unit`,
  `io.println(int.to_string(..))`.
- **B2.** `String`: literals, `<>`, `==`, `string.*`, `int.*`.
- **B3.** Ground ADTs and `match`, nested patterns, literal patterns, the
  match failure.
- **B4.** Parametric types and generic functions: monomorphization.
- **B5.** Lambdas, captures, functions as values, computed callees.
- **B6.** `Array`, `Buffer`, `io.*` and the command line; argument records
  past the runtime's parameter cap if LLVM needs them (`tailcc` takes many).
- **B7.** Imports: the merged module.
- **B8.** The self-hosted sources: `lex`, `parse`, `exhaust`, `affine`,
  `check` built by the Elya back end agree with their Rust-built twins on
  their whole corpus. Part 6 starts here.

**D7. How the IR reaches `clang`.** Until part 6, the test harness writes the
IR the Elya back end prints and runs `clang` on it with the runtime (as the
codegen crate already does with its object files). Part 6 needs the compiler
to run `clang` itself: `io.run` (R3 item 1 deferred it to here) is specified
with part 6, not now.

## 2. What is not decided here

The order of lowering inside each piece, the IR's naming, how the classifier
recognises Elya-S (a syntactic walk plus the builtin list, as the checker's
stages did), and whether the Elya back end later grows effects (a new spec,
after the fixed point).
