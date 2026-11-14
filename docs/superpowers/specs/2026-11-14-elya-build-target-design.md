# `elya build --target`

**Status:** done (2026-11-14, on `main`, an interactive session). The boards
(specs 2026-11-13-elya-boards, 2026-11-14-elya-riscv-board) were reachable only
from the library and the tests; this is the command. Design choices are
Claude's (rule 2), from ChatGPT's design (section 3).

## 0. Measured first

`elya build <file> [-o <out>]` (`crates/cli/src/main.rs`) took `-o` only right
after the file, and built for the host. A board's link lived in the board test
alone: building the runtime and the board's files with `clang`, finding the
board's `libgcc` through its GCC, `ld.lld` with the link script.

## 1. Decisions

**D1. The flag.** `elya build <file.elya> [-o <out>] [--target <machine>]`, the
flags in any order, each at most once; the machines are `host` (the default),
`cortex-m3` and `riscv32`. A board's output is an ELF, `<stem>.elf` by default.
An unknown machine, a flag twice or without its value, or a second file is a
usage error (status 2) before the source is read. A board named to an `elya`
built without the `boards` feature (`elya-cli`'s, which turns on
`elya-codegen`'s) is refused by name, saying how to build one. Success prints
`built <path>`, as for the host; the QEMU line is in `docs/boards.md` rather
than printed (no `elya run --target`: running is QEMU's, and a second way to
run a program would need its own contract for timeouts and status).

**D2. The link lives in the library.** `elya_codegen::Board` (`CortexM3`,
`Riscv32`: `named`, `name`, `machine`, `qemu_command`) and
`elya_codegen::link_for(obj, exe, board)`. A board's pieces (the runtime, the
memory functions, the board's hooks and start-up, each built for its
processor, and the path of its `libgcc`) are built at a process's first link
for that board, in a directory the process creates, and kept, as the host's
runtime object is. Tools are found on `PATH` (`clang` or `clang-18`, `ld.lld`
or `ld.lld-18`, `arm-none-eabi-gcc`, `riscv64-unknown-elf-gcc` or
`riscv32-unknown-elf-gcc`), or named by `ELYA_CLANG`, `ELYA_LLD`,
`ELYA_ARM_GCC`, `ELYA_RISCV_GCC` (that one only, when set); a missing one is
`codegen: board '<name>': needs `<tool>` on PATH (tried ...; or set ...)`, a
GCC with no library for the processor (it answers a bare `libgcc.a`) is named
too. The board test now links through `link_for`: the command and the tests
cannot drift apart.

**D3. Not now.** Stack and heap sizes on the command line (the link scripts take
them as `--defsym`); an installed `elya` that carries its runtime and board
sources inside it (today both the host's and the boards' are read from the
source tree, `CARGO_MANIFEST_DIR`).

## 2. Tests

`build_cli.rs` +2 (configuration B): the flag is checked before anything is
read (an unknown machine named with the list, a flag twice, a flag with no
value, two files; `--target host` builds and runs); a board without the
feature is refused by name. `build_target.rs` (new, 2, configuration C): an
ELF for each board, 32-bit, `EM_ARM` and `EM_RISCV`, that QEMU runs writing
what the program means, the default name and `-o`, the flags in both orders; a
tool that does not run (`ELYA_LLD`) named, status 1, no ELF. `boards.rs` +1
(unit): the names. `native_boards.rs` links through `link_for`; the timers
example there is run again (twice at most) before a difference fails, its
real-clock half being timing-dependent (HANDOFF, "Also open").

**Controls** (each reverted, compared byte for byte), each failing its test:
K1 `ELYA_LLD` ignored (the missing tool is not named); K2 a board's output not
`.elf` (the default name differs); K3 `BOARDS` true without the feature (the
refusal is not printed).

## 3. ChatGPT's design, checked

Asked for the design, ChatGPT read the CLI, `link`, `runtime_object` and the
board test and proposed the flag, `link_for` in the library, one cache per
board, the tool overrides, no stack or heap options yet, and the tests (an
ELF's class and machine checked, not only its name). Kept, except: it would
have printed nothing for QEMU and gated `Board` behind the feature (here
`Board` and `link_for` are always there, and the CLI asks
`elya_codegen::BOARDS`, so a build without the feature can say what is
missing); it would have embedded the sources in the binary (D3: the host's are
not either, so both move together, later). It also wrote the first draft of
`docs/boards.md`, rewritten here for the command.

## 4. Predictions

Gate: configurations A and B, 1533 + 3 = **1536 passed, 166 suites**
(`build_target.rs` is an empty suite in each); configuration C, 5 + 2 =
**7 passed, 2 suites**. No existing expectation changes.
