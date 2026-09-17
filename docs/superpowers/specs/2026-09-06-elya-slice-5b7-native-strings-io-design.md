# Slice 5b-7 — Native Strings and `io.println` (arc node N6) — Design

**Status:** design approved in brainstorming at the user's five-fork cut; awaiting spec review.
**Predecessor:** Slice 5b-6 (native closures, N5), closed at `5cd3d2a` + close-out `bdd2ad4`.
**Successor candidates:** N7 (runtime polymorphism), N8 (effects).

## 0. What this slice ships

A native Elya program can **say something**. Until now the only channel out of a
compiled program was the `%lld` the shim prints for `main`'s Int return value.
After this slice a program prints text, and the text is checked against what the
evaluator printed for the same source.

```elya
pub fn main() {
  io.println("hello")
  io.println("from native code")
  42
}
```

Compiled and run, that writes two lines to stdout, then the shim's `42`, and the
differential test holds the two lines against `Interp::output()`.

Four things make it work, and they are the four the forks decided:

1. A string is a **tagged heap block**, allocated by the runtime, traced by the
   collector, swept like any other object (§1).
2. `is_heap_ty` widens to admit `Ty::Base(TyCon::Str)`, or a string in an ADT
   field or a closure capture is collected while live (§2).
3. **`Unit` becomes representable** — without it no program can sequence a
   `println` at all (§3). This was not on the fork map and is the slice's
   load-bearing surprise.
4. `io.println` reaches the back end at all: one `node_types.insert` in the type
   checker, and a distinct Core node for the builtin call (§4).

## 1. Fork 1 — a string is a heap block, tagged and traced

**Decision: option B.** A string literal evaluates to a pointer to a heap block
allocated through `elya_alloc`, with a real descriptor row and a real tag.

### 1.1 Layout

```
[ word0: (size<<1)|mark ][ word1: next-link ]   <- the two internal prefix words
[ tag ][ len ][ bytes ... ]                     <- the visible payload
```

`tag` and `len` occupy one word each; the bytes follow, packed eight to a word.
Visible words are therefore `2 + ceil((len + 1) / 8)`. The `+ 1` reserves the
NUL: `elya_alloc` already zeroes both the `calloc` path and the recycle path, so
NUL termination costs nothing beyond that one reserved byte — but the **sizing**
must reserve it, or a string whose length is an exact multiple of 8 runs its
terminator into the next block.

This is deliberately the same shape as an ADT object and a closure block, for the
reason 5b-6 gave and then proved: the one function whose failure mode is silent
(`gc_mark`) gains no second dispatch path. `gc_mark` is expected to come out of
this slice **byte-identical** for the third slice running, and if it does not,
that is a signal the representation is wrong, not a signal to patch `gc_mark`.

### 1.2 The descriptor row, and a contract that must widen

The string row is `arity = 0, mask = 0`. Nothing that follows the tag is a heap
reference: `len` is an integer and the bytes are bytes.

That row makes explicit a distinction the descriptor table has never had to draw,
because until now the two coincided for every row:

> The descriptor's `arity` is **the count of traced-candidate words following the
> tag**, NOT the block's size. `Block::meta = (size << 1) | mark` remains the
> single source of truth for sweeping.

For every constructor row, `arity` = the field count = the words after the tag.
For every lambda row, `arity` = `1 + n_captures` = likewise. For the string row
they diverge: `arity` is 0 while the block may be dozens of words. `gc_sweep`
already reads the size from `b->meta >> 1` and never from the descriptor, so the
divergence is safe **today** — but it is safe by accident of how `gc_sweep` was
written, and this slice makes it safe on purpose by writing the contract into
`runtime.c`'s comment block next to `gc_descriptors`.

**Tag assignment.** The string row goes after the lambda rows:
`n_real_ctors + lambdas.len()`. It gets the same hard drift guard the lambda rows
got at `crates/codegen/src/lib.rs:1391-1395` — a `Unsupported` error, not a
`debug_assert`, because release builds must not skip it, and because a tag that
disagrees with its row index mis-traces silently. As built, the string guard sits
at `lib.rs:1412-1416`.

