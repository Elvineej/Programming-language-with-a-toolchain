# The back end in Elya: the plan

Spec: `docs/superpowers/specs/2026-11-27-elya-backend-in-elya-design.md`.
Each step: red test first, negative controls run and reverted (`cmp`), a
review by an agent that has not seen the work, the gate at the predicted
count, Windows CI, then `main`.

1. [x] **Core** (`selfhost/core.elya`, D3 piece 1), 2026-11-27. `elya core
   --dump` added to the CLI (`core::pretty_typed` of `core::lower_module`).
   The checker in Elya gains `typed` (every node's type, resolved through
   the final substitution) and `show_ty`/`no_names`, so one printer names
   the module's variables as Rust's one `TyPrinter` does. Tests:
   tests/selfhost_core.rs (evaluator) and
   crates/codegen/tests/selfhost_core.rs (native), every corpus program
   without imports that checks (358; 8 with a `Float` literal are skipped,
   since the parser in Elya keeps no float's value) and 15 focused ones.
   Controls C1-C22 in the test's header: 21 caught, C14 cannot be seen
   (the dump has no spans). First run: all 358 corpus programs agreed.
   Natively the pass takes 87 ms on selfhost/lex.elya against Rust's 9 ms
   (the checker dominates, as it does for `check --dump`).
   Review: three defects. A ground type's function field kept its labels as
   written, where Rust sorts them and keeps a repeated one's last writing
   (fixed, Rust's `BTreeMap` order); a literal's characters that Rust's
   `{:?}` escapes beyond ASCII (combining marks, zero-width and other
   non-printable ones) were printed raw: such a literal is now refused
   (`a character Debug escapes`) and not compared, the characters kept
   being Latin-1 and Latin Extended (but U+00AD), punctuation to U+2027,
   arrows, box drawing and CJK ideographs; and a comment had Rust's order
   of lowering a block backwards. 3,116 random programs agreed; of 116
   mutants, 3 are equivalent and 8 more focused programs kill the 27 that
   lived (23 focused now).
2. [x] **B1** (2026-11-27): first-order `Int`/`Bool`/`Unit` programs and
   string literals printed (`selfhost/emit.elya`, importing Core in Elya).
   The harness, crates/codegen/tests/selfhost_backend.rs, builds the back
   end natively, runs it on every corpus program without imports, and for
   each it does not refuse links the IR with the runtime (`clang` takes the
   `.ll` in place of an object) and runs it beside the Rust-built program:
   standard output, standard error and the exit code compared. 35 corpus
   programs and 16 focused ones agree (the traps: overflow, zero
   divisors, `MIN / -1`; a hundred million tail calls); 669 are refused by
   name. Controls D1-D9 in the test's header, all caught (D3, D5 and D7
   after focused programs were added for them). `main`'s value is printed
   as the Rust back end prints it: an `Int` or `Unit` by `elya_main_int`, a
   `Bool` by name.
3. [x] **B2** (2026-11-27): `String` (literals, `<>`, `==`, `string.*`),
   `int.*` (wrapping, bitwise, shifts with their range stop), and rooting:
   every heap value of the scope, the operands already computed and a
   call's arguments are on the shadow stack across a call that may
   allocate. The harness also runs every Elya-built program collecting at
   every allocation (`ELYA_GC_STRESS=1`), where an unrooted value is freed
   and poisoned: that run found two faults at once (the entry's `True`/`False`
   unrooted; a constant split inside a UTF-8 character). 43
   corpus programs and 25 focused ones agree; controls E1-E7 in the
   header, E3 argued (the runtime roots its own arguments).
4. [x] **B3** (2026-11-27): ground ADTs, `match` and `==` on data, in
   the emitter itself rather than a separate pass: each arm's pattern is
   tested in order, a failed test going to the next arm, its binders bound
   as the test passes (values in SSA, so nothing is bound before its arm
   is chosen); past the last arm `elya_match_fail` (unreachable in a
   program that checks: E0430). Constructors are `[tag][fields]` from
   `elya_alloc`, a `Bool` field widened and an address `ptrtoint`, with a
   descriptor row per constructor (its pointer mask) before the string's
   and the traced array's; `==` on data is `elya_eq`. 53 corpus programs
   and 31 focused ones agree; controls F1-F10 in the header (F4, F6
   dropped: not faults the emitter can make).
5. [x] **B4** (2026-11-27): monomorphization (`selfhost/mono.elya`): a
   reference to a generic top-level function goes to a clone at the types
   it is used at (`f.mono.k`, one per instantiated signature), cloned
   bodies made ground, a variable nothing constrains `Unit`. Core in Elya
   now carries the parametric declarations (`CGeneric`, fields at
   `TVar(i)`), and the emitter makes each instance the module uses a type
   of its own, its constructors tags of their own (keyed by constructor
   and instance), found from every type the functions mention and the
   fields of each instance, to a fixpoint. A `let` whose value keeps a
   type variable is refused (Rust clones such a value per use; not done
   here). The collection run is now every allocation for the focused
   programs and every 9,973rd for the corpus, whose largest build lists of
   a million. 64 corpus programs and 36 focused ones agree; controls G2,
   G3, G5 and G6 in the header.
6. [x] **B5** (2026-11-27): closures as the Rust back end lays them out,
   `[tag][code][captures...]`, the code taking the closure first. A
   lambda's captures are the names its body reads that the scope binds, in
   order of first reading; its code is written after the function it is in
   (`@elya.lam.k`), loading them at entry. A top-level function used as a
   value is a capture-less closure over a wrapper written once
   (`@elya.wrap.f`), a constructor with fields used as a value a lambda
   building it. A call of anything but a top-level function by name loads
   the code and calls it, `musttail` in tail position. The closures' tags
   lie between the constructors' and the string's, as `elya_eq` reads
   them; their count is bounded before writing (each lambda, each mention
   of a function's name, each constructor), the unused rows empty. Keys of
   instances drop effect rows, which do not change a value's layout (a
   generic function's row variable left one in). 82 corpus programs and
   40 focused ones agree; controls H2-H4 and H6 in the header.
7. [x] **B6** (2026-11-27): `Array` (traced, with its own tag after the
   string's, when its elements are heap values; else the string's tag,
   traced nowhere, as the Rust back end makes them), `Buffer`, and `io.*`
   (files, standard input, arguments), each call that may allocate rooted.
   101 corpus programs (selfhost/lex.elya among them) and 46 focused ones
   agree; controls I1 and I3-I5 in the header.
8. [x] **B7** (2026-11-27): imports. The resolver in Elya (part 4c) gains
   `resolved` (the merged module, its text's length, whether loading or
   resolving found an error), and the back end reads its program through
   it; two of its types were renamed (`RvLS`, `RvRS`) since type names are
   one namespace across a program. The harness no longer skips programs
   with imports: 110 corpus programs and 46 focused ones agree.
9. [x] **B8** (2026-11-27): the self-hosted sources. Each of `lex`,
   `parse`, `exhaust`, `affine`, `check`, `core`, `resolve` and `emit` is
   built by the Rust back end and by the back end in Elya, and the two run
   on the repository's Elya files (the examples, the self-hosted sources,
   the standard modules, tests/ui): 327 runs agree. The back end is among
   them: built by itself it writes the same IR as built by Rust, on its own
   source too, so the back end has reached its fixed point at the IR text.
   Elya-built tools run about as fast as Rust-built ones (`check` on
   check.elya: 1.52 s against 1.73 s; `emit` on check.elya: 10.3 s against
   13.4 s; both through clang).
