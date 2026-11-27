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
];
