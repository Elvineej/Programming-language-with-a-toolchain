// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! The programs of `tests/io_exit.rs`, shared with
//! `crates/codegen/tests/native_io_exit.rs` (no tests here): `io.exit` (spec
//! `docs/superpowers/specs/2026-11-28-elya-io-exit-design.md`).

/// Exits with 3 between two lines: the second is never written.
pub const MIDDLE: &str =
    "pub fn main() / {IO} {\n  io.println(\"before\")\n  io.exit(3)\n  io.println(\"after\")\n}\n";

/// Exits with 0 deep in a non-tail recursion: main has no value to print.
pub const DEEP: &str = "fn f(n: Int) / {IO} -> Int { if n == 0 { io.exit(0)\n 0 } else { 1 + f(n - 1) } }\npub fn main() / {IO} -> Int {\n  io.println(\"x\")\n  f(5)\n}\n";

/// Exits with 7 from a handler's clause, before resuming.
pub const HANDLER: &str = "effect Ask { fn ask() -> Int }\npub fn main() / {IO} -> Int {\n  io.println(\"in\")\n  handle { ask() + 1 } with { Ask.ask() -> { io.exit(7)\n resume(1) } }\n}\n";

/// A status outside 0..255, each way.
pub const HIGH: &str = "pub fn main() / {IO} {\n  io.println(\"x\")\n  io.exit(256)\n}\n";
pub const HIGH_ERR: &str = "elya: io.exit: status 256 is outside 0..255\n";
pub const LOW: &str = "pub fn main() / {IO} {\n  io.exit(0 - 1)\n}\n";
pub const LOW_ERR: &str = "elya: io.exit: status -1 is outside 0..255\n";