**Corrected after implementation: this guard is proven by its own existence, and
no test proves it.** §10 item 4 called it *"a compile-time refusal, tested as
one"*, and that is the one claim in this spec implementation did not support. A
**refusal** is program-triggerable: some source text reaches it and gets a message
back, and every other refusal this slice adds (§8) is tested exactly that way. This
is not one. `string_tag` is *computed* as `n_real_ctors + lambdas.len()` and `desc`
is *built* to precisely that many rows, a few lines apart in the same function —
so the two agree by construction, and **no Elya program can make them disagree**.
What the guard defends against is a future edit that changes one and not the
other; that edit fails the build, which is the entire point. The lambda guard it
was modelled on has no test either, for the same reason, and that should have been
the tell when §10 was written.

The plan's acceptance table has it right: it names the guard's
`Unsupported("string tag disagrees with its descriptor row index")` as the proof,
not a test of it. The discipline this settles, for every slice after: **a
structural invariant is discharged by the check that enforces it; only a
reachable refusal owes an execution test.** Writing a test here would mean
reaching into the compiler to break an invariant no program can break — a test of
the test, not of the language.

### 1.3 Why a descriptor row at all

`gc_gray_push` sets the mark bit **before** the tag-range check, so an object
whose tag is out of range **survives**; what it loses is the tracing of its
children. A string is a leaf, so one could argue the row is unnecessary and rely
on the out-of-range skip.

That argument is rejected, and the reason is diagnostic rather than functional:
relying on the skip would make a **real** missing row indistinguishable from a
**deliberate** one, turning hazard direction (a) from a bug into a mechanism. The
invariant "every live tag is `< gc_n_ctors`" stays total and stays checkable.

### 1.4 Named and rejected: the static `.rodata` block (option A)

Option A was to emit each literal as a constant global with the same header
shape, never allocated and never swept.

It is rejected because **a test refuses it**.
`mask_and_repr_agree_on_pointers` (`crates/codegen/tests/native_codegen.rs:1947-1969`)
asserts the biconditional `repr_ty(t) is a pointer ⟺ is_heap_ty(t)`. Under
option A a `Str` is a pointer that is not heap-resident, so the test goes red,
and the only way to ship A is to weaken the invariant to
"pointer ⟹ traced **or statically allocated**". That predicate is exactly what
catches hazard direction (b) (§2). It is one slice old and it is already refusing
a bad representation choice on its first outing — which is the strongest evidence
available that it is worth keeping sharp.

### 1.5 What is NOT rejected: constant bytes in `.rodata`

Rejecting option A does not mean the literal's bytes are synthesized at runtime.
The bytes live in `.rodata` as an ordinary constant global and are passed to the
runtime by pointer. What is heap-allocated is the **block**; what is static is
the **source bytes**. The value the program hands around, stores in fields, and
captures in closures is always the heap block pointer, never the `.rodata`
pointer — so `repr_ty(Str)` is a pointer that is always heap-resident and the
biconditional holds.

### 1.6 Strings at or above `GC_MAX_WORDS`

`elya_alloc` free-lists blocks below `GC_MAX_WORDS = 16` visible words and calls
`free()` at or above. A 113-byte string is 16 visible words and takes the `free()`
path. This is correct and needs no change — the constant is not nudged, and the
behaviour is stated here so a later reader does not read it as an oversight.

## 2. The hazard with teeth at N6 is direction (b)

`is_heap_ty` is currently `matches!(ty, Ty::Con(..) | Ty::Fn(..))`
(`crates/codegen/src/lib.rs:204`). It must widen to admit
`Ty::Base(TyCon::Str)`.

If it does not: a string stored in an ADT field or captured by a closure leaves
its mask bit **clear**, the collector does not trace it, the block is swept while
live, and it is recycled and overwritten by the next allocation of a compatible
size. The value read back is a wrong string, not a crash.

