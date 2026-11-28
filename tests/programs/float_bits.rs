// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! The programs of `tests/float_bits.rs`, shared with
//! `crates/codegen/tests/native_float_bits.rs` (no tests here):
//! `float.parse`, `float.to_bits`, `float.of_bits` (spec
//! `docs/superpowers/specs/2026-11-28-elya-float-in-elya-design.md` D1).

/// Texts `float.parse` takes, with the bits of their nearest double.
pub const ACCEPTED: &[&str] = &[
    "0",
    "-0.0",
    "1.5",
    "0.1",
    "1_000.125e-2",
    "2.5e-310",
    "4.9e-324",
    "2.4703282292062327e-324",
    "1.7976931348623157e308",
    "1.7976931348623159e308",
    "1e400",
    "123456789012345678901234567890",
    "9007199254740993",
    "1E+5",
];

/// Texts it refuses.
pub const REFUSED: &[&str] = &[
    "", "-", ".5", "5.", "1e", "1_", "_1", "1__0", "1x", "+1", "inf", "nan", "1.5.2",
];

/// A program printing each accepted text's bits, then the bits read back.
pub fn bits_program() -> String {
    let mut body = String::new();
    for t in ACCEPTED {
        body.push_str(&format!(
            "  io.println(int.to_string(float.to_bits(float.parse(\"{t}\"))))\n"
        ));
    }
    body.push_str("  io.println(float.to_string(float.of_bits(9221120237041090561)))\n");
    body.push_str("  io.println(int.to_string(float.to_bits(float.of_bits(-1))))\n");
    format!("pub fn main() / {{IO}} {{\n{body}}}\n")
}

/// What [`bits_program`] prints: Rust's own nearest doubles.
pub fn bits_out() -> String {
    let mut out = String::new();
    for t in ACCEPTED {
        let x: f64 = t.replace('_', "").parse().expect("Rust reads it");
        out.push_str(&format!("{}\n", x.to_bits() as i64));
    }
    out.push_str("nan\n-1\n");
    out
}

/// A program parsing `text`.
pub fn refused_program(text: &str) -> String {
    format!("pub fn main() -> Float {{ float.parse(\"{text}\") }}\n")
}
