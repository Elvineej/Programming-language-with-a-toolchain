//! The back end in Elya's focused programs, stage by stage (spec
//! docs/superpowers/specs/2026-11-27-elya-backend-in-elya-design.md D6).

/// The corpus programs the current stage must agree on, at least.
#[allow(dead_code)]
pub const FLOOR: usize = 30;

#[allow(dead_code)]
pub const FOCUSED: &[&str] = &[
    // B1: arithmetic, tail calls, `if` as a value and in tail position.
    "fn fact(n: Int, acc: Int) -> Int { if n == 0 { acc } else { fact(n - 1, acc * n) } }\npub fn main() / {IO} { io.println(int.to_string(fact(20, 1))) }\n",
    // B1: `if` as a value, comparisons, `Bool` parameters and results.
    "fn pick(b: Bool, x: Int, y: Int) -> Int { let v = if b { x } else { y }\n v + 1 }\nfn lt(a: Int, b: Int) -> Bool { a < b }\npub fn main() / {IO} { io.println(int.to_string(pick(lt(3, 2), 10, 20) + pick(3 >= 3, 1, 2) + pick(4 != 4, 100, 200))) }\n",
    // B1: mutual recursion a million deep in tail position.
    "fn ev(n: Int) -> Bool { if n == 0 { True } else { od(n - 1) } }\nfn od(n: Int) -> Bool { if n == 0 { False } else { ev(n - 1) } }\npub fn main() -> Bool { ev(1000000) }\n",
    // B1: division and remainder, negative operands.
    "pub fn main() / {IO} { io.println(int.to_string(-7 / 2)) \n io.println(int.to_string(-7 % 2))\n io.println(int.to_string(7 % -1))\n io.println(int.to_string(-9223372036854775807 - 1 / 1)) }\n",
    // B1: a zero divisor stops the program.
    "fn f(n: Int) -> Int { 10 / n }\npub fn main() -> Int { f(0) }\n",
    // B1: a zero remainder divisor stops it.
    "fn f(n: Int) -> Int { 10 % n }\npub fn main() -> Int { f(0) }\n",
    // B1: `MIN / -1` overflows.
    "fn f(a: Int, b: Int) -> Int { a / b }\npub fn main() -> Int { f(-9223372036854775807 - 1, -1) }\n",
    // B1: addition overflows.
    "fn f(a: Int) -> Int { a + 1 }\npub fn main() -> Int { f(9223372036854775807) }\n",
    // B1: negation overflows at MIN.
    "fn f(a: Int) -> Int { -a }\npub fn main() -> Int { f(-9223372036854775807 - 1) }\n",
    // B1: `&&` and `||` short-circuit past a failing operand.
    "fn f(n: Int) -> Bool { n != 0 && 10 / n > 1 || n == 0 }\npub fn main() -> Bool { f(0) }\n",
    // B1: `!`, `Unit` values and statements.
    "fn u(x: Unit) -> Unit { x }\nfn g(b: Bool) -> Bool { !b == False }\npub fn main() / {IO} { u(Unit)\n io.println(\"a\\tb\\\"c\\\\\")\n if g(True) { io.println(\"yes\") } else { io.println(\"no\") } }\n",
    // B1: `Unit` compared and printed by main.
    "fn f() -> Unit { Unit }\npub fn main() / {IO} { if f() == Unit { io.println(\"same\") } else { io.println(\"not\") } }\n",
    // B1: `MIN % -1` is 0, not a trap.
    "fn f(a: Int, b: Int) -> Int { a % b }\npub fn main() -> Int { f(-9223372036854775807 - 1, -1) }\n",
    // B1: a loop of a hundred million tail calls runs in constant stack.
    "fn count(n: Int, acc: Int) -> Int { if n == 0 { acc } else { count(n - 1, acc + 2) } }\npub fn main() -> Int { count(100000000, 0) }\n",
    // B1: orderings are signed.
    "fn lt(a: Int, b: Int) -> Bool { a < b }\nfn ge(a: Int, b: Int) -> Bool { a >= b }\npub fn main() -> Bool { lt(-1, 1) && ge(1, -1) }\n",
    // B1: multiplication overflows.
    "fn f(a: Int) -> Int { a * 3 }\npub fn main() -> Int { f(4611686018427387904) }\n",
    // B2: strings built across calls, an operand rooted while the next one allocates.
    "fn two(a: String, b: String) -> String { a <> \"|\" <> b }\nfn f(x: String) -> String { two(x <> \"a\", int.to_string(42)) }\nfn loop(n: Int, acc: String) -> String { if n == 0 { acc } else { loop(n - 1, two(acc, f(int.to_string(n)))) } }\npub fn main() / {IO} { io.println(loop(30, \"\")) }\n",
    // B2: equality, length, bytes and slices, UTF-8 included.
    "fn same(a: String, b: String) -> Bool { a == b }\npub fn main() / {IO} { let s = \"héllo wörld\"\n io.println(int.to_string(string.length(s)) <> \" \" <> int.to_string(string.byte_at(s, 1)) <> \" \" <> string.slice(s, 0, 3) <> string.slice(s, 7, 13))\n if same(s, \"héllo\" <> \" wörld\") && s != \"x\" { io.println(\"eq\") } else { io.println(\"ne\") } }\n",
    // B2: a byte out of range stops the program by name.
    "pub fn main() -> Int { string.byte_at(\"abc\", 3) }\n",
    // B2: a slice splitting a character stops it.
    "pub fn main() / {IO} { io.println(string.slice(\"é\", 0, 1)) }\n",
    // B2: shifts, bitwise and wrapping arithmetic.
    "fn f(a: Int, b: Int) -> Int { int.shl(a, b) + int.shr(-a, b) + int.ushr(-1, 60) + int.and(a, 6) + int.or(a, 8) + int.xor(a, 3) }\npub fn main() / {IO} { io.println(int.to_string(f(5, 2)))\n io.println(int.to_string(int.wrapping_add(9223372036854775807, 1)))\n io.println(int.to_string(int.wrapping_mul(4611686018427387904, 4) + int.wrapping_sub(-9223372036854775807 - 1, 1))) }\n",
    // B2: a shift out of range stops the program by name.
    "fn f(n: Int) -> Int { int.ushr(1, n) }\npub fn main() -> Int { f(64) }\n",
    // B2: a negative shift too.
    "fn f(n: Int) -> Int { int.shl(1, n) }\npub fn main() -> Int { f(-1) }\n",
    // B2: many short-lived strings, the live ones kept.
    "fn spin(n: Int, keep: String) -> String { if n == 0 { keep } else { let junk = int.to_string(n) <> \"junk\"\n spin(n - 1, if n % 1000 == 0 { keep <> string.slice(junk, 0, 1) } else { keep }) } }\npub fn main() / {IO} { io.println(spin(20000, \"k\")) }\n",
    // B2: a slice of a value nothing else holds: its argument rooted.
    "fn f(n: Int) -> String { string.slice(int.to_string(n) <> \"abcdef\", 2, 6) }\npub fn main() / {IO} { io.println(f(12345)) }\n",
];