**This is the slice's headline acceptance test, and it is constructible from the
surface.** Verified rather than assumed: `elaborate_adt_ty`
(`src/types.rs:1497-1545`) maps `"String" => Ty::str()` at `:1513`, and
constructor schemes are built from its output at `:1790`
(`field_tys.push(elaborate_adt_ty(..))` inside the variant loop). So an ADT
field declared `String` genuinely carries `Ty::Base(TyCon::Str)` into
`LowerCtx::ctors`' `field_tys`, which is the list the descriptor mask is
computed from. The 5b-6 close-out could not build this test for `Ty::Fn`
because the surface has no function-type annotation; for `Str` the surface
has had the annotation all along.

**The test asserts on the string content read back** — never on survival, never
on exit status, never on the absence of a crash. That is the standing rule 5b-6
paid for: a freed closure was still callable, and a freed string block will still
be readable, because the collector recycles rather than unmaps.

### 2.1 The direction-(a) control would be VACUOUS here, and must not be built as if it had teeth

The mirror hazard — omitting the string's descriptor row — does **not** produce a
wrong answer, because `gc_gray_push` sets the mark bit before the tag check
(§1.3) and a string is a leaf with no children to lose. An un-fixed build with the
row omitted would print the right string.

A control that passes on the un-fixed build is precisely the vacuous shape this
project has been bitten by twice. So: **no direction-(a) execution control is
built for strings.** The row's necessity is argued structurally (§1.3), and the
tag/row-index agreement is enforced by the hard drift guard (§1.2), which is a
compile-time refusal and testable as one.

Stating this in the spec is the point. The failure mode being guarded against is
not "we forgot a control" but "we shipped one that could not fail".

### 2.2 Rooting comes for free; the mask does not

Two different predicates govern two different mechanisms, and it is worth being
explicit because they are easy to conflate:

- **Rooting** (`gc_root`) keys on `v.is_pointer_value()` — the LLVM value, i.e.
  on `repr_ty`. Widening `repr_ty(Str)` to a pointer therefore gives strings
  shadow-stack rooting everywhere, with **no changes to rooting code**.
- **Tracing** (the descriptor mask) keys on `is_heap_ty`.

`mask_and_repr_agree_on_pointers` is the tie between them. That is why widening
`repr_ty` without widening `is_heap_ty` is not merely incomplete but detectably
inconsistent.

## 3. `Unit` must become representable — the blocker that was not on the fork map

`repr_ty` currently maps `Int → i64`, `Bool → i1`, `Ty::Con(..) → ptr`,
`Ty::Fn(..) → ptr`, and everything else — including both `Str` **and `Unit`** —
to `Unsupported("unrepresentable type")`.

`io.println` returns `Unit`. And `lower_block` (`src/core.rs:205-232`) turns a
bare statement `Stmt::Expr(e)` into `("_".to_string(), lowered)` — that is, into a
**`Let` node whose right-hand side is the call**. A `Let`'s rhs goes through
`repr_ty` to get its slot type.

So without Unit representability, `io.println("hi")` as a statement fails
codegen, and **no program can sequence a `println` at all**. Making strings work
while leaving Unit unrepresentable ships a feature that cannot be used. This is
in scope, and it is not optional.

**Decision:** `repr_ty(Ty::Base(TyCon::Unit)) = i64`, with the value `0`.
`is_heap_ty(Unit)` stays **false** — Unit is an immediate, not an object. The
zero word is never dereferenced, never traced, and never rooted (it is an
IntValue, so `gc_root`'s `is_pointer_value()` check declines it, which is
correct). `require_int` is **not** relaxed: it still admits only
`Ty::Base(TyCon::Int)`, so `main` must still return an Int.

`mask_and_repr_agree_on_pointers` gains **two** new cases — `Str` (pointer,
heap) and `Unit` (not a pointer, not heap) — because its current case list is
exactly `[Int, Bool, Con("List"), Fn([Int])->Int]` and contains neither.

## 4. Fork 2 — how `io.println` reaches the back end

### 4.1 The recorder gap is `Untyped`, not `Ty::Error` — a correction

The 5b-6 ledger filed this under the recorder-totality class as a `Ty::Error`
problem. That was wrong in a way that matters for the size of the fix.

