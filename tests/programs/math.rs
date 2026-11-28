// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! Shared programs and expected results for the evaluator and native math suites.

use std::fmt::Write;

const IMPORT: &str = "import elya/math\n";

fn cases<'a>(
    setup: &str,
    show: impl Fn(&str) -> String,
    rows: impl IntoIterator<Item = (&'a str, i64)>,
) -> (String, String) {
    let mut source = format!("{IMPORT}pub fn main() / {{IO}} -> Int {{\n{setup}");
    let mut expected = String::new();
    for (expr, value) in rows {
        writeln!(source, "  io.println(int.to_string({}))", show(expr)).unwrap();
        writeln!(expected, "{value}").unwrap();
    }
    source.push_str("  0\n}\n");
    (source, expected)
}

fn int_cases() -> (String, String) {
    cases(
        "  let low = -9223372036854775808\n  let high = 9223372036854775807\n",
        str::to_owned,
        [
            ("math.abs(0)", 0),
            ("math.abs(-1)", 1),
            ("math.abs(high)", i64::MAX),
            ("math.min(low, high)", i64::MIN),
            ("math.min(1, -1)", -1),
            ("math.max(low, high)", i64::MAX),
            ("math.max(1, -1)", 1),
            ("math.clamp(low, -1, 1)", -1),
            ("math.clamp(high, -1, 1)", 1),
            ("math.clamp(0, -1, 1)", 0),
            ("math.clamp(1, -1, 1)", 1),
            ("math.clamp(low, low, low)", i64::MIN),
            ("math.sign(low)", -1),
            ("math.sign(0)", 0),
            ("math.sign(high)", 1),
            ("math.gcd(0, 0)", 0),
            ("math.gcd(-42, 0)", 42),
            ("math.gcd(0, -42)", 42),
            ("math.gcd(-54, 24)", 6),
            ("math.gcd(54, -24)", 6),
            ("math.gcd(-54, -24)", 6),
            ("math.gcd(low, high)", 1),
            ("math.gcd(-1, low)", 1),
            ("math.gcd(low, 4)", 4),
            ("math.lcm(-21, 6)", 42),
            ("math.lcm(-21, -6)", 42),
            ("math.lcm(0, low)", 0),
            ("math.lcm(low, 0)", 0),
            ("math.lcm(high, high)", i64::MAX),
            ("math.pow(0, 0)", 1),
            ("math.pow(0, high)", 0),
            ("math.pow(-1, high)", -1),
            ("math.pow(-3, 3)", -27),
            ("math.pow(low, 1)", i64::MIN),
            ("math.pow(2, 62)", 1 << 62),
            ("math.pow(-2, 63)", i64::MIN),
            ("math.isqrt(0)", 0),
            ("math.isqrt(1)", 1),
            ("math.isqrt(2)", 1),
            ("math.isqrt(15)", 3),
            ("math.isqrt(16)", 4),
            ("math.isqrt(18014398777917441)", 134217729),
            ("math.isqrt(18014398777917440)", 134217728),
            ("math.isqrt(9223372030926249001)", 3037000499),
            ("math.isqrt(9223372030926249000)", 3037000498),
            ("math.isqrt(high)", 3037000499),
            ("math.div_floor(7, 3)", 2),
            ("math.div_floor(-7, 3)", -3),
            ("math.div_floor(7, -3)", -3),
            ("math.div_floor(-7, -3)", 2),
            ("math.div_floor(6, -3)", -2),
            ("math.div_floor(0, -3)", 0),
            ("math.div_floor(low, 1)", i64::MIN),
            ("math.div_floor(high, low)", -1),
            ("math.mod_floor(7, 3)", 1),
            ("math.mod_floor(-7, 3)", 2),
            ("math.mod_floor(7, -3)", -2),
            ("math.mod_floor(-7, -3)", -1),
            ("math.mod_floor(-6, 3)", 0),
            ("math.mod_floor(0, -3)", 0),
            ("math.mod_floor(low, -1)", 0),
            ("math.mod_floor(1, low)", i64::MIN + 1),
        ],
    )
}

