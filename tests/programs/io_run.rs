// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! The programs of `tests/io_run.rs`, shared with
//! `crates/codegen/tests/native_io_run.rs` (no tests here): `io.run` (spec
//! `docs/superpowers/specs/2026-11-27-elya-the-switch-design.md` D1). A
//! command is the platform's shell running a script, so the tests run on
//! Linux, macOS and Windows alike.

/// The command running `script` in the platform's shell, as `io.run` takes it.
pub fn shell(script: &str) -> String {
    if cfg!(windows) {
        format!("cmd\\n/c\\n{script}")
    } else {
        format!("sh\\n-c\\n{script}")
    }
}

/// A program printing the statuses of `exit 0` and `exit 3`, and of a script
/// that writes a line of its own between two of the program's.
pub fn statuses() -> String {
    format!(
        "pub fn main() / {{IO}} {{\n  io.println(int.to_string(io.run(\"{}\")))\n  io.println(int.to_string(io.run(\"{}\")))\n  io.println(\"before\")\n  let r = io.run(\"{}\")\n  io.println(\"after \" <> int.to_string(r))\n}}\n",
        shell("exit 0"),
        shell("exit 3"),
        shell("echo child"),
    )
}

/// What [`statuses`] prints natively, where the child's line comes in order.
pub const STATUSES_OUT: &str = "0\n3\nbefore\nchild\nafter 0\n0\n";

/// A program running something that is not there.
pub const MISSING: &str = "pub fn main() / {IO} { io.println(\"x\")\n let r = io.run(\"elya-no-such-program-here\")\n io.println(int.to_string(r)) }\n";
pub const MISSING_ERR: &str =
    "elya: cannot run `elya-no-such-program-here`: no such file or directory\n";

/// An empty command, and one holding a NUL byte.
pub const EMPTY: &str = "pub fn main() -> Int { io.run(\"\") }\n";
pub const EMPTY_ERR: &str = "elya: cannot run ``: an empty command\n";
/// A raw NUL byte in the command (an Elya string can hold one).
pub const NUL: &str = "pub fn main() -> Int { io.run(\"sh\u{0}x\") }\n";
pub const NUL_ERR: &str = "elya: cannot run `sh\u{0}x`: a command cannot hold a NUL byte\n";
