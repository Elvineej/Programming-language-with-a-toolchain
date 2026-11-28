# The switch: Elya built by itself (self-hosting part 6)

2026-11-27. Status: decided, not built. R3 item 6 of
`2026-11-14-elya-self-hosting-design.md`: "`elya` built by the Rust compiler
from the Elya sources, then by itself". Design choices are Claude's (rule 2).

## 0. Measured first

Part 5's stage B8 (plan `2026-11-27-elya-backend-in-elya.md`) built every
self-hosted source twice, by the Rust back end and by the back end in Elya,
and the builds agree on 327 runs over the repository's Elya files. The back
end built by itself writes the same IR as built by Rust, its own source
included. What is missing for a compiler that builds itself:

- a driver: nothing in Elya reads `elya build <file> -o <out>`, runs the
  front end's passes in order, stops at an error, and writes a program;
- running `clang`: no Elya program can start another program (R3 item 1 put
  `io.run` off to here);
- the front end's last stage, F6 (the grants of imports, E0606), is not in
  the checker in Elya, and Rust's diagnostics are rendered (source lines,
  labels, help) where the Elya passes print the dump form.

## 1. Decisions

**D1. `io.run(command: String) -> Int`, performing `IO`.** The command's
lines are the program and its arguments (`"clang\nprog.ll\n-o\nprog"`); an
argument cannot hold a newline, which a compiler's command lines never need.
It answers the program's exit status (on Unix a program stopped by a signal
answers 128 plus the signal, as a shell does). The program inherits the
standard streams; Elya's own output is flushed first, so the two stay in
order. Failing by name, status 1: `elya: cannot run `<program>`: <reason>`
(not found, permission denied, a NUL byte in the command, an empty
command). Recorded and replayed like the other `io.*`: `io.run("...") = 0`,
a replay running nothing. In the evaluator a program's own output is printed when its
run ends, as it always has been, so a child's output comes before it there;
natively it comes in order. Natively a platform hook,
`elya_platform_run(const char *const *argv)`: `posix_spawnp` and `waitpid`
hosted on Unix, `_wspawnvp` over the UTF-8 arguments on Windows, a board's
default failing (`this platform cannot run programs`). Its twins: the
resolver's, checker's, Core's and back end's tables in Elya.

**D2. The driver, `selfhost/elya.elya`.** `elya build <file> [-o <out>]
[--runtime <dir>]`: the resolver in Elya, then the checker in Elya, then
exhaustiveness and the affine facts as the checker reads them; any error
printed in the dump form (`error <code> <start> <end> "<message>"`) and
status 1; else Core, monomorphization and the back end, the IR written beside
the output, and `clang` run on it with the runtime (`<dir>/runtime.c`, the
Rust back end's flags: `-O2`, `-no-pie` on Linux, `-pthread` on Unix). A
program outside Elya-S is refused by name, status 1. Only `build`: the
evaluator stays Rust's (`elya run`), and so does `check`'s rendering, until
a renderer is written in Elya (not decided here).

**D3. The fixed point, by stages.** Stage 1: the Rust compiler builds
`selfhost/elya.elya`. Stage 2: stage 1 builds it. Stage 3: stage 2 builds it.
Stage 2 and stage 3 must write the same IR text, byte for byte (the usual
test; part 5 has already met it for the back end alone). Each stage builds a
sample of the corpus to programs that behave as the Rust-built ones do.

**D4. Before Rust is retired** (R1): every test agrees, which needs F6, a
diagnostic renderer in Elya, and the evaluator (or a decision that native
`build` is the only way to run). Not decided here; the switch makes Elya able
to build Elya, not yet the only compiler.

## 2. What is not decided here

How the driver finds the runtime by default (an argument for now), the
renderer, and whether `io.run` grows a way to capture a program's output
(a compiler driver does not need it).

## 3. Built (2026-11-27)

`io.run` (D1) in both back ends and every Elya twin's table, with
tests/io_run.rs (the evaluator: statuses, the three failures) and
crates/codegen/tests/native_io_run.rs (statuses with the child's line in
order, the failures in the same words, a record and a replay that runs
nothing). The driver (D2) is `selfhost/elya.elya`; `main`'s value is its
status (0 built, 1 not); since spec 2026-11-28-elya-io-exit it ends with
that status (`io.exit`), 2 on a usage error. **The fixed point (D3)
holds**: crates/codegen/tests/selfhost_switch.rs builds the driver with the
Rust compiler (stage 1, 25 s), stage 1 builds it (stage 2, 86 s), stage 2
builds it again (stage 3, 68 s), and stages 2 and 3 write the same IR, byte
for byte (11.2 MB); the binaries differ only in what `clang` records of the
IR file's name. Stages 2 and 3 build `examples/01`-`03` to programs that
behave as the Rust-built ones do, and refuse the others (handlers) by name.
Stage 2 rejects a program with a type error (E0400), a non-exhaustive match
(E0430), an affine value used twice (E0428) or `==` on functions (E0436),
printing the error in the dump form and building nothing.