What actually happens: the io.println builtin branch (`src/types.rs:1094-1105`)
routes only the **arguments** through `infer_expr`. It unifies against
`want = Ty::Fn(vec![Ty::str()], EffectRow::pure(), Unit)`, calls
`add_effect(amb, "IO", ...)`, and returns `Ty::unit()` — and it **never visits the
callee node**. Since `infer_expr` is what records into `node_types`
(`src/types.rs:880-884`), the callee's span is simply **absent** from the table.

Core's `lower_expr` looks the type up **first** (`src/core.rs:244`), so it fails
with `LowerError::Untyped(callee.span)` and never reaches the
`Expr::Qualified { .. } => Unsupported("Qualified")` arm at `src/core.rs:327-334`.
The `Expr::Qualified { .. } => Ty::Error` arm at `src/types.rs:907` is **dead for
the call form** — it is reachable only for a qualified name in a non-call
position.

The span is **absent, not mistyped**. That distinction is the whole finding: an
absent span is a one-line recorder fix, whereas a mistyped one would have implied
inference work.

### 4.2 The fix is one `node_types.insert`

The template already exists twenty lines below, in the operation-callee branch
(`src/types.rs:1136-1157`), which ends with `self.node_types.insert(callee.span, callee_ty);`
after building the callee type **with the effect in its row**. The builtin branch
gets the same two lines, constructing:

```rust
Ty::Fn(vec![Ty::str()], <row carrying IO>, Box::new(Ty::unit()))
```

and inserting it at `callee.span`. **Not an inference redesign.** No new
unification, no change to what `add_effect` does, no change to any existing
diagnostic.

### 4.3 The row carries `{IO}`, not pure

The recorded callee type carries the `{IO}` effect row rather than a pure row.
Three reasons, in order of weight:

1. It **matches the operation precedent** at `:1136-1157`, which is the code this
   change is copying. Two adjacent branches recording the same shape differently
   is the kind of drift that costs a slice later.
2. It is **what N8's `Handle` will need**. A handler has to see which calls in its
   body perform which effects; a callee typed pure is a callee a handler cannot
   see.
3. `{IO}` is already the observed truth elsewhere: `tests/effect_types.rs:36-41`
   asserts `io.println`'s enclosing scheme prints as `fn() / {IO} -> Unit` with no
   diagnostics and no handler. Recording pure at the callee would make the callee
   disagree with the scheme its own call produced.

Pure is smaller today and wrong later. `OBSERVABLE_EFFECTS` already contains
`"IO"`, so nothing about handler-discharge behaviour changes at N6.

### 4.4 Core lowers the builtin call as its own node

`CoreKind` gains a builtin-call node — a `Builtin(name, args)` in the shape of the
existing `Ctor(String, Rc<[CoreExpr]>)`, whose own doc comment says it is
"syntactically distinct from application (5b-4 §3.2)".

This **sidesteps** the standing `"function used as a value"` refusal rather than
colliding with it. A builtin call is not an application of a first-class value:
there is no closure block, no code pointer, no `Ty::Fn` value in flight. Routing
it through `App` would require `io.println` to have a uniform function
representation — which is 5b-6's deliberately-uncut obligation and is wanted at
N7 anyway. Lowering it as its own node means N6 does not pay for that
generalisation twice.

`src/resolve.rs:10` lists exactly one builtin, so the node's name field admits
exactly one value today, and codegen refuses any other **by name**.

## 5. Fork 3 — no function-type annotation at N6

**Confirmed: N6 does not give the surface a function-type annotation.**
`TypeAnn { name: String, args: Vec<Spanned<TypeAnn>> }` (`src/ast.rs:64-67`) is
flat and has no arrow form; adding one touches the parser, the annotation
elaborator in two places, and the ADT field path — a change with no consumer at
N6, since strings need only the `String` annotation that already exists.

**Consequence for the ledger:** the ADT-field `Ty::Fn` execution test, which 5b-6
left filed against "whichever of N6/N7 first adds a function-type annotation", is
now filed against **N7**. It stops hanging between the two. N7 is where a uniform
function representation is being built anyway, so the annotation and the test land
together there.

## 6. Fork 4 — the runtime surface: two symbols

Layout knowledge stays in the C. Codegen knows the tag (it owns the descriptor
table) and nothing else about how bytes are packed.

