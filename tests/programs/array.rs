//! The programs of `tests/array.rs`, shared with
//! `crates/codegen/tests/native_array.rs` (no tests here).

/// Arrays of each kind of element (heap values the collector follows, and
/// words it does not), pushed across growths, set, read by borrowing, through
/// generic functions and a handler (CPS code natively); a buffer's length.
pub const ARRAYS: &str = r#"type Pair { Pair(String, Int) }
effect Note { fn note(s: String) -> Unit }
fn pairs(xs: Array(Pair), i: Int, n: Int) -> Array(Pair) {
  if i == n { xs } else { pairs(array.push(xs, Pair("k" <> int.to_string(i), i)), i + 1, n) }
}
fn total(borrow xs: Array(Pair), i: Int, acc: Int) -> Int {
  if i == array.length(xs) { acc } else {
    match array.get(xs, i) { Pair(s, n) -> total(xs, i + 1, acc + n + string.length(s)) }
  }
}
fn squares(xs: Array(Int), i: Int, n: Int) -> Array(Int) {
  if i == n { xs } else { squares(array.push(xs, i * i), i + 1, n) }
}
fn sum(borrow xs: Array(Int), i: Int, acc: Int) -> Int {
  if i == array.length(xs) { acc } else { sum(xs, i + 1, acc + array.get(xs, i)) }
}
fn last(borrow xs: Array(a)) -> a { array.get(xs, array.length(xs) - 1) }
fn filled(xs: Array(a), x: a, n: Int) -> Array(a) {
  if n == 0 { xs } else { filled(array.push(xs, x), x, n - 1) }
}
fn doubled(xs: Array(Int), i: Int) -> Array(Int) {
  if i == array.length(xs) { xs } else {
    doubled(array.set(xs, i, array.get(xs, i) + array.get(xs, i)), i + 1)
  }
}
fn noted(xs: Array(String)) -> Array(String) {
  let _ = note(last(xs))
  array.push(xs, "after")
}
pub fn main() -> Int {
  let empty: Array(Int) = array.new()
  io.println(int.to_string(array.length(empty)))
  let ps = pairs(array.new(), 0, 1000)
  io.println(int.to_string(total(ps, 0, 0)))
  let ps = array.set(ps, 3, Pair("changed", 1000))
  io.println(int.to_string(total(ps, 0, 0)))
  match last(ps) { Pair(s, n) -> io.println(s <> " " <> int.to_string(n)) }
  let sq = doubled(squares(array.new(), 0, 100), 0)
  io.println(int.to_string(sum(sq, 0, 0)) <> " " <> int.to_string(last(sq)))
  let bs = array.set(filled(array.new(), False, 20), 7, True)
  io.println(if array.get(bs, 7) { "yes" } else { "no" })
  io.println(if array.get(bs, 8) { "yes" } else { "no" })
  let fs = filled(array.new(), 1.5, 9)
  io.println(float.to_string(array.get(fs, 8) +. last(fs)))
  let us = filled(array.new(), Unit, 3)
  io.println(int.to_string(array.length(us)))
  let fns = array.push(array.push(array.new(), fn(x: Int) { x + 1 }), fn(x: Int) { x * 10 })
  let f = array.get(fns, 1)
  io.println(int.to_string(f(4) + last(fns)(5)))
  let ss = handle { noted(filled(array.new(), "héllo", 9)) } with {
    Note.note(s) -> { io.println("noted " <> s)  resume(Unit) }
  }
  io.println(last(ss) <> " " <> array.get(ss, 0) <> " " <> int.to_string(array.length(ss)))
  let b = buffer.push(buffer.new(), "héllo")
  let n = buffer.length(b)
  io.println(buffer.finish(buffer.push(b, "!")) <> " " <> int.to_string(n))
  array.length(ps) + array.length(sq)
}
"#;

pub const ARRAYS_OUT: &str = "0\n503390\n504392\nk999 999\n656700 19602\nyes\nno\n3.0\n3\n90\nnoted héllo\nafter héllo 10\nhéllo! 6\n";
pub const ARRAYS_VALUE: &str = "1100";

/// An index out of range stops by name: the program, the message, and what
/// it printed before.
pub const OUT_OF_RANGE: &[(&str, &str)] = &[
    (
        "pub fn main() -> Int {\n  io.println(\"before\")\n  let xs = array.push(array.new(), 1)\n  array.get(xs, 1)\n}\n",
        "array index 1 out of range (the array has 1)",
    ),
    (
        "pub fn main() -> Int {\n  io.println(\"before\")\n  let xs = array.push(array.new(), 1)\n  array.get(xs, 0 - 1)\n}\n",
        "array index -1 out of range (the array has 1)",
    ),
    (
        "pub fn main() -> Int {\n  io.println(\"before\")\n  let xs: Array(String) = array.new()\n  array.length(array.set(xs, 0, \"x\"))\n}\n",
        "array index 0 out of range (the array has 0)",
    ),
];

/// `n` pushes, then the sum of every element read by index: linear in `n`.
pub fn reads(n: u32) -> String {
    format!(
        "fn fill(xs: Array(Int), i: Int, n: Int) -> Array(Int) {{\n  if i == n {{ xs }} else {{ fill(array.push(xs, i), i + 1, n) }}\n}}\n\
         fn sum(borrow xs: Array(Int), i: Int, acc: Int) -> Int {{\n  if i == array.length(xs) {{ acc }} else {{ sum(xs, i + 1, acc + array.get(xs, i)) }}\n}}\n\
         pub fn main() -> Int {{ sum(fill(array.new(), 0, {n}), 0, 0) }}\n"
    )
}

pub fn reads_value(n: u64) -> String {
    (n * (n - 1) / 2).to_string()
}
