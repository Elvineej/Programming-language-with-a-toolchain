# Effects and handlers in the back end in Elya

2026-11-28. Status: decided, not built. Step (5) of spec
`2026-11-28-elya-retiring-rust-design.md` T1, the largest. Design choices are
Claude's (rule 2).

## 0. Measured first

The back end in Elya (`selfhost/emit.elya`) refuses by name every `handle`,
`resume`, operation and effectful call ("a handler", "an effectful call").
The Rust back end implements them with *selective CPS* (slices 5b-8 to 5b-10
and their sequels): about 7,700 lines over `cps.rs` (the partition: which
functions and calls need their continuation), `anf.rs` (naming what a
continuation saves), `cps_emit.rs` (frames, handler frames, perform, resume),
`specialize.rs` (effect-polymorphic functions cloned per effect) and `any.rs`
(`with any`). The runtime half (`runtime.c`: continuation objects, handler
frames, `elya_current_handler`, one-shot and multi-shot resumption, the frame
copy) is shared by both back ends already: the back end in Elya writes calls to
it, as it writes calls to the collector.

## 1. Decisions

**E0. The same convention, word for word.** The back end in Elya emits the
Rust back end's runtime protocol (frame layouts `[tag][code][next][saved..]`,
handler frames `[tag][code][next][table][parent][saved..]`, every effectful
function, frame code and handler code answering `i64`), so the runtime is
unchanged and a program's behaviour can be compared with the Rust-built one's,
as stages B1-B8 were. No design is invented here that the Rust back end does
not already have; where the Rust code is shaped by inkwell rather than by the
convention, the Elya port is free to differ.

**E1. The partition** (`cps.rs`'s `needs_cps`, `contains_effect`, the sites):
a function is CPS when its region performs or makes an effectful call; a row
whose only label is `IO`, or that is open, is direct (D16). Ported first,
LLVM-free, and checked by dumping both partitions for every corpus program
(a new `elya cps --dump`, the house's usual oracle).

**E2. One-shot handlers, direct `handle`s**: frames, the handler frame and
its return clause, `perform` through `elya_current_handler`, `resume` once,
tail and non-tail. The corpus's handler programs that need only this compare
natively, under the collector's stress as before.

**E3. CPS `handle`s (a handler that leaks an effect), effectful lambdas and
effectful `match`** (slices 5b-9a, 5b-9b, 5b-10).

**E4. Multi-shot** (`with multi`, the frame copy on resume).

**E5. Effect-polymorphic functions** (`specialize.rs`'s clones per effect,
and its guard), then **`with any`** (`any.rs`).

**E6. B8 again**: the self-hosted sources keep compiling to the same IR (none
of them uses a handler), and the whole corpus is compared with no refusal
left but those Rust's back end makes itself.

Each stage: red tests first (corpus programs it should now compare, focused
programs at its edges), negative controls, an independent review, the gate at
its predicted count, Windows green, then `main`.

## 2. Not decided here

Whether the Elya port keeps `anf.rs` as a separate pass or names saved values
while emitting (the Rust split exists for inkwell's sake); decided in E2's
plan, measured on the corpus's largest handler programs.