```c
void *elya_str_lit(int64_t tag, const char *bytes, int64_t len);
void  elya_println(void *s);
```

### 6.1 `elya_str_lit`

Computes visible words as `2 + ((len + 1) + 7) / 8`, calls `elya_alloc`, writes
the tag and the length, `memcpy`s the bytes. The terminator is already zero.

### 6.2 `elya_println` writes stdout, never stderr — a hard requirement

```c
fwrite(bytes, 1, len, stdout);
fputc('\n', stdout);
```

**This is a hard requirement, not a preference.** `assert_runs`
(`crates/codegen/tests/native_codegen.rs:203-215`) asserts stderr is **empty** for
every corpus entry. That assertion is what makes the `ELY_GC_STATS` gate work:
`elya_gc_report` is the collector's diagnostic channel and it writes to stderr,
gated off by default precisely so the empty-stderr assertion holds. If `println`
wrote to stderr, the two channels would share a stream, the empty-stderr
assertion would have to be relaxed for every printing program, and the gate that
keeps GC statistics from leaking into normal runs would be gone.

`elya_match_fail` and `elya_gc_report` remain the only stderr writers in the
runtime.

### 6.3 `require_int` stays

`main` must still return `Int`, so the shim still prints exactly one `%lld` line,
last. This is not a limitation to work around — §7 depends on it.

### 6.4 The literal arm becomes an allocation site

`elya_str_lit` calls `elya_alloc`, which can trigger a collection. So the string
literal arm is an **allocation site** and needs the same `gc_root_env` /
`gc_unroot` bracket the `Ctor` arm has (`crates/codegen/src/lib.rs:905-944` —
`gc_root` per field on the way in, one `gc_unroot` of the accumulated count on
the way out).

Checked case by case:

- `Ctor(a, "x")` and `f(a, "x")` — already safe. The enclosing `Ctor`/call arm
  brackets its own operands, and that bracket covers the literal.
- `let a = Cons(1, Nil) in let s = "x" in a` — **not** safe without the new
  bracket. `a` is live across the literal's allocation and nothing else roots it
  at that point.

The `bytes` argument points into `.rodata`, not the heap, so it is not a root and
must not be pushed as one.

## 7. Fork 5 — the differential test

**Settled: compare `interp.output()` against native stdout minus the trailing
`%lld` line.**

### 7.1 The comparison rule, spelled exactly

The reference side is free: `run_module_value` already returns the `Interp`
(`src/eval.rs:269`), and `eval_main_int`
(`crates/codegen/tests/native_codegen.rs:416-433`) currently discards it with
`let (_, v) = ...`. Bind it and read `interp.output()` (`src/eval.rs:195`). The
evaluator **buffers** rather than writing stdout (`src/eval.rs:201`), so there is
no interleaving question on the reference side.

The native side is split mechanically. Spelled so that `io.println("")` is handled
correctly — strip exactly **one** trailing newline, never `trim_end_matches`:

```rust
let s = String::from_utf8_lossy(&out.stdout).into_owned();
let s = s
    .strip_suffix('\n')
    .expect("native output must end with the value line's newline");
let (text, value) = match s.rsplit_once('\n') {
    Some((t, v)) => (format!("{t}\n"), v.to_string()),
    None => (String::new(), s.to_string()),
};
```

`text` is compared against `interp.output()`; `value` is parsed and compared
against the evaluator's Int result exactly as today.

**Forks 4 and 5 are coupled, and the coupling is the argument.** "Minus the
trailing line" is mechanical *because* `require_int` stays: `main` returns Int, so
the shim prints exactly one line, last, always. Relaxing `require_int` would make
the split a guess. Whoever later proposes relaxing it must re-open this section.

### 7.2 The negative controls are mandatory

A text comparison that passes on empty-vs-empty is the vacuous shape this project
has been bitten by twice. Both controls below are **built, run, and shown failing
— and failing differently** — before the green counts.

- **C5-a — `elya_println` writes nothing.** The differential must fail on a
  **text mismatch**. Specifically: not on exit status, and not on the value line.
  If it fails on the value line, the split in §7.1 is wrong. If it fails on exit
  status, the test is measuring the wrong thing.
