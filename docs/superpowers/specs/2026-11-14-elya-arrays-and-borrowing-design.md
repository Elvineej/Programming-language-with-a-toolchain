# Self-hosting, part 2b: linear arrays, read by borrowing

**Status:** built (2026-11-14, on `main`, an interactive session; the plan
`docs/superpowers/plans/2026-11-14-elya-arrays-and-borrowing.md`). HANDOFF
step 1, part 2b (spec 2026-11-14-elya-self-hosting R3). Design choices are
Claude's (rule 2). It takes the first of a reviewer's directions the
maintainer asked to be weighed (docs/ROADMAP.md, 2026-11-14: a function's
contract says whether an argument is borrowed or consumed), in the smallest
form that is sound.

## 0. Measured first

- A compiler's tables (names, types, tokens by index) are lists today: the
  `n`th element costs `n` steps, and the lexer in Elya already pays it
  nowhere only because it never indexes. Part 4's parser reads tokens by
  position. (The plan measures a list's `nth` against the array's.)
- The buffer (part 2a) showed the way for writes: a `linear` value updated in
  place because no one else holds it. It has no reads: `finish` ends it.
- An array must be read without being ended. The affine pass counts every
  occurrence of a binding as a use (`affine.rs`, D2 of spec
  2026-11-06-elya-affine-across-functions), so `array.get(a, 0)` followed by
  `array.get(a, 1)` is E0428 as things stand.
- Elya has no tuples, so "the element and the array back" would need a type
  declared per use; and it would thread the array through every read.
- Natively a generic function is compiled per instance (monomorphised), so an
  array's element type is known where it is made: pointers or not.

## 1. Decisions

