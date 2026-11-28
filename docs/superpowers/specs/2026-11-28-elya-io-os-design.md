# `io.os`: the platform a program runs on

2026-11-28. Status: decided. Step (1) of spec
`2026-11-28-elya-retiring-rust-design.md` T1, after `io.exit`. Design choices
are Claude's (rule 2).

## 0. Measured first

The driver in Elya passes `clang` Linux's flags (`-no-pie`, `-pthread`) on
every platform, so the switch test runs on Unix only. The Rust back end
chooses its flags when Rust compiles it (`#[cfg]` in `elya_codegen::link`);
an Elya program has no such moment, so it must ask.

## 1. Decisions

**D1. `io.os() -> String`, performing `IO`**: `"linux"`, `"macos"`,
`"windows"`, `"unix"` (another Unix) or `"none"` (a board, or a platform the
runtime does not know).

**D2. Recorded and replayed** as the other questions are
(`io.os() = "linux"`): a replay answers what the trace says, so a run
recorded on one platform replays the same on another.

**D3. Natively** a platform hook, `elya_platform_os()`: the hosted one by the
C compiler's platform macros, a board's default `"none"`; `elya_io_os` in
the runtime records and replays it. The evaluator answers from Rust's
`std::env::consts::OS` in the same words.

**D4. The driver** passes `clang` the Rust back end's flags by platform:
`-no-pie -pthread` on Linux, `-pthread` on another Unix, none on Windows;
the switch test then runs on every platform.