- **C5-b — `elya_println` writes to stderr instead of stdout.** The corpus's
  empty-stderr assertion must fire. This is the executable proof of §6.2's hard
  requirement, and its failure signature must differ from C5-a's.

Two controls, two distinct failure signatures. If both fail the same way, neither
is discriminating and the test is not yet proven.

### 7.3 A positive requirement

**At least one corpus entry must print more than one line.** A single-line corpus
cannot distinguish "prints the text" from "prints the last line", and cannot catch
an off-by-one in the `rsplit_once` split. At least one entry must also print a
string containing no characters (`io.println("")`), which is what §7.1's
`strip_suffix` spelling exists to handle.

## 8. Further reach surfaced while writing this spec

Everything in this section was found during the pre-spec evidence sweep and is
**not** on the fork map. Items 8.2 and 8.3 are new work this slice must do; 8.4 is
pre-existing and N6-adjacent.

### 8.1 The general principle

> **Widening `repr_ty` un-shadows every downstream site that was protected only
> by `repr_ty`'s refusal firing first.**

`repr_ty` runs early and refuses unrepresentable types before the rest of lowering
sees them. That refusal has been silently protecting several sites that are not
themselves total. Widening it to admit `Str` and `Unit` removes the shield.

This is worth naming because **it will happen again at N7**, where widening
`Ty::Var` will un-shadow the same class of site. The fix pattern is the same
every time: find the sites that dispatch on `Ty`, and make each one total or
refuse by name.

The sites that dispatch on `Ty` in non-test code are `repr_ty` (`:184`),
`is_heap_ty` (`:204`), `require_int` (`:228`), the Eq/Ne operand guard (`:735-739`),
and the four word-conversion sites of §8.3. That is the whole list.

### 8.2 `<>` (`BinOp::Concat`) becomes a compiler PANIC

The surface already has one string operation, which the fork map did not account
for: `BinOp::Concat`, spelled `<>`, types `String × String → String`
(`src/types.rs:796`) and is implemented in the evaluator (`src/eval.rs:236`).

In the Prim arm (`crates/codegen/src/lib.rs:725-765`), operands are lowered with
`.into_int_value()` at `:740-741`, **before** the `other =>` arm at the bottom
that refuses `Concat` by name. Today `repr_ty(Str)` errors first, so the panic is
shadowed. Once `repr_ty(Str)` yields a pointer, the operand lowers to a
`PointerValue` and `BasicValueEnum::into_int_value` **panics** — verified in
inkwell 0.5.0 (`values/enums.rs:305-311`: `panic!("Found {:?} but expected the
IntValue variant", self)`).

**Fix:** hoist the `Concat` refusal above the operand lowering, in the exact shape
and position of the existing Eq/Ne operand guard at `:729-739` — whose own comment
already argues for that ordering, saying the refusal is placed before the operands
are lowered "so the message names the operator rather than reporting the operand's
type as unrepresentable". The message already exists: `op_label` covers all 18
`BinOp`s including `BinOp::Concat => "Concat"` (`:103`). Only the **position** is
wrong.

The Eq/Ne guard is already total for `Str` — it admits only Int and Bool — so
`"a" == "b"` stays correctly refused **by name**, and deserves a test asserting
that it errors rather than panics.

### 8.3 The `Unit` decision creates a latent panic at four word-conversion sites

Four sites convert between a value and an i64 heap word, and all four have the
same three-arm shape:

| Site | Line | Direction |
|---|---|---|
| Closure capture read (lifted-body prologue) | `lib.rs:334-341` | word → value |
| Closure capture store (closure construction) | `lib.rs:822-830` | value → word |
| Ctor field store | `lib.rs:930-938` | value → word |
| Ctor field read (match binding) | `lib.rs:1000-1007` | word → value |

The arms are `Ty::Base(TyCon::Int)` (identity), `Ty::Base(TyCon::Bool)`
(zext/trunc), and `_` (`ptrtoint` / `inttoptr`).

`Str` falls to `_` and is handled **correctly** — a string is a pointer.
`Unit` also falls to `_` and is handled **wrongly**: on the store side it is an
`IntValue`, so `.into_pointer_value()` **panics**; on the read side the word is
reinterpreted as a pointer.