**D1. `Array(a)`, a builtin linear type, opaque,** as `Buffer` is: its name
reserved (E0211), not comparable, not data for `any`. Its elements must be
duplicable (an `Array(Buffer)` is refused, E0437's words): a read copies an
element out, and an affine element would then be held twice. Arrays of affine
elements wait for a swap.

**D2. Five builtins, pure:**

| builtin | type | the array is |
|---|---|---|
| `array.new()` | `fn() -> Array(a)` | made, empty |
| `array.push(xs, x)` | `fn(Array(a), a) -> Array(a)` | consumed; its successor answered |
| `array.set(xs, i, x)` | `fn(Array(a), Int, a) -> Array(a)` | consumed; its successor answered |
| `array.get(xs, i)` | `fn(borrow Array(a), Int) -> a` | borrowed |
| `array.length(xs)` | `fn(borrow Array(a)) -> Int` | borrowed |

An index outside `0 .. length` stops the run by name, the same words on both
back ends: `elya: array index <i> out of range (the array has <n>)`. `push`
is amortised O(1) (the capacity doubles, as the buffer's); `set`, `get` and
`length` are O(1). `buffer.length(borrow Buffer) -> Int` joins them.

**D3. A borrow is a use that does not consume; it happens at its call, and
must not come after the use that consumes.** In the affine pass an occurrence
of a binding *as the argument of a borrowed parameter* counts no use. What
makes that sound is one new rule, checked in the same pass, per body (a
function's, a lambda's, a clause's):

> a borrow of a binding takes place when its call is made, after all of that
> call's arguments are evaluated, and lasts until the call returns; it must
> not take place after a consuming use of the binding (E0438: "`xs` is read
> here after it was given away").

Evaluation order is the language's (arguments left to right, a block's
statements in order; the arms of an `if` or a `match` are alternatives, and
after them the binding is consumed if any arm consumed it). A binding given
as an argument is given away when that call is made, as a borrow takes place
then, and before the call's own borrows: so `array.set(xs, i, array.get(xs,
i) + 1)` reads and then writes (the `get`'s call is made first), and
`walk(xs, xs, 1)` is refused. Any other consuming use gives the binding away
where it is written; a lambda that captures it, where the lambda is written
(conservative: it may run whenever). The first draft
placed the borrow where its argument is evaluated; ChatGPT's review broke it
with `array.get(xs, f(xs))`, where `f` consumes and updates `xs` while the
read is pending (and the same through a branch, a one-shot handler, a
recursive call and `walk(xs, xs, 1)` with one borrowed and one consumed
parameter). Placing the borrow at the call refuses each: a consuming use in
any argument of the call comes before it. During the call nothing else can
consume the binding: the callee holds only the borrow (D5), a lambda that
captures it was an argument (so a use before the borrow), and an operation
clause that uses it is refused already (a clause counts many uses, E0428).
Without the rule a read could see a later write (the update is in place),
which a program that reads as pure must never observe. With it, every read
sees the value the text names.

**D4. Where a borrow cannot be followed, it is an ordinary use.** A borrow
inside a lambda that captures the binding counts as a use of the capture, as
any occurrence does now (the lambda may run after the consuming use, or
twice). So does a borrow of a binding held across a `multi` point (E0429
applies unchanged). So does a borrow, in the body of a `handle` whose
continuation may escape (a clause has a `resume` inside a lambda), of a
binding from outside that body: such a handler may answer before its body has
finished, and what follows the `handle` then runs while the borrow is pending.
ChatGPT's second review found it: `handle { Done(array.get(xs, pause())) }
with { Pause.pause() -> Later(fn() { resume(0) }) }`, then `array.set(xs, 0,
99)`, then the kept continuation resumed, read 99. A one-shot continuation is
resumed at most once, not before what follows its handler; D3's order is the
order within a body, which a handler that only aborts or resumes in its
clause keeps. The rule is conservative (a lambda with a `resume` that runs
before its clause returns counts too) and refuses the review's five programs
(Findings 1-5, among them a continuation copied through an array and through
a `borrow` function): there the read is a use, and the write a second
(E0428), or a `borrow` parameter is given away (E0439). Nothing is refused
that was accepted before.

**D5. A function may declare a borrowed parameter:** `fn total(borrow xs:
Array(Int), i: Int) -> Int`. In its body `xs` may occur only as a borrowed
argument (of a builtin or of another function's `borrow` parameter): it is
never consumed, returned, stored in a value, or captured by a lambda (E0439:
"a borrowed value is given away here"; through a generic `borrow x: a`,
E0437 at the instance). At a call of a function *by its
name*, an argument in a `borrow` position is a borrow (D3). A function value
(`let f = total`, a parameter, a closure) does not carry the mode: called
through one, the argument is consumed as now, and a `borrow` function used as
a value is refused where its parameter's type is linear (E0439), since the
caller could not know. `borrow` on a parameter of a duplicable type changes
nothing and is allowed (generic code).

This is the whole of "ownership in the contract" taken here: two modes,
consumed (the default) and borrowed, on named functions. Function *types*
with modes, and closures that say they run once, are the next step (the
roadmap), after the switch if part 4 does not need them.

**D6. Representation.** In the evaluator `Value::Array(Rc<RefCell<Vec<Value>>>)`,
written through the `RefCell` as the buffer is. Natively one object,
`[tag][length][capacity][words]`, each element one word (every Elya value
is). Two tags, chosen where the array is made from the element's type (the
predicate a constructor's field uses, `is_heap_ty`): an array of heap values
carries a tag of its own, whose descriptor row has the arity -1, which
`gc_scan` reads as "`length` words from the fourth" (a row cannot say a
length that changes); an array of anything else carries the string tag,
traced nowhere, as the buffer does. `push` past the capacity allocates the next object and copies, the
operands rooted. No write barrier: the collector has none to keep (it stops
the world and marks everything); a generational collector (HANDOFF step 2)
would need one at `set` and `push`, and the audit of old-to-young stores
gains these two.

**D7. Maps are Elya,** over arrays: `elya/map` in the standard library, open
addressing on a string's bytes, written after this lands, with the lexer's
keyword lookup and part 4's symbol tables as its first users. *Built*
(2026-11-15; ChatGPT wrote it to Claude's brief, Codex being out of usage,
checked here). A borrowed value cannot be taken apart (matching a `borrow`
parameter consumes it, E0439), so the map is not a declared type around an
array: it is one `Array(MapSlot(a))`, `type MapSlot(a) { MapEmpty,
MapFull(String, a), MapCount(Int) }`, slot 0 the count, slots 1 to the
capacity (a power of two, first 8) a table probed linearly, FNV-1a over the
key's bytes, doubled when an insert would pass three quarters. `map.new()`,
`map.put(m, k, v)` (consumes, answers the successor), and, borrowing,
`map.get(m, k)` (`MapFound(v)` or `MapMissing`), `map.has`, `map.size`,
`map.get_or(m, k, default)`. No deletion yet. Its values are duplicable, as
an array's elements. Since `MapSlot` is the module's, an importer can build
or read a slot: an abstract type (HANDOFF step 4) would close it. Measured natively:
a million inserts and a million lookups of decimal keys in 4.5 s (250,000 in
1.1 s: linear); half the time was the shadow stack's pushes and pops around
calls (callgrind, 46%). The emitters then stopped rooting around reads that
allocate nothing and around arguments followed only by ones that cannot
allocate (`crates/codegen/tests/native_roots.rs`): 3.1 s. What remains is
mostly the collector (a `MapFull` per insert) and calls to user functions,
which count as allocating until an analysis says otherwise.

**D8. Not here:** arrays of affine elements (a swap); slices; a borrow that
is returned or stored (regions); modes in function types.

## 2. Tests (planned)

Both back ends, the same bytes: push across growth, set, get, length, out of
range by name; an array of strings and one of ints under `ELYA_GC_STRESS`
(the freed-block check included); a read after the consuming use (E0438) in
sequence, across arguments, after an `if` one arm of which consumed; reads
before it, and in both arms, accepted; a `borrow` parameter read twice and
passed on, accepted; one consumed, returned or captured (E0439); a `borrow`
function called through a value (consumed, so E0428 on a second use);
`Array(Buffer)` refused; `nth` on a list against `array.get`, the measurement.

## 3. Measured after

The sum of every element read by index, natively: 40,000 elements took 3.33 s
through a list's `nth` and 6 ms through `array.get`; 4,000,000 elements are
read within the test's two seconds. The plan has the table.
