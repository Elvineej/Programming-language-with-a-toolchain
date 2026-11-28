// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! The back end in Elya's focused programs, stage by stage (spec
//! docs/superpowers/specs/2026-11-27-elya-backend-in-elya-design.md D6).

/// The corpus programs the back end must agree on, at least (156 programs
/// were compared in all, focused included, when it was written).
#[allow(dead_code)]
pub const FLOOR: usize = 110;

/// The focused programs refused on purpose, by index; every other one must
/// be built and compared (a refusal is never a pass).
#[allow(dead_code)]
pub const REFUSED: &[usize] = &[35, 57, 58, 59, 65];

/// A program of one constructor of `fields` strings, each read back: 62 is
/// the widest a row's one mask word holds, 63 is refused by name.
#[allow(dead_code)]
pub fn wide_ctor(fields: usize) -> String {
    let tys = vec!["String"; fields].join(", ");
    let args: Vec<String> = (0..fields).map(|i| format!("\"s{i}\" <> \"\"")).collect();
    let pats: Vec<String> = (0..fields).map(|i| format!("x{i}")).collect();
    let last = fields - 1;
    format!(
        "type Big {{ Big({tys}) }}\nfn mk() -> Big {{ Big({}) }}\npub fn main() / {{IO}} {{ match mk() {{ Big({}) -> io.println(x0 <> x{last}) }} }}\n",
        args.join(", "),
        pats.join(", ")
    )
}