Both exposures are constructible from the surface once N6 lands:
`data Box { MkBox(Unit) }` gives a Unit-typed ADT field (`elaborate_adt_ty` maps
`"Unit"`), and `let u = io.println("hi") in fn(x) { u }` gives a Unit-typed
capture. Neither is constructible today, because `repr_ty(Unit)` refuses first —
§8.1 again.

**Fix:** the conversion is extracted into **one pair of helper functions**
(`value_to_word` / `word_to_value`), each matching once, and all four sites call
them; the `Ty::Base(TyCon::Unit)` arm is then added in one place rather than
four. This is a requirement of the slice, not a preference: the whole argument
for the shape is that ONE negative control then covers four sites, which four
inline arms would not give. It follows the precedent `is_heap_ty` set in 5b-6 —
whose own comment says it is "ONE predicate, used by both the constructor
descriptor rows and the closure descriptor rows, so a single negative control
falsifies both call sites" (`lib.rs:197-198`).

### 8.4 `Match` has no scrutinee-type guard — pre-existing

`CoreKind::Match` lowers its scrutinee with an unconditional
`.into_pointer_value()` (`lib.rs:949`), and nothing between there and the arm
bodies guards it. `match 1 { _ => 2 }` **panics the compiler today**. This is
pre-existing, not N6-created.

N6 makes it worse in a subtler way: a `Str` scrutinee reaches the tag load
**without** panicking, because a string block genuinely has a tag word — and then
compares the string's tag against constructor tags, falling through every arm to
`elya_match_fail`. Wrong behaviour instead of a loud crash.

**In scope for N6:** a one-line by-name refusal that the scrutinee must be
`Ty::Con(..)`. It closes the pre-existing Int case as a side effect. This is a
refusal, not a feature — string matching is not being added.

`CorePat::Lit(_)` is already refused by name at `lib.rs:1018` and `lib.rs:1070`
(obligation T6), so string **literal patterns** are already safe and add no new
hazard.

### 8.5 Test-harness reach

- `assert_runs` (`native_codegen.rs:203-215`) trims all of stdout into a single
  string and compares it against one expected value. It is fine for the existing
  non-printing corpora and must stay untouched; printing programs need a sibling
  helper implementing §7.1's split.
- `run_with_gc_stats` (`:725-760`) also returns stdout and must stay consistent
  with the same split rule, or a printing program in a GC test will mis-parse its
  value line.

### 8.6 Front-end reach is near zero

Worth recording because it is the reason this slice is small.
`TyCon::Str`, `Token::Str`, `Expr::Str`, `PatLit::Str`, `Value::Str`, and
`CoreLit::Str` **all already exist**, as does `"String" => Ty::str()` in both
`ann_to_ty` (`src/core.rs:188-200`) and `elaborate_adt_ty` (`src/types.rs:1513`).
The front end has been able to type and evaluate strings since well before the
native arc began. N6 is a back-end slice with one two-line front-end recorder fix
(§4.2).

## 9. Scope — what this slice does NOT ship

- **No string operations.** `<>` is refused **by name** (§8.2); `==`/`!=` on
  strings stay refused by name by the existing Eq/Ne guard. No length, no
  indexing, no slicing, no comparison.
- **No string interning and no literal hoisting.** Each evaluation of a string
  literal allocates a fresh block. A literal inside a loop allocates once per
  iteration. This is a deferred obligation, not an oversight — recorded in §11.
- **No function-type annotation** (§5).
- **`require_int` is not relaxed** (§6.3).
- **`io.println` is typed strictly `fn(String) -> Unit`.** No printing of Ints,
  no formatting, no interpolation. `resolve::builtins()` gains no entries.
- **No string matching.** Match on a string scrutinee is refused (§8.4).
- **`Float` stays refused.**
- **No native effect handlers.** `{IO}` is recorded in the row (§4.3) and
  discharged exactly as today; `Handle`/`Resume` are N8.

## 10. Testing

