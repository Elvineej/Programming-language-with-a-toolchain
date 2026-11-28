// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! `io.os()` in the evaluator (spec
//! `docs/superpowers/specs/2026-11-28-elya-io-os-design.md` D1, D2): the
//! platform's name in the native runtime's words, recorded and replayed.

const PROGRAM: &str = "pub fn main() / {IO} { io.println(io.os()) }\n";

/// What this platform answers, as `elya_platform_os` does.
fn here() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "unix"
    }
}

#[test]
fn io_os_names_the_platform() {
    let o = elya::run_source_outcome("main.elya", PROGRAM);
    assert!(o.error.is_none(), "{:?}", o.error);
    assert_eq!(o.output, format!("{}\n", here()));
}

#[test]
fn io_os_is_recorded_and_replayed() {
    let (trace, o) = elya::record_source("main.elya", PROGRAM);
    assert!(o.error.is_none(), "{:?}", o.error);
    let trace = trace.expect("a trace");
    assert!(
        trace.contains(&format!("io.os() = \"{}\"", here())),
        "{trace}"
    );
    let r = elya::replay_source(
        "main.elya",
        PROGRAM,
        "elya trace 1\nio.os() = \"windows\"\n",
    );
    assert_eq!((r.output.as_str(), r.error), ("windows\n", None));
}