/// A program whose lambda captures `caps` strings: 61 is the most a closure's
/// row holds, 62 is refused by name.
#[allow(dead_code)]
pub fn wide_closure(caps: usize) -> String {
    let ps: Vec<String> = (0..caps).map(|i| format!("a{i}: String")).collect();
    let uses: Vec<String> = (0..caps).map(|i| format!("a{i}")).collect();
    let args: Vec<String> = (0..caps).map(|i| format!("\"c{i}\" <> \"\"")).collect();
    format!(
        "fn mk({}) -> fn(Int) -> String {{ fn(x) {{ {} }} }}\npub fn main() / {{IO}} {{ io.println(mk({})(0)) }}\n",
        ps.join(", "),
        uses.join(" <> "),
        args.join(", ")
    )
}

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
    // B3: a ground list built and folded, its nodes kept across collections.
    "type L { N, C(Int, L) }\nfn build(n: Int, acc: L) -> L { if n == 0 { acc } else { build(n - 1, C(n, acc)) } }\nfn sum(l: L, acc: Int) -> Int { match l { N -> acc  C(x, rest) -> sum(rest, acc + x) } }\npub fn main() -> Int { sum(build(5000, N), 0) }\n",
    // B3: nested patterns, first match wins, a match as a value.
    "type T { A, B(Int, String), C(Bool, T) }\nfn f(t: T) -> Int { let v = match t { A -> 0  B(1, _) -> 10  B(n, \"x\") -> n + 100  B(n, _) -> n  C(True, B(m, _)) -> m + 1  C(False, C(_, A)) -> 7  C(_, rest) -> f(rest) }\n v * 2 }\npub fn main() / {IO} { io.println(int.to_string(f(A)) <> \" \" <> int.to_string(f(B(1, \"y\"))) <> \" \" <> int.to_string(f(B(5, \"x\"))) <> \" \" <> int.to_string(f(B(5, \"z\"))) <> \" \" <> int.to_string(f(C(True, B(3, \"\")))) <> \" \" <> int.to_string(f(C(False, C(True, A)))) <> \" \" <> int.to_string(f(C(True, C(False, B(9, \"q\")))))) }\n",
    // B3: literal scrutinees: Int, Bool, String, Unit.
    "fn i(n: Int) -> String { match n { 0 -> \"zero\"  1 -> \"one\"  _ -> \"many\" } }\nfn b(x: Bool) -> Int { match x { True -> 1  False -> 0 } }\nfn s(x: String) -> Int { match x { \"\" -> 0  \"é\" -> 2  _ -> 9 } }\nfn u(x: Unit) -> Int { match x { Unit -> 5 } }\npub fn main() / {IO} { io.println(i(0) <> i(1) <> i(7) <> int.to_string(b(True) + s(\"é\") + s(\"\") + s(\"zz\") + u(Unit))) }\n",
    // B3: structural equality on data.
    "type P { P(Int, String), Q(P, Bool) }\nfn same(a: P, b: P) -> Bool { a == b }\npub fn main() / {IO} { let x = Q(P(1, \"a\"), True)\n if same(x, Q(P(1, \"a\"), True)) && x != Q(P(1, \"b\"), True) && !same(P(2, \"c\"), P(3, \"c\")) { io.println(\"ok\") } else { io.println(\"bad\") } }\n",
    // B3: binders bound only once their arm's test passes.
    "type T { A(Int), B(Int, Int) }\nfn f(t: T, x: Int) -> Int { match t { B(x, 0) -> x  A(_) -> x  B(_, y) -> x + y } }\npub fn main() -> Int { f(B(5, 1), 100) + f(A(2), 10) }\n",
    // B3: a tree of strings kept while building another.
    "type Tr { Leaf, Node(Tr, String, Tr) }\nfn ins(t: Tr, s: String) -> Tr { match t { Leaf -> Node(Leaf, s, Leaf)  Node(l, v, r) -> if string.length(s) < string.length(v) { Node(ins(l, s), v, r) } else { Node(l, v, ins(r, s)) } } }\nfn go(n: Int, t: Tr) -> Tr { if n == 0 { t } else { go(n - 1, ins(t, int.to_string(n * 7919 % 1000))) } }\nfn show(t: Tr) -> String { match t { Leaf -> \".\"  Node(l, v, r) -> \"(\" <> show(l) <> v <> show(r) <> \")\" } }\npub fn main() / {IO} { io.println(show(go(60, Leaf))) }\n",
    // B4: a generic list at three types, its functions cloned per instance.
    "type List(a) { Nil, Cons(a, List(a)) }\nfn len(xs: List(a), n: Int) -> Int { match xs { Nil -> n  Cons(_, r) -> len(r, n + 1) } }\nfn rev(xs: List(a), acc: List(a)) -> List(a) { match xs { Nil -> acc  Cons(x, r) -> rev(r, Cons(x, acc)) } }\nfn heads(xs: List(List(String))) -> String { match xs { Nil -> \"\"  Cons(Cons(s, _), r) -> s <> heads(r)  Cons(Nil, r) -> \"-\" <> heads(r) } }\npub fn main() / {IO} { let a = rev(Cons(1, Cons(2, Nil)), Nil)\n let b = rev(Cons(\"x\", Cons(\"y\", Nil)), Nil)\n let c = Cons(b, Cons(Nil, Cons(Cons(\"z\", Nil), Nil)))\n io.println(int.to_string(len(a, 0) + len(b, 0) + len(c, 0)) <> heads(c)) }\n",
    // B4: two parameters swapped, a Maybe of a pair, equality on instances.
    "type Pair(a, b) { Pair(a, b) }\ntype Maybe(a) { No, Yes(a) }\nfn swap(p: Pair(a, b)) -> Pair(b, a) { match p { Pair(x, y) -> Pair(y, x) } }\nfn get(m: Maybe(a), d: a) -> a { match m { No -> d  Yes(x) -> x } }\npub fn main() / {IO} { let p = swap(Pair(1, \"one\"))\n let q = get(Yes(p), Pair(\"none\", 0))\n match q { Pair(s, n) -> io.println(s <> int.to_string(n)) }\n if Yes(Pair(1, True)) == Yes(Pair(1, True)) && Yes(2) != No { io.println(\"eq\") } else { io.println(\"ne\") } }\n",
    // B4: an unconstrained variable is Unit.
    "type List(a) { Nil, Cons(a, List(a)) }\nfn len(xs: List(a)) -> Int { match xs { Nil -> 0  Cons(_, r) -> 1 + len(r) } }\npub fn main() -> Int { len(Nil) }\n",
    // B4: a generic list built and folded by tail calls.
    "type List(a) { Nil, Cons(a, List(a)) }\nfn range(n: Int, acc: List(Int)) -> List(Int) { if n == 0 { acc } else { range(n - 1, Cons(n, acc)) } }\nfn fold(xs: List(a), acc: Int, f: Int) -> Int { match xs { Nil -> acc  Cons(_, r) -> fold(r, acc + f, f) } }\npub fn main() -> Int { fold(range(3000, Nil), 0, 3) }\n",
    // B4: a generic `let` used at two types is refused, not miscompiled.
    "type List(a) { Nil, Cons(a, List(a)) }\nfn len(xs: List(a)) -> Int { match xs { Nil -> 0  Cons(_, r) -> 1 + len(r) } }\npub fn main() -> Int { let n = Nil\n let a = Cons(1, n)\n let b = Cons(\"s\", Cons(\"t\", n))\n len(a) * 10 + len(b) }\n",
    // B5: closures capturing an Int, a String and a Bool, nested captures.
    "fn mk(n: Int, s: String, b: Bool) -> fn(Int) -> String { fn(x) { let inner = fn(y) { if b { s <> int.to_string(n + y) } else { s } }\n inner(x * 2) } }\npub fn main() / {IO} { let f = mk(10, \"v\", True)\n let g = mk(0, \"w\", False)\n io.println(f(1) <> g(5) <> f(2)) }\n",
    // B5: functions in data, a computed callee, a constructor as a value.
    "type List(a) { Nil, Cons(a, List(a)) }\ntype Box { Box(Int) }\nfn inc(x: Int) -> Int { x + 1 }\nfn dbl(x: Int) -> Int { x * 2 }\nfn apply_all(fs: List(fn(Int) -> Int), x: Int) -> Int { match fs { Nil -> x  Cons(f, r) -> apply_all(r, f(x)) } }\nfn map(xs: List(Int), f: fn(Int) -> Box) -> List(Box) { match xs { Nil -> Nil  Cons(h, t) -> Cons(f(h), map(t, f)) } }\nfn sum(xs: List(Box)) -> Int { match xs { Nil -> 0  Cons(Box(n), t) -> n + sum(t) } }\npub fn main() / {IO} { let c = True\n io.println(int.to_string(apply_all(Cons(inc, Cons(dbl, Cons(fn(x) { x - 3 }, Nil))), 5) + (if c { inc } else { dbl })(100) + sum(map(Cons(1, Cons(2, Nil)), Box)))) }\n",
    // B5: a loop through a closure in tail position, a hundred million deep.
    "type K { K(fn(K, Int, Int) -> Int) }\nfn go(k: K, n: Int, acc: Int) -> Int { if n == 0 { acc } else { match k { K(f) -> f(k, n - 1, acc + 1) } } }\npub fn main() -> Int { go(K(go), 100000000, 0) }\n",
    // B5: captured strings kept across collections.
    "fn adder(s: String) -> fn(String) -> String { fn(t) { s <> t <> s } }\nfn loop(n: Int, f: fn(String) -> String, acc: String) -> String { if n == 0 { acc } else { loop(n - 1, adder(int.to_string(n)), f(string.slice(acc, 0, 1))) } }\npub fn main() / {IO} { io.println(loop(300, adder(\"x\"), \"start\")) }\n",
    // B6: arrays of words and of heap values, grown, set and read back.
    "fn fill(a: Array(Int), i: Int, n: Int) -> Array(Int) { if i == n { a } else { fill(array.push(a, i * i), i + 1, n) } }\nfn names(a: Array(String), i: Int) -> Array(String) { if i == 0 { a } else { names(array.push(a, \"n\" <> int.to_string(i)), i - 1) } }\nfn sum(borrow a: Array(Int), i: Int, acc: Int) -> Int { if i == array.length(a) { acc } else { sum(a, i + 1, acc + array.get(a, i)) } }\npub fn main() / {IO} { let a = array.set(fill(array.new(), 0, 200), 3, 1000)\n let s = names(array.new(), 50)\n io.println(int.to_string(sum(a, 0, 0)) <> array.get(s, 7) <> int.to_string(array.length(s))) }\n",
    // B6: an index out of range stops the program by name.
    "pub fn main() -> Int { let a = array.push(array.new(), 1)\n array.get(a, 1) }\n",
    // B6: a buffer built in pieces, its length, its text.
    "fn build(b: Buffer, n: Int) -> Buffer { if n == 0 { b } else { build(buffer.push(b, int.to_string(n) <> \",\"), n - 1) } }\npub fn main() / {IO} { let b = build(buffer.new(), 300)\n let n = buffer.length(b)\n io.println(int.to_string(n) <> \":\" <> string.slice(buffer.finish(b), 0, 20)) }\n",
    // B6: arrays of Bool and of data in a record.
    "type R { R(Array(Bool), Array(String)) }\nfn mk() -> R { R(array.push(array.push(array.new(), True), False), array.push(array.new(), \"x\")) }\npub fn main() / {IO} { match mk() { R(bs, ss) -> if array.get(bs, 1) { io.println(\"t\") } else { io.println(array.get(ss, 0) <> int.to_string(array.length(bs))) } } }\n",
    // B6: a missing argument and a missing file stop the program by name.
    "pub fn main() / {IO} { if io.file_exists(\"/nonexistent/elya\") { io.println(\"odd\") } else { io.println(int.to_string(io.arg_count())) }\n io.println(io.read_file(\"/nonexistent/elya\")) }\n",
    // B6: reading an argument that is not there.
    "pub fn main() / {IO} { io.println(io.arg(0)) }\n",
    // B3: a string pattern allocated with the computed scrutinee rooted.
    "type T { B(String, String), C(T, Int) }\nfn mk(s: String) -> T { B(s, s <> \"!\") }\nfn f(n: Int) -> String { match mk(int.to_string(n)) { B(\"x\", _) -> \"no\"  B(_, t) -> t  C(_, _) -> \"c\" } }\npub fn main() / {IO} { io.println(f(12)) }\n",
    // B3: a string pattern inside a constructor, its enclosing objects rooted.
    "type T { B(String, String), C(T, Int) }\nfn g(t: T) -> Int { match t { C(B(\"k\", _), n) -> n  C(B(_, s), n) -> string.length(s) + n  _ -> 0 } }\npub fn main() -> Int { g(C(B(\"a\" <> \"b\", \"cd\" <> \"e\"), 4)) }\n",
    // B3: a string pattern with a pending operand rooted.
    "fn g(s: String) -> String { int.to_string(string.length(s)) <> match s { \"a\" -> \"A\"  \"b\" -> \"B\"  _ -> \"?\" } }\npub fn main() / {IO} { io.println(g(\"zz\") <> g(\"b\")) }\n",
    // B5: a computed callee rooted while its arguments allocate.
    "fn adder(s: String) -> fn(String) -> String { fn(t) { s <> t } }\npub fn main() / {IO} { io.println(adder(\"a\" <> \"b\")(int.to_string(7) <> \"c\")) }\n",
    // B5: the closure-tag bound counts lambdas.
    "fn k() -> fn(Int) -> Int { if 1 < 2 { fn(x) { 1 } } else { fn(x) { 2 } } }\npub fn main() -> Int { k()(0) }\n",
    // B5: the closure-tag bound counts constructors used as values.
    "type B { B(Int) }\nfn k() -> fn(Int) -> B { if 1 < 2 { B } else { B } }\npub fn main() -> Int { match k()(3) { B(_) -> 7 } }\n",
    // B5: a parameter shadowing a top-level function, as a value and as a callee.
    "fn inc(x: Int) -> Int { x + 1 }\nfn ap(f: fn(Int) -> Int, x: Int) -> Int { f(x) }\nfn g(inc: fn(Int) -> Int) -> Int { ap(inc, 10) + inc(1) }\nfn h(inc: fn(Int) -> Int) -> Int { inc(5) }\npub fn main() -> Int { g(fn(x) { x * 3 }) * 100 + h(fn(x) { x - 7 }) }\n",
    // B4: a local named like a generic function is not instantiated.
    "type List(a) { Nil, Cons(a, List(a)) }\nfn len(xs: List(a)) -> Int { match xs { Nil -> 0  Cons(_, r) -> 1 + len(r) } }\nfn f(len: Int) -> Int { len + 1 }\npub fn main() -> Int { f(41) + len(Cons(1, Nil)) }\n",
    // B1: dividing by -1, and dividing MIN, are not overflows.
    "fn f(a: Int, b: Int) -> Int { a / b }\npub fn main() -> Int { f(7, -1) + f(-9223372036854775807 - 1, 2) }\n",
    // B3: a non-tail match's binders bound only in their own arm.
    "type T { A(Int), B(Int, Int) }\nfn f(t: T, x: Int) -> Int { let v = match t { B(x, 0) -> x  A(_) -> x  B(_, y) -> x + y }\n v * 2 }\npub fn main() -> Int { f(B(5, 1), 100) + f(A(2), 10) }\n",
    // B5: a capture read only as a match's scrutinee.
    "type T { A, B(Int) }\nfn mk(t: T) -> fn(Int) -> Int { fn(x) { match t { A -> x  B(n) -> n } } }\npub fn main() -> Int { mk(B(7))(1) * 10 + mk(A)(3) }\n",
    // Refused: a handler in an `else` branch.
    "effect Ask { fn ask() -> Int }\nfn f(b: Bool) -> Int { if b { 1 } else { handle { ask() + 1 } with { Ask.ask() -> resume(41) } } }\npub fn main() -> Int { f(False) }\n",
    // Refused: a handler in a match arm.
    "effect Ask { fn ask() -> Int }\nfn f(n: Int) -> Int { match n { 0 -> 1  _ -> handle { ask() + 1 } with { Ask.ask() -> resume(41) } } }\npub fn main() -> Int { f(5) }\n",
    // Refused: a handler in a lambda's body.
    "effect Ask { fn ask() -> Int }\nfn ap(f: fn(Int) -> Int) -> Int { f(1) }\npub fn main() -> Int { ap(fn(x) { handle { ask() + x } with { Ask.ask() -> resume(41) } }) }\n",
    // Records: a six-parameter tail loop.
    "fn loop(a: Int, b: Int, c: Int, d: Int, e: Int, f: Int) -> Int { if a == 0 { b + c + d + e + f } else { loop(a - 1, b + 1, c, d * 1, e, f + a) } }\npub fn main() -> Int { loop(2000, 0, 1, 2, 3, 4) }\n",
    // Records: a lambda of five parameters through a higher-order function, strings in the record.
    "fn ap(f: fn(String, Int, String, Int, String) -> String) -> String { f(\"a\", 1, \"b\" <> \"c\", 2, int.to_string(3)) }\npub fn main() / {IO} { let s = \"!\" <> \"?\"\n io.println(ap(fn(a, b, c, d, e) { a <> int.to_string(b) <> c <> int.to_string(d) <> e <> s })) }\n",
    // Records: a constructor of five fields as a value; a field of a four-parameter function.
    "type R { R(Int, String, Int, String, Int) }\ntype H { H(fn(Int, Int, Int, Int) -> Int) }\nfn mk(f: fn(Int, String, Int, String, Int) -> R) -> R { f(1, \"x\", 2, \"y\", 3) }\nfn sum4(a: Int, b: Int, c: Int, d: Int) -> Int { a * 1000 + b * 100 + c * 10 + d }\npub fn main() / {IO} { match mk(R) { R(a, s, b, t, c) -> match H(sum4) { H(g) -> io.println(s <> t <> int.to_string(a + b + c + g(1, 2, 3, 4))) } } }\n",
    // Records: an instance of a parametric type holding four-parameter functions.
    "type List(a) { Nil, Cons(a, List(a)) }\nfn run(fs: List(fn(Int, Int, Int, Int) -> Int), acc: Int) -> Int { match fs { Nil -> acc  Cons(f, r) -> run(r, acc + f(1, 2, 3, 4)) } }\nfn add4(a: Int, b: Int, c: Int, d: Int) -> Int { a + b + c + d }\npub fn main() -> Int { run(Cons(add4, Cons(fn(a, b, c, d) { a * b * c * d }, Nil)), 0) }\n",
    // Records: mutual tail calls of eight parameters.
    "fn ping(n: Int, a: Int, b: Int, c: Int, d: Int, e: Int, f: Int, g: Int) -> Int { if n == 0 { a + b + c + d + e + f + g } else { pong(n - 1, b, c, d, e, f, g, a + 1) } }\nfn pong(n: Int, a: Int, b: Int, c: Int, d: Int, e: Int, f: Int, g: Int) -> Int { ping(n, g, a, b, c, d, e, f) }\npub fn main() -> Int { ping(3000, 1, 2, 3, 4, 5, 6, 7) }\n",
    // Refused: a parametric type's field of a four-parameter function.
    "type Box(a) { Box(fn(a, a, a, a) -> a) }\nfn get(b: Box(Int)) -> Int { match b { Box(f) -> f(1, 2, 3, 4) } }\npub fn main() -> Int { get(Box(fn(a, b, c, d) { a + b + c + d })) }\n",
    // Records: a function of four parameters never called is still written.
    "type T { A(Int) }\nfn unused(a: Int, b: Int, t: T, d: String) -> Int { match t { A(n) -> a + b + n + string.length(d) } }\npub fn main() -> Int { 7 }\n",
    // Floats: arithmetic and its text, inf, NaN and -0.0 from arithmetic.
    "pub fn main() / {IO} {\n  let z = 0.0\n  io.println(float.to_string(0.1 +. 0.2))\n  io.println(float.to_string(1.0 /. z))\n  io.println(float.to_string(z /. z))\n  io.println(float.to_string(-.z))\n  io.println(float.to_string(2.5e-310 *. 0.5 -. 1.0e300 *. 1.0e10))\n}\n",
    // Floats: comparisons, NaN unequal to itself, -0.0 equal to 0.0.
    "fn b(x: Bool) -> String { if x { \"T\" } else { \"F\" } }\npub fn main() / {IO} {\n  let n = 0.0 /. 0.0\n  io.println(b(n == n) <> b(n != n) <> b(n <. 1.0) <> b(0.0 == -0.0) <> b(1.5 >=. 1.5) <> b(2.0 >. 1.0) <> b(1.0 <=. 0.5))\n}\n",
    // Floats: the conversions, the bits both ways, parse.
    "pub fn main() / {IO} -> Int {\n  io.println(float.to_string(float.of_int(7) *. 0.5))\n  io.println(int.to_string(float.round(2.5) + float.truncate(-2.7)))\n  io.println(int.to_string(float.to_bits(1.0)))\n  io.println(float.to_string(float.of_bits(4611686018427387904)))\n  io.println(float.to_string(float.parse(\"1_000.125e-2\")))\n  float.round(-0.5)\n}\n",
    // Floats: == on data compares a Float field as a float.
    "type P { P(Float, Int) }\npub fn main() -> Bool { P(0.0, 1) == P(-0.0, 1) && P(0.0 /. 0.0, 1) != P(0.0 /. 0.0, 1) }\n",
    // Floats: main answers a Float.
    "pub fn main() -> Float { 1.5 *. 3.0 }\n",
    // Floats: parse fails by name.
    "pub fn main() -> Float { float.parse(\"1.5x\") }\n",
    // Floats: in data, captured by a closure, summed in a tail loop.
    "type L { N, C(Float, L) }\nfn sum(l: L, acc: Float) -> Float { match l { N -> acc  C(x, r) -> sum(r, acc +. x) } }\npub fn main() / {IO} {\n  let k = 0.25\n  let f = fn(x) { x *. k }\n  io.println(float.to_string(sum(C(f(2.0), C(1.0e-3, N)), 0.0)))\n}\n",
];