Every guarantee below is proven by a built-and-run execution test, with two
exceptions. Item 9 is a review check, because `gc_mark` not changing is not a
behaviour a test can observe — that exception was stated from the start. Item 4
turned out to be a second, found during implementation: it is a structural
invariant no program can trip, discharged by the check that enforces it (§1.2).
No IR snapshots for native behaviour, no `#[ignore]`.

1. **The corpus prints.** New printing corpus entries, at least one printing
   **more than one line** and at least one printing the empty string (§7.3), each
   checked by the §7.1 differential against `Interp::output()`.
2. **The differential's two mandatory negative controls** (§7.2), built, run, and
   shown failing **differently**.
3. **Direction (b), the headline hazard** (§2): a `String` in an ADT field, with
   `is_heap_ty` un-widened, built and run, asserting on the **string content read
   back** — and shown producing a wrong value rather than a crash.
4. **The tag/row drift guard** (§1.2) — **corrected: proven by its own
   existence, not by a test.** It is a structural invariant, not a reachable
   refusal: no surface program can make the tag and the row index disagree, so
   there is nothing to write an execution test against. The plan's acceptance
   table already named the guard itself as the proof; this item was the single
   place this spec overclaimed. See §1.2.
5. **`mask_and_repr_agree_on_pointers` gains `Str` and `Unit`** (§3).
6. **Sequencing works**: a program with a `println` statement followed by more
   statements compiles and runs — the direct test of §3.
7. **Refusals by name, each asserting the specific `Unsupported` message**:
   `<>` on strings (§8.2), `==` on strings, a non-`Ty::Con` match scrutinee
   (§8.4), and a builtin name other than `io.println`.
8. **The Unit word-conversion arms** (§8.3): a Unit-typed ADT field and a
   Unit-typed closure capture, both compiled and run, asserting on the value read
   back.
9. **`gc_mark` is byte-identical.** Asserted by review, the way 5b-6 did it: if
   the diff is non-empty, the representation choice is wrong.

## 11. Obligations ledger after this slice

**Discharged:** none of the standing obligations. N6 is additive.

**New:**

- **String literal allocation is not hoisted or interned** (§9). Each evaluation
  allocates. Wanted when a real workload puts a literal in a hot loop; not before,
  and not without a measurement.
- **The descriptor `arity` contract is now genuinely wider than "size"** (§1.2).
  Any future row whose traced-candidate count differs from its word count must
  respect that `gc_sweep` reads `meta`, never the descriptor.

**Carried, unchanged:** T2 (whole-module representability), T5 (the sequential
match chain is an unoptimised floor), T6 (literal-pattern match unfocused), T7
(refcount-vs-tracing divergence, re-filed at N8), uniform function representation
(N7), the affine obligations, and the relay+own-effect row leak.

**Re-filed by this slice:** the ADT-field `Ty::Fn` execution test now files
against **N7** rather than hanging between N6 and N7 (§5).

**Settled by execution, recorded at close-out:**

- **`gc_mark` came out byte-identical**, as §10 item 9 required and §2 predicted.
  Measured rather than eyeballed: the function extracted from `runtime.c` at
  `1578db9` and at close-out is **638 bytes on both sides, byte-for-byte equal**.
  The collector traces a string-bearing heap with exactly the code 5b-5 shipped;
  N6 added a descriptor row and touched the tracer not at all. `runtime.c`'s
  whole change is +40/-5, and all of it is `elya_str_lit`, the `<string.h>` and
  Win32 `<fcntl.h>`/`<io.h>` includes that keep stdout binary, and comment text.
- **§10 item 4 overclaimed** and is corrected in place (above, and §1.2). Of the
  nine testing items, seven are discharged by execution, item 9 by review as
  stated from the start, and item 4 by the guard's own existence — which is what
  the plan's acceptance table said all along. No test was invented to close it
  and no code was changed to suit it.
- **The differential's two negative controls failed differently, and were
  removed** (§7.2, item 2). The text arm compares `""` against `"hello\n"` and the
  empty-stderr arm fails on a different assertion in a different test. Both were
  built, run, observed failing, and reverted in the same commit that added the
  proofs they control — so the harness is shown to be able to fail before it is
  trusted to pass.
