// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
// Programs for `&&` and `||` short-circuiting (spec
// 2026-11-15-elya-short-circuit), shared by tests/short_circuit.rs and
// crates/codegen/tests/native_short_circuit.rs.

/// The right operand runs only when it decides the answer: a division by
/// zero on the right of a `False &&` or a `True ||` is never reached.
pub const SKIPPED: &str = "fn crash(n: Int) -> Bool { 10 / n == 1 }
pub fn main() -> Int {
  let a = if False && crash(0) { 1 } else { 2 }
  let b = if True || crash(0) { 10 } else { 20 }
  let c = if True && crash(10) { 100 } else { 200 }
  let d = if False || crash(5) { 1000 } else { 2000 }
  a + b + c + d
}
";
pub const SKIPPED_VALUE: &str = "2112";

/// Effects on the right happen only when it runs, after the left's.
pub const ORDER: &str = "import elya/io
fn say(s: String, b: Bool) / {IO} -> Bool {
  io.println(s)
  b
}
pub fn main() / {IO} {
  let x = say(\"a\", False) && say(\"b\", True)
  let y = say(\"c\", True) || say(\"d\", True)
  let z = say(\"e\", True) && say(\"f\", False)
  if x || y && !z { io.println(\"g\") } else { io.println(\"h\") }
}
";
pub const ORDER_OUT: &str = "a\nc\ne\nf\ng\n";

/// The guard Python and C programmers write: the index is checked before
/// the element is read, so the read past the end never happens.
pub const GUARD: &str = "fn first_zero(borrow xs: Array(Int), i: Int) -> Int {
  if i < array.length(xs) && array.get(xs, i) != 0 { first_zero(xs, i + 1) } else { i }
}
pub fn main() -> Int {
  let xs = array.push(array.push(array.push(array.new(), 1), 2), 3)
  let n = first_zero(xs, 0)
  n * 10 + array.length(xs)
}
";
pub const GUARD_VALUE: &str = "33";

/// The right operand is in tail position: a loop written as `||` runs in
/// constant stack, as the same loop written with `if` does.
pub const TAIL: &str = "fn reaches(n: Int) -> Bool { n == 0 || reaches(n - 1) }
fn never(n: Int) -> Bool { n != 0 && never(n - 1) }
pub fn main() -> Int {
  let a = if reaches(1000000) { 1 } else { 0 }
  let b = if never(1000000) { 10 } else { 0 }
  a + b
}
";
pub const TAIL_VALUE: &str = "1";

/// An effect performed on the right, under a handler resuming twice: the
/// right runs only on the path where the first flip answered `False`, so the
/// count is 1 + (1 + 0) = 2. Read strictly, both flips run on every path:
/// (1 + 1) + (1 + 0) = 3.
pub const MULTI: &str = "effect multi Flip { fn flip() -> Bool }
pub fn main() -> Int {
  handle { if flip() || flip() { 1 } else { 0 } } with multi { Flip.flip() -> resume(True) + resume(False) }
}
";
pub const MULTI_VALUE: &str = "2";