fn float_cases() -> (String, String) {
    let nan = f64::from_bits(0x7ff8_0000_0000_0123);
    cases(
        "  let inf = float.of_bits(9218868437227405312)\n  let nan = float.of_bits(9221120237041090851)\n",
        |expr| format!("float.to_bits({expr})"),
        [
            ("math.fabs(-2.5)", 2.5),
            ("math.fabs(-0.0)", 0.0),
            ("math.fabs(-.inf)", f64::INFINITY),
            ("math.fabs(-.nan)", nan),
            ("math.fmin(-2.5, 1.0)", -2.5),
            ("math.fmin(1.0, -2.5)", -2.5),
            ("math.fmin(0.0, -0.0)", -0.0),
            ("math.fmin(-0.0, 0.0)", -0.0),
            ("math.fmin(nan, 1.0)", 1.0),
            ("math.fmin(1.0, nan)", 1.0),
            ("math.fmin(nan, -.nan)", nan),
            ("math.fmax(-2.5, 1.0)", 1.0),
            ("math.fmax(1.0, -2.5)", 1.0),
            ("math.fmax(0.0, -0.0)", 0.0),
            ("math.fmax(-0.0, 0.0)", 0.0),
            ("math.fmax(nan, 1.0)", 1.0),
            ("math.fmax(1.0, nan)", 1.0),
            ("math.fmax(nan, -.nan)", nan),
            ("math.floor(-0.5)", -1.0),
            ("math.ceil(-0.5)", -0.0),
            ("math.floor(-1.0)", -1.0),
            ("math.ceil(-1.0)", -1.0),
            ("math.floor(2.5)", 2.0),
            ("math.ceil(2.5)", 3.0),
            ("math.floor(-0.0)", -0.0),
            ("math.ceil(-0.0)", -0.0),
            ("math.floor(0.0)", 0.0),
            ("math.ceil(0.0)", 0.0),
            ("math.floor(0.5)", 0.0),
            ("math.ceil(0.5)", 1.0),
            ("math.floor(4503599627370495.5)", 4503599627370495.0),
            ("math.ceil(4503599627370495.5)", 4503599627370496.0),
            ("math.floor(1.0e300)", 1.0e300),
            ("math.ceil(1.0e300)", 1.0e300),
            ("math.floor(-1.0e300)", -1.0e300),
            ("math.ceil(-1.0e300)", -1.0e300),
            ("math.floor(inf)", f64::INFINITY),
            ("math.ceil(inf)", f64::INFINITY),
            ("math.floor(-.inf)", f64::NEG_INFINITY),
            ("math.ceil(-.inf)", f64::NEG_INFINITY),
            ("math.floor(nan)", nan),
            ("math.ceil(nan)", nan),
        ]
        .map(|(expr, value)| (expr, value.to_bits() as i64)),
    )
}

fn classification_cases() -> (String, String) {
    cases(
        "  let nan = float.of_bits(-4503599627370495)\n  let inf = float.of_bits(9218868437227405312)\n  let tiny = float.of_bits(1)\n",
        |expr| format!("if {expr} {{ 1 }} else {{ 0 }}"),
        [
            ("math.is_nan(nan)", 1),
            ("math.is_nan(inf)", 0),
            ("math.is_nan(-0.0)", 0),
            ("math.is_infinite(nan)", 0),
            ("math.is_infinite(inf)", 1),
            ("math.is_infinite(-.inf)", 1),
            ("math.is_infinite(1.0e300)", 0),
            ("math.is_finite(nan)", 0),
            ("math.is_finite(inf)", 0),
            ("math.is_finite(-.inf)", 0),
            ("math.is_finite(1.0e300)", 1),
            ("math.is_finite(tiny)", 1),
            ("math.is_finite(-0.0)", 1),
        ],
    )
}

pub fn programs() -> [(String, String); 3] {
    [int_cases(), float_cases(), classification_cases()]
}

pub const FAILURES: &[(&str, &str)] = &[
    ("math.abs(-9223372036854775808)", "integer overflow"),
    ("math.gcd(-9223372036854775808, 0)", "integer overflow"),
    ("math.lcm(9223372036854775807, 2)", "integer overflow"),
    ("math.pow(2, 63)", "integer overflow"),
    (
        "math.div_floor(-9223372036854775808, -1)",
        "integer overflow",
    ),
    ("math.div_floor(1, 0)", "division by zero"),
    ("math.mod_floor(1, 0)", "remainder by zero"),
];

pub fn failure_program(expr: &str) -> String {
    format!("{IMPORT}pub fn main() -> Int {{ {expr} }}\n")
}
