# Running Elya on QEMU boards

An Elya program builds for two boards as well as for the machine it is built
on: a Cortex-M3 (QEMU's `mps2-an385`) and a 32-bit RISC-V (QEMU's `virt`,
`rv32imac`). The program, the runtime and the board's start-up are linked with
no C library; the program writes what it writes on a host, and stops with the
same status.

## What you need

- An `elya` built with the `boards` feature, which needs an LLVM 18 with the
  ARM and RISC-V back ends (the official LLVM release has them; a vcpkg LLVM
  built for x86 alone does not):

  ```sh
  cargo build -p elya-cli --release --features boards
  ```

- `clang` and `ld.lld` (version 18).
- For the Cortex-M3, `arm-none-eabi-gcc`; for RISC-V,
  `riscv64-unknown-elf-gcc` (or `riscv32-unknown-elf-gcc`) with the
  `rv32imac/ilp32` multilib. Only their `libgcc` is used: 64-bit division and
  soft floats.
- To run the result, `qemu-system-arm` or `qemu-system-riscv32`.

`ELYA_CLANG`, `ELYA_LLD`, `ELYA_ARM_GCC` and `ELYA_RISCV_GCC` name a tool when
it is not on `PATH` under its usual name. A missing tool is an error naming it.

## Build and run

```sh
elya build prog.elya --target cortex-m3          # writes prog.elf
qemu-system-arm -M mps2-an385 -display none -serial none -monitor none \
  -semihosting-config enable=on,target=native -kernel prog.elf

elya build prog.elya --target riscv32 -o prog-rv.elf
qemu-system-riscv32 -M virt -bios none -display none -serial none -monitor none \
  -semihosting-config enable=on,target=native -kernel prog-rv.elf
```

`--target host` is the ordinary build. Do not run QEMU with `-nographic`: it
makes QEMU's standard output non-blocking, and a write to a full pipe then
fails (the program stops with status 70).

## What a program sees

- Standard output and standard error, through semihosting; its exit status, as
  on a host.
- A clock: `time.now_ms` counts from the MPS2 FPGA's 100 Hz counter on the
  Cortex-M3 (steps of 10 ms) and from `mtime` on RISC-V; `time.sleep_ms` waits
  on it.
- No files, no arguments, no input, no environment variables, no settings
  (`ELYA_GC_STRESS` and the like are a host's): `io.file_exists` answers
  `False`, `io.arg_count()` 0, `io.read_stdin()` the empty string, and
  `io.read_file` and `io.write_file` stop with
  `elya: cannot read `<path>`: this platform has no files`, status 1.
- A recursion deeper than the stack stops with
  `elya: native stack overflow: recursion deeper than the 1024 KiB stack`,
  status 1, as on a host: a 4 KiB guard below the stack (the MPU on the
  Cortex-M3, a locked PMP entry on RISC-V) catches it before it reaches
  anything else. Any other fault is named with where it was
  (`elya: hard fault at pc ...` or `elya: trap mcause ...`), status 70.

## Memory

| | Cortex-M3 (`mps2-an385`) | RISC-V (`virt`) |
|---|---|---|
| Code | 4 MiB at 0 | in RAM, first |
| RAM | 4 MiB at `0x20000000` | 128 MiB at `0x80000000` |
| In RAM, in order | guard, stack, data, fault stack, heap | code, guard, stack, data, trap stack, heap |
| Stack | 1 MiB | 1 MiB |
| Heap | the rest of RAM | the rest of RAM |

The board files are in `crates/codegen/boards/` (`cortex-m3/`, `riscv32/`, and
`common/mem.c`); `elya_codegen::link_for` links with them.

## Limits

- Floats are soft (no FPU is used).
- On RISC-V every Elya function takes twelve 32-bit words of arguments; one
  needing more (a handler clause for an operation of six Ints) is refused by
  name (spec `2026-11-14-elya-riscv-board`).
- On the Cortex-M3 some calls are padded to work around an LLVM 18 bug (spec
  `2026-11-13-elya-boards`, D4).
- The stack and the heap sizes are the link script's (`ELYA_STACK_BYTES`,
  `ELYA_HEAP_BYTES` as `--defsym`); `elya build` does not set them yet.
