# Retiring Rust: what is left, in order

2026-11-28. Status: decided, not built. The road's R1 (spec
`2026-11-14-elya-self-hosting-design.md`) and the switch's D4 (spec
`2026-11-27-elya-the-switch-design.md`): Rust stays the bootstrap compiler
until an Elya compiler compiles itself **and every test agrees**. The first
half holds since part 6 (the fixed point). This spec orders the second half.
Design choices are Claude's (rule 2).

## 0. Measured first

On `main` at 1150a80, the compiler in Elya (`selfhost/elya.elya`) builds the
Elya-S programs: Int, Bool, Unit, String, data and `match`, closures, arrays,
buffers, `io.*` (with `io.run`). What the tests hold that it does not do:

| what | where it stops today |
|---|---|
| `Float` | the lexer in Elya leaves float literals out (E0002 is not reported: Elya could not parse a float), Core in Elya refuses them, the back end has no float operations |
| effects and handlers | the back end in Elya refuses `handle`, `resume` and every operation by name |
| E0606 (the grants of imports) | the checker in Elya's stage F6: Codex's job since 2026-11-27 |
| rendered diagnostics | the passes in Elya print the dump form (`error <code> <start> <end> "<message>"`); `elya check` renders source lines, labels and help |
| an exit status | the driver's `main` returns its status as a value, printed as every native program's `Int` is; nothing ends a program with a status of its choosing |
| platform flags | the driver passes `clang` Unix's flags (`-no-pie`, `-pthread`) |
| `elya run`, `elya test`, record and replay | the evaluator, Rust's |

## 1. Decisions

**T1. Order, cheapest dependency first:** (1) an exit status (`io.exit`),
then the driver's platform flags; (2) F6 (Codex, under way); (3) the
diagnostic renderer in Elya (a Codex job: it is self-contained, compared byte
for byte with `elya check`'s output); (4) `Float` through the front end in
Elya (a float literal's value needs a decimal-to-binary conversion: a builtin
`float.parse`, the runtime's correctly rounded `float_parse.c` and its Rust
twin, rather than one written in Elya now) and the back end (bit casts and
the runtime's `elya_float_to_string`, as the Rust back end does); (5) effects
and handlers in the back end in Elya, the largest (the Rust back end's CPS
partition and emitter, `cps.rs` and `cps_emit.rs`, about 5,000 lines);
(6) the evaluator.

**T2. The evaluator is decided last, by measurement.** `elya run` today
interprets; its oracle role in the tests (native must print what the
evaluator prints) is what keeps both back ends honest. Before (6) the options
are an evaluator in Elya, or `run` as build-and-execute with the evaluator
kept as a test oracle only. Not decided here.

**T3. Every step lands as the others did:** a spec, red tests, negative
controls, an independent review, the gate at its predicted count, Windows
green, then `main`.
