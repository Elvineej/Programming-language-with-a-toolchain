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
5. [ ] **B4**: monomorphization (`selfhost/mono.elya`).
6. [ ] **B5-B8**: closures; arrays, buffers, `io.*`; imports (after part
   4c); the self-hosted sources themselves.
