//! The fragments of the checker in Elya's stages (plan
//! docs/superpowers/plans/2026-11-15-elya-checker-in-elya.md, step 4): the
//! smallest stage whose fragment holds a program, read off its syntax.

use elya::ast::{Block, Decl, Expr, Module, Pattern, RowAnn, Stmt, TypeAnn};
use elya::span::Spanned;

/// First-order programs with type errors, for F1's diagnostics: one each,
/// and several in one program (their order is compared too).
pub const FOCUSED_F1: &[&str] = &[
    "pub fn main() -> Int { 1 + True }\n",
    "pub fn main() -> Int { if 1 { 2 } else { 3 } }\n",
    "pub fn main() -> Int { if True { 2 } else { \"x\" } }\n",
    "fn f(x: Int) -> Int { x }\npub fn main() -> Int { f(1, 2) }\n",
    "fn f(x: Int, y: Int) -> Int { x + y }\npub fn main() -> Int { f(1) }\n",
    "fn f(x: Int) -> Bool { x }\npub fn main() -> Int { 0 }\n",
    "pub fn main() -> Int { let x: String = 1\n 2 }\n",
    "pub fn main() -> Int { !3 }\n",
    "pub fn main() -> Float { -.1 }\n",
    "pub fn main() -> Int { 1.5 +. 2 }\n",
    "pub fn main() -> Bool { \"a\" == 1 }\n",
    "pub fn main() -> String { \"a\" <> 1 <> True }\n",
    "pub fn main() -> Int { if 1 < 2.0 { \"a\" } else { 3 + False } }\n",
    "fn g(x) { x + 1 }\nfn h(y) { g(y) && True }\npub fn main() -> Int { h(1) }\n",
    "fn a(n) { b(n) + 1 }\nfn b(n) { if n { a(n) } else { 0 } }\npub fn main() -> Int { a(True) + b(1) }\n",
    "pub fn main() -> Int { let x = 1\n let y = x <> \"s\"\n if y { x } else { 1.0 } }\n",
    // The review's (2026-11-22): the value restriction, the levels a
    // non-value `let` and a binding lower, and the group's member order.
    "fn g(y) { g(y) }\npub fn main() -> Int { let b = g(1)\n let c = b\n if c { 1 } else { c } }\n",
    "fn g(y) { g(y) }\npub fn main() -> Int { let a = { let b = g(1)\n let c = b\n if c { 1 } else { c } }\n a }\n",
    "fn g(y) { g(y) }\nfn f(x) { let a = x == g(1)\n let c = x\n if c { 1 } else { c } }\npub fn main() -> Int { 0 }\n",
    "fn a(n) { let e = 1 + True\n b(n) }\nfn b(n) { let e = if 2 { 1 } else { 2 }\n a(n) }\npub fn main() -> Int { a(1) }\n",
];

/// Programs of F2a with errors: each diagnostic the stage adds, and several
/// in one program.
pub const FOCUSED_F2A: &[&str] = &[
    "type Opt(a) { None, Some(a) }\npub fn main() -> Int { match Some(1) { None -> 0  Some(x) -> x } }\n",
    "type Opt(a) { None, Some(a) }\npub fn main() -> Int { match Some(True) { None -> 0  Some(x) -> x } }\n",
    "type Opt(a) { None, Some(a) }\npub fn main() -> Int { let o = Some(1, 2)\n 0 }\n",
    "type Pair(a, b) { Pair(a, b) }\npub fn main() -> Int { match Pair(1, 2) { Pair(x) -> x } }\n",
    "type T { A(Nope) }\npub fn main() -> Int { 0 }\n",
    "type T(a) { A(a) }\ntype U { B(T), C(T(Int, Int)) }\npub fn main() -> Int { 0 }\n",
    "type Never { }\nfn f(n: Never) -> Int { match n { } }\nfn g(x: Int) -> Int { match x { } }\npub fn main() -> Int { 0 }\n",
    "fn f(x: Int(Bool), y: Opt) -> Int { 0 }\npub fn main() -> Int { 0 }\n",
    "type Opt(a) { None, Some(a) }\nfn f(x: Opt) -> Opt(Int, Int) { x }\npub fn main() -> Int { 0 }\n",
    "fn id(x: a) -> a { x }\nfn bad(x: a, y: a) -> a { if True { x } else { y } }\npub fn main() -> Int { bad(1, True) + id(2) }\n",
    "pub fn main() -> Int { string.length(1) + string.byte_at(\"a\") + int.wrapping_add(1, True) }\n",
    "pub fn main() -> String { int.to_string(1.5) <> float.to_string(1) <> string.slice(\"ab\", 0, 1) }\n",
    "pub fn main() -> Int { match 3 { 1 -> \"one\"  True -> 2  _ -> 3 } }\n",
    "type L(a) { N, C(a, L(a)) }\nfn len(l: L(a)) -> Int { match l { N -> 0  C(_, r) -> 1 + len(r) } }\nfn bad(l) { match l { N -> 0  C(x, r) -> x <> bad(r) } }\npub fn main() -> Int { len(C(1, N)) + bad(C(\"a\", N)) }\n",
    "type Opt(a) { None, Some(a) }\npub fn main() -> Int { let n: Opt(Int) = None\n let s: Opt(String) = Some(1)\n match n { Some(v) -> v  None -> float.round(2.5) } }\n",
    "type Box(a) { Box(a) }\npub fn main() -> Bool { Box(1) == Box(2) && Box(\"a\") != Box(3) }\n",
    // A field whose type has the wrong number of arguments is still that
    // type: using its constructor meets it (control K5).
    "type T(a) { A(a) }\ntype U { B(T) }\npub fn main() -> Int { match B(A(1)) { B(A(x)) -> x } }\n",
    // The second review's (2026-11-22). A `let` does not quantify the
    // function's annotation variables (its D1, a defect it found):
    "type O(a) { No, Ye(a) }\npub fn main() -> Int { let y: O(a) = No\n let z: a = True\n match y { Ye(v) -> v + 1  No -> 0 } }\n",
    "type O(a) { No, Ye(a) }\nfn f(x) { let y: O(a) = No\n let z: a = True\n y }\npub fn main() -> Int { 0 }\n",
    // and the behaviours its mutations showed unpinned: annotation variables
    // per member, kept for the body, and a lowercase name with arguments;
    "fn f(x: a, y: b) -> a { x }\nfn g(x: a) -> a { let y: a = x\n let z: b = 1\n y }\npub fn main() -> Int { 0 }\n",
    "fn f(x: a) -> Int { let y: a = 1\n g(True) }\nfn g(x: a) -> Int { let y: a = 2\n f(x) }\npub fn main() -> Int { 0 }\n",
    "fn f(x: a(Int)) -> Int { 0 }\npub fn main() -> Int { 0 }\n",
    "type T(a, a) { B(a) }\nfn f(x: T(Int, Bool)) -> Int { match x { B(v) -> v } }\npub fn main() -> Int { 0 }\n",
    // an empty match on an error, and on a declared type with constructors;
    "type O(a) { No, Ye(a) }\npub fn main() -> Int { match Ye(1, 2) { } }\n",
    "type O(a) { No, Ye(a) }\nfn f(z) { match Ye(z) { } }\npub fn main() -> Int { 0 }\n",
    // arm bindings staying in their arm, string patterns, a repeated variable;
    "fn f(x) { match True { x -> 1  _ -> x } }\npub fn main() -> Int { f(True) }\n",
    "fn h(x) { match x { \"a\" -> 0  _ -> 1 } }\npub fn main() -> Int { h(1) }\n",
    "type P(a, b) { P(a, b) }\nfn f(p) { match p { P(x, x) -> x } }\npub fn main() -> Int { f(P(True, 1)) + 1 }\n",
    // field types: a parameter given arguments, a base type given arguments;
    "type T(a) { A(b), B(a(Int)) }\npub fn main() -> Int { 0 }\n",
    "type T { A(Int(Bool)), B(Bool(x)) }\npub fn main() -> Int { match A(1) { A(n) -> n  B(b) -> 0 } }\n",
    // and every pure builtin's signature.
    "fn f(s) { string.slice(s, string.length(s), int.shl(1, 2)) }\nfn g(n) { float.of_int(int.xor(n, n)) }\nfn h(x) { float.truncate(x) + int.ushr(x, 1) }\nfn k(x) { int.and(int.or(x, 1), int.shr(x, 2)) + float.round(1.5) + int.wrapping_mul(int.wrapping_sub(x, 1), int.wrapping_add(x, 2)) }\npub fn main() -> Int { 0 }\n",
];

/// Programs of F2b, higher order: compositions, recursion through a lambda
/// (a group member's row deferred), let-polymorphic lambdas, function types
/// in annotations of both variances, functions as values, equality on
/// functions (E0436), an infinite type through a call of a call.
pub const FOCUSED_F2B: &[&str] = &[
    "type Never { }\nfn f(a: Never, b: Never) -> Bool { a == b }\npub fn main() -> Int { 0 }\n",
    "fn app(f, x) { f(x) }\nfn compose(f, g) { fn(x) { g(f(x)) } }\npub fn main() -> Int { compose(fn(a) { a + 1 }, fn(b) { b * 2 })(3) }\n",
    "fn loop(n) { let k = fn() { loop(n - 1) }\n if n == 0 { 0 } else { k() } }\npub fn main() -> Int { loop(3) }\n",
    "fn even(n) { if n == 0 { True } else { (fn(m) { odd(m) })(n - 1) } }\nfn odd(n) { if n == 0 { False } else { even(n - 1) } }\npub fn main() -> Int { if even(4) { 1 } else { 0 } }\n",
    "pub fn main() -> Int { let id = fn(x) { x }\n let a = id(1)\n let b = id(True)\n if b { a } else { 0 } }\n",
    "fn twice(f: fn(Int) -> Int, x: Int) -> Int { f(f(x)) }\nfn mk() -> fn(Int) -> Int { fn(n) { n + 1 } }\npub fn main() -> Int { twice(mk(), 1) }\n",
    "fn inc(n: Int) -> Int { n + 1 }\npub fn main() -> Int { let f = inc\n let g = f\n g(1) + f(True) }\n",
    "pub fn main() -> Bool { let f = fn(x) { x }\n f == f }\n",
    "type Box(a) { Box(a) }\nfn eq(x, y) { x == y }\npub fn main() -> Bool { eq(Box(1), Box(2)) && eq(fn(a) { a }, fn(b) { b }) }\n",
    "fn f(x) { x(x) }\npub fn main() -> Int { 0 }\n",
    "fn g(h) { h(1)(2) }\npub fn main() -> Int { g(fn(a) { fn(b) { a + b } }) }\n",
    "pub fn main() -> Int { let f = fn(a, b) { a + b }\n f(1) + f(1, 2, 3) }\n",
    "type Op { Op(fn(Int, Int) -> Int) }\nfn run(o: Op) -> Int { match o { Op(f) -> f(1, 2) } }\npub fn main() -> Int { run(Op(fn(a, b) { a * b })) }\n",
    "fn map(l, f) { match l { N -> N  C(x, r) -> C(f(x), map(r, f)) } }\ntype L(a) { N, C(a, L(a)) }\npub fn main() -> Int { match map(C(1, N), fn(x) { x + 1 }) { N -> 0  C(y, _) -> y } }\n",
    "fn apply(f: fn(a) -> b, x: a) -> b { f(x) }\npub fn main() -> Int { apply(fn(n) { n + 1 }, True) }\n",
    "fn k(x) { fn(y) { x } }\nfn c() { k }\npub fn main() -> Int { c()(1)(2) }\n",
    "pub fn main() -> Int { let f: fn(Int) -> Int = fn(x) { x }\n let g: fn(Bool) -> Int = f\n 0 }\n",
    "fn rec(f) { rec(fn(x) { f(x) }) }\npub fn main() -> Int { 0 }\n",
    "fn fix(f) { fn(x) { f(fix(f))(x) } }\npub fn main() -> Int { 0 }\n",
    "fn both(a, b) { if True { a } else { b } }\nfn pick() { both(fn(x) { x + 1 }, fn(y) { y }) }\npub fn main() -> Int { pick()(2) }\n",
    // A function used as a value gets an upcast tail (`open_covariant`):
    // joined with a parameter, it leaves that parameter's row open (control
    // K9, which survived before it).
    "fn inc(n: Int) -> Int { n + 1 }\nfn pick(f, g) { if True { f } else { g } }\nfn use(h) { pick(inc, h)(1) }\npub fn main() -> Int { use(inc) }\n",
    // A `let` quantifies a lambda's row: the instance joined to a relayed
    // parameter does not open the one returned (control K12).
    "fn pick(f, g) { if True { f } else { g } }\nfn use(h) { let f = fn(x) { x }\n let a = pick(f, h)\n let b = a(1)\n f }\npub fn main() -> Int { use(fn(n) { n })(True)\n 0 }\n",
    // The third review's (2026-11-23). A `let` keeps a deferred lambda's
    // row (its defect, control K13):
    "type P(x, y) { P(x, y) }\nfn a(n) { let f = fn(k) { k(1) + b(n) }\n P(f, f) }\nfn b(n) { if n == 0 { 0 } else { match a(n - 1) { P(_, _) -> 0 } } }\npub fn main() -> Int { 0 }\n",
    // and the behaviours its mutations showed unpinned (inclusions, a
    // flush at a lambda's exit, deferral, a lambda's phantom tail, 2.4,
    // variance of annotations, `fn` fields, E0436 once per span and over
    // every argument, a type met again):
    "fn r(k) { k(0) + (fn(x) { x })(1) }\npub fn main() -> Int { r(fn(z) { z }) }\n",
    "fn mk() { fn(x) { x + 1 } }\nfn use(h) { let g = if True { mk } else { h }\n g()(1) }\npub fn main() -> Int { use(mk) }\n",
    "fn id(v) { v }\nfn r(k) { let b = k(0)\n fn() { let g = id(fn(x) { x })\n let a = g(1)\n let h = if True { g } else { k }\n a } }\npub fn main() -> Int { r(fn(z) { z })() }\n",
    "fn a(k, n) { let f = fn() { a(k, n - 1) }\n if n == 0 { k(0) } else { f() } }\npub fn main() -> Int { a(fn(z) { z }, 2) }\n",
    "fn u(h) { let g = if True { fn(x) { x } } else { h }\n g(1) }\npub fn main() -> Int { u(fn(z) { z }) }\n",
    "fn id(v) { v }\nfn r(k) { let g = id(fn(x) { x })\n let a = g(1)\n let h = if True { g } else { k }\n a }\npub fn main() -> Int { r(fn(z) { z }) }\n",
    "fn a(n) { fn() { b(fn(x) { x }, n) } }\nfn b(k, n) { if n == 0 { k(0) } else { a(n - 1)() } }\npub fn main() -> Int { b(fn(z) { z }, 2) }\n",
    "fn p(f: fn(fn(Int) -> Int) -> Int) { f }\npub fn main() -> Int { 0 }\n",
    "type O { O(fn(Int) -> Int) }\nfn get(o) { match o { O(f) -> f } }\npub fn main() -> Int { get(O(fn(x) { x }))(1) }\n",
    "fn e(x, y) { x == x && y == y }\nfn h() { e(fn() { 1 }, fn() { 2 }) }\npub fn main() -> Int { 0 }\n",
    "type P(x, y) { P(x, y) }\nfn e(y) { let p = P(fn() { 1 }, y)\n p == p }\nfn h() { e(fn() { 2 }) }\npub fn main() -> Int { 0 }\n",
    "type L { N, C(Int, L) }\nfn e(x: L) -> Bool { x == x }\npub fn main() -> Int { 0 }\n",
];

/// F4a: effect declarations, performs, handlers and their return and
/// operation clauses, `resume` (escaping into a lambda too), labelled rows
/// written and declared, the host's builtins, `main`'s discharge, and the
/// row diagnostics (E0420, E0421, E0423, E0432).
pub const FOCUSED_F4A: &[&str] = &[
    "effect Log { fn log(msg: String) -> Unit }\nfn f() { log(\"a\") }\npub fn main() -> Int { handle { f()  1 } with { log(m) -> resume(Unit) } }\n",
    "effect Log { fn log(msg: String) -> Unit }\nfn f() { log(\"a\") }\npub fn main() -> Int { f()  0 }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn f() { log(\"a\")  ask() }\npub fn main() -> Int { handle { handle f() with { ask() -> resume(2) } } with { log(m) -> resume(Unit) } }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn f() / {Log} -> Int { ask() }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn f() / {Ask, Log} -> Int { ask() }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(msg: String) -> Unit }\nfn f() / {} -> Int { log(\"x\")  1 }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn f() / {} -> Int { log(\"x\")  ask() }\npub fn main() -> Int { 0 }\n",
    "fn f() / {Nope} -> Int { 1 }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(msg: String) -> Unit }\ntype T { T(fn() / {Log, Nope} -> Int) }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(msg: String) -> Unit }\ntype T { T(fn(Int) / {Log} -> Int) }\nfn run(t: T) -> Int { match t { T(g) -> g(1) } }\npub fn main() -> Int { handle run(T(fn(n) { log(\"n\")  n })) with { log(m) -> resume(Unit) } }\n",
    "fn f() { io.println(\"x\") }\npub fn main() / {IO} { f() }\n",
    "pub fn main() / {IO} -> Int { let t = time.now_ms()\n time.sleep_ms(1)\n io.arg_count() + t }\n",
    "fn f(p) { io.write_file(p, io.read_file(p))  io.file_exists(p) }\npub fn main() -> Int { 0 }\n",
    "pub fn main() -> Int { let g = fn() { io.println(io.read_stdin()) }\n 0 }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\npub fn main() -> Int { handle { ask() } with { ask() -> resume(1)  return(x) -> { log(\"r\")  x + 1 } } }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn h() { handle { ask() } with { ask() -> { log(\"c\")  resume(1) } } }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn h() { handle { log(\"b\")  ask() } with { ask() -> resume(1) } }\npub fn main() -> Int { handle h() with { log(m) -> resume(Unit) } }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn h() { handle { log(\"b\")  ask() } with { ask() -> { let k = fn() { resume(1) }\n k() } } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask() -> Int }\nfn h() { handle { ask() + ask() } with { ask() -> { let k = fn(x) { resume(x) }\n k(1) } return(r) -> r * 2 } }\npub fn main() -> Int { h() }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\npub fn main() -> Int { handle { ask() } with { ask() -> resume(1)  log(m) -> resume(Unit) } }\n",
    "effect Ask { fn ask() -> Int }\npub fn main() -> Int { let ask = fn() { 3 }\n ask() }\n",
    "effect Ask { fn ask() -> Int }\nfn g(ask) { ask() }\npub fn main() -> Int { g(fn() { 1 }) }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn run(f: fn() / {Log} -> Int) -> Int { f() }\npub fn main() -> Int { handle run(fn() { ask() }) with { ask() -> resume(1) } }\n",
    "effect Log { fn log(msg: String) -> Unit }\nfn run(f: fn() / {Log} -> Int) -> Int { handle f() with { log(m) -> resume(Unit) } }\npub fn main() -> Int { run(fn() { log(\"x\")  2 }) }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn pick(b: Bool, f: fn() / {Log} -> Int, g: fn() / {Ask} -> Int) -> Int { 0 }\nfn same(f: fn() / {Log} -> Int) -> fn() / {Ask} -> Int { f }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(msg: String) -> Unit }\nfn keep(f) { f }\nfn use() { let g = keep(fn() { log(\"x\") })\n g() }\npub fn main() -> Int { handle { use()  1 } with { log(m) -> resume(Unit) } }\n",
    "effect Log { fn log(msg: String) -> Unit }\nfn walk(n) { if n == 0 { 0 } else { handle { log(\"s\")  walk(n - 1) } with { log(m) -> resume(Unit) } } }\npub fn main() -> Int { walk(2) }\n",
    "effect Log { fn log(msg: String) -> Unit }\nfn a(n) { if n == 0 { log(\"z\")  0 } else { (fn() { b(n - 1) })() } }\nfn b(n) { a(n) }\npub fn main() -> Int { handle a(3) with { log(m) -> resume(Unit) } }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn loop(n) { handle { if n == 0 { ask() } else { loop(n - 1) } } with { log(m) -> { let k = fn() { resume(Unit) }\n k() } } }\npub fn main() -> Int { handle loop(1) with { ask() -> resume(5) } }\n",
    "effect Log { fn log(msg: String) -> Unit }\npub fn main() / {IO} { log(\"x\")  io.println(\"y\") }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn f() { log(\"x\")  ask() }\npub fn main() -> Int { f() }\n",
    "effect Log { fn log(msg: String) -> Unit }\nfn twice(f) { f()  f() }\nfn say() { log(\"x\") }\npub fn main() -> Int { handle { twice(say)  1 } with { log(m) -> resume(Unit) } }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn both(f, g) { f()  g() }\npub fn main() -> Int { handle { handle both(fn() { log(\"x\") }, fn() { ask() }) with { ask() -> resume(3) } } with { log(m) -> resume(Unit) } }\n",
    "effect Log { fn log(msg: String) -> Unit }\nfn wrap(f) { handle f() with { log(m) -> resume(Unit) } }\npub fn main() -> Int { wrap(fn() { log(\"x\")  4 }) }\n",
    "effect Log { fn log(msg: String) -> Unit }\nfn wrap(f) { handle f() with { log(m) -> { io.println(m)  resume(Unit) } } }\npub fn main() / {IO} -> Int { wrap(fn() { log(\"x\")  4 }) }\n",
    "effect Ask { fn ask() -> Int }\npub fn main() -> Int { handle { ask() } with { ask() -> { let r = fn(x) { resume(x) }\n r(1) } } }\n",
    "effect Ask { fn ask() -> Int }\npub fn main() -> Int { handle { ask() } with { ask() -> { let r = fn(x) { resume(x) }\n r(\"no\") } } }\n",
    "effect Log { fn log(msg: String) -> Unit }\npub fn main() -> Int { handle { log(\"x\")  1 } with { log(m) -> resume(m) } }\n",
    "effect Log { fn log(msg: String) -> Unit }\npub fn main() -> Int { handle { log(1)  1 } with { log(m) -> resume(Unit) } }\n",
    "effect Log { fn log(msg: String) -> Unit }\nfn mk() { fn() { log(\"x\") } }\nfn go() { mk()() }\npub fn main() -> Int { handle { go()  1 } with { log(m) -> resume(Unit) } }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn mk() -> fn() / {Log} -> Unit { fn() { ask()  log(\"x\") } }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\npub fn main() -> Int { handle { handle { log(\"a\")  ask() } with { ask() -> { log(\"in\")  resume(7) } } } with { log(m) -> resume(Unit) } }\n",
    "effect Log { fn log(msg: String) -> Unit }\nfn quiet() / {} -> Int { handle { log(\"x\")  1 } with { log(m) -> resume(Unit) } }\npub fn main() -> Int { quiet() }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn half() / {Ask} -> Int { handle { log(\"x\")  ask() } with { log(m) -> resume(Unit) } }\npub fn main() -> Int { handle half() with { ask() -> resume(1) } }\n",
    "effect Zed { fn zed() -> Unit }\neffect Abc { fn abc() -> Unit }\nfn f() { zed()  abc()  io.println(\"x\") }\npub fn main() / {IO} { handle { handle f() with { abc() -> resume(Unit) } } with { zed() -> resume(Unit) } }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\ntype T { T(fn() / {Log} -> Int) }\npub fn main() -> Int { match T(fn() { ask() }) { T(g) -> 0 } }\n",
    "effect Log { fn log(msg: String) -> Unit }\nfn run(f) / {Log} -> Int { log(\"r\")  f() }\npub fn main() -> Int { handle run(fn() { 1 }) with { log(m) -> resume(Unit) } }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn run(f) / {Log} -> Int { f() }\npub fn main() -> Int { handle { handle run(fn() { ask() }) with { ask() -> resume(1) } } with { log(m) -> resume(Unit) } }\n",
    "effect Log { fn log(msg: String) -> Unit }\nfn go(n) { let k = fn() { log(\"k\") }\n if n == 0 { k() } else { go(n - 1) } }\npub fn main() -> Int { handle { go(2)  0 } with { log(m) -> resume(Unit) } }\n",
    // K21: a phantom call row that takes a label after its inclusion was recorded.
    "effect Log { fn log(msg: String) -> Unit }\nfn pick(x) { x }\nfn use_log(f: fn() / {Log} -> Int) -> Int { 0 }\nfn go() { let k = pick(fn() { 1 })\n k()\n use_log(k) }\npub fn main() -> Int { 0 }\n",
    // The review's mutants M4, M11, M17, M19, M30, M37 (verified to differ).
    "effect Log { fn log(msg: String) -> Unit }\nfn run(g: fn() / {Log, Log} -> Unit) { g() }\npub fn main() -> Int { run(fn() { log(\"x\") })\n 0 }\n",
    "effect Ask { fn ask() -> Int }\neffect Log { fn log(msg: String) -> Unit }\nfn h() { handle { ask() } with { ask() -> { let k = fn() { resume(1) }\n k() }  return(x) -> { log(\"r\")\n x } } }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(msg: String) -> Unit }\nfn id(x) { x }\npub fn main() -> Int { let g = id(fn() { 1 })\n g()\n let z = if True { g } else { fn() { log(\"a\")\n 2 } }\n 0 }\n",
    "effect Log { fn log(msg: String) -> Unit }\nfn id(x) { x }\npub fn main() -> Int { let g = id(fn() { 1 })\n let h = fn() { g() }\n let z = if True { g } else { fn() { log(\"a\")\n 2 } }\n 0 }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn id(x) { x }\nfn f() { handle { ask() } with { ask() -> resume(1)  return(x) -> { let g = id(fn() { 1 })\n g()\n let z = if True { g } else { fn() { log(\"a\")\n 2 } }\n x } } }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(msg: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn id(x) { x }\npub fn main() -> Int { let g = id(fn() { 1 })\n let h = fn() { g() }\n let k = fn() { g() }\n let z = if True { g } else { fn() { log(\"a\")\n ask() } }\n 0 }\n",
    "effect Ask { fn ask() -> Int }\neffect Log { fn log(msg: String) -> Unit }\nfn h() { handle { ask() } with { ask() -> { let k = fn() { resume(1) }\n k() } }\n log(\"after\") }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask() -> Int }\neffect Log { fn log(msg: String) -> Unit }\nfn h() { handle { ask() } with { ask() -> { let k = fn() { resume(1) }\n k }  return(x) -> { log(\"r\")\n fn() { x } } } }\npub fn main() -> Int { 0 }\n",
];

/// Stage F4b's focused programs: `with multi` (E0427 over a one-shot
/// effect, the E0426 warning when the body performs `IO`), named row
/// variables (shared across a function's annotations, its declared row
/// checked before anything closes, E0432 spelled like an effect, E0400 a
/// type and a row), row parameters of declared types (the kinds' fixpoint,
/// a row argument printed as its row, E0400 either way round, E0432 an
/// undeclared row variable), and equality on a type holding a row.
pub const FOCUSED_F4B: &[&str] = &[
    "effect Ask { fn ask() -> Int }\nfn h() { handle { ask() } with multi { ask() -> resume(1) } }\npub fn main() -> Int { 0 }\n",
    "effect multi Flip { fn flip() -> Bool }\nfn h() { handle { if flip() { 1 } else { 2 } } with multi { flip() -> resume(True) + resume(False) } }\npub fn main() -> Int { h() }\n",
    "effect multi Flip { fn flip() -> Bool }\nfn h() { handle { io.println(\"x\")\n if flip() { 1 } else { 2 } } with multi { flip() -> resume(True) + resume(False) } }\npub fn main() / {IO} -> Int { h() }\n",
    "effect multi Flip { fn flip() -> Bool }\nfn h() { handle { if flip() { 1 } else { 2 } } with multi { flip() -> { io.println(\"c\")\n resume(True) } } }\npub fn main() / {IO} -> Int { h() }\n",
    "effect Ask { fn ask() -> Int }\nfn h() { handle { io.println(\"x\")\n ask() } with multi { Ask.ask() -> resume(1) } }\npub fn main() / {IO} -> Int { h() }\n",
    "effect multi Flip { fn flip() -> Bool }\neffect Ask { fn ask() -> Int }\nfn h() { handle { if flip() { ask() } else { 2 } } with multi { flip() -> resume(True) } }\nfn g() { handle { h() } with { ask() -> resume(3) } }\npub fn main() -> Int { g() }\n",
    "effect Log { fn log(m: String) -> Unit }\nfn f(g: fn() / {Log | e} -> Int) / {Log | e} -> Int { g() }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(m: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn f(g: fn() / {e} -> Int) / {Log | e} -> Int { g() }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(m: String) -> Unit }\nfn f(g: fn() / {e} -> Int) / {Log | e} -> Int { log(\"a\")\n g() }\nfn k() { f(fn() { 2 }) }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(m: String) -> Unit }\nfn f(g: fn() / {log} -> Int) -> Int { g() }\npub fn main() -> Int { 0 }\n",
    "fn f(x: e, g: fn() / {e} -> Int) -> Int { g() }\npub fn main() -> Int { 0 }\n",
    "fn f(g: fn() / {e} -> Int, x: e) -> Int { g() }\npub fn main() -> Int { 0 }\n",
    "fn f(g: fn() / {io} -> Int) -> Int { g() }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(m: String) -> Unit }\nfn f(g: fn() / {e} -> Int) / {e} -> Int { let h = fn() { g() }\n let k = h\n k() + h() }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(m: String) -> Unit }\nfn f(g: fn() / {e} -> Int) -> fn() / {e} -> Int { g }\nfn u() { f(fn() { log(\"x\")\n 1 }) }\npub fn main() -> Int { 0 }\n",
    "type Task(e) { Task(fn() / {e} -> Int) }\nfn mk() { Task(fn() { 1 }) }\nfn run(t) { match t { Task(f) -> f() } }\npub fn main() -> Int { run(mk()) }\n",
    "effect Log { fn log(m: String) -> Unit }\ntype Task(e) { Task(fn() / {e} -> Int) }\nfn mk() { Task(fn() { log(\"a\")\n 1 }) }\nfn keep(t: Task({Log})) -> Task({Log}) { t }\nfn more(t: Task({Log | r})) -> Task({Log | r}) { t }\npub fn main() -> Int { 0 }\n",
    "type Task(e) { Task(fn() / {e} -> Int) }\nfn bad(t: Task(Int)) -> Int { 0 }\npub fn main() -> Int { 0 }\n",
    "type Box(a) { Box(a) }\nfn bad(t: Box({IO})) -> Int { 0 }\npub fn main() -> Int { 0 }\n",
    "type U(a) { U(Task(a)) }\ntype Task(e) { Task(fn() / {e} -> Int) }\nfn mk(t) { U(t) }\npub fn main() -> Int { 0 }\n",
    "type Task(e) { Task(fn() / {e} -> Int) }\ntype V(a) { V(Task(a), a) }\npub fn main() -> Int { 0 }\n",
    "type Two(e, a) { Two(fn(a) / {e} -> a) }\nfn ap(t: Two(e, b), x: b) / {e} -> b { match t { Two(f) -> f(x) } }\npub fn main() -> Int { ap(Two(fn(x) { x + 1 }), 2) }\n",
    "effect Log { fn log(m: String) -> Unit }\ntype Task(e) { Task(fn() / {e} -> Int) }\nfn run(t: Task(e)) / {e} -> Int { match t { Task(f) -> f() } }\npub fn main() -> Int { run(Task(fn() { log(\"x\")\n 1 })) }\n",
    "effect Log { fn log(m: String) -> Unit }\nfn f(g: fn() / {Log, Log | e} -> Int) / {Log | e} -> Int { g() }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(m: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn f(g: fn() / {e} -> Int) / {Ask, Log | e} -> Int { log(\"a\")\n g() }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(m: String) -> Unit }\nfn a(g: fn() / {e} -> Int) / {e} -> Int { b(g) }\nfn b(g: fn() / {e} -> Int) / {e} -> Int { if True { g() } else { a(g) } }\npub fn main() -> Int { 0 }\n",
    "type Task(e) { Task(fn() / {e} -> Int), Done }\nfn eq(a: Task({}), b: Task({})) -> Bool { a == b }\npub fn main() -> Int { 0 }\n",
    // Written for controls K36, K39 and K40, which survived the rest.
    "effect Log { fn log(m: String) -> Unit }\nfn f(g: fn() / {Log} -> Int) -> Int {\n  let h = { let k: fn() / {e} -> Int = fn() { 1 }\n k }\n  let j: fn() / {e} -> Int = g\n  h()\n}\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(m: String) -> Unit }\nfn f(g: fn() / {Log} -> Int) {\n  let h = fn(x: fn() / {e} -> Int) { x }\n  let a = h(g)\n  let b = h(fn() { 1 })\n  b\n}\npub fn main() -> Int { 0 }\n",
    "type Task(e) { Task(fn() / {e} -> Int) }\ntype X { X(Task(b)) }\ntype W(a) { W(Task(Int)) }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(m: String) -> Unit }\nfn f() / {Log | e} -> Int { let k: fn() / {e} -> Int = fn() { 1 }\n log(\"x\")\n 0 }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(m: String) -> Unit }\neffect Ask { fn ask() -> Int }\nfn f() / {Log | e} -> Int { let k: fn() / {e} -> Int = fn() { ask() }\n log(\"x\")\n 0 }\npub fn main() -> Int { 0 }\n",
    // The review's mutants A1, A3/N8, C2, C5, D2, D4, D11, E4, E5b, E6, E10,
    // G1, H4, H9, C4b, C7, N1 (a Rust quirk, kept: 2.48 reads the last
    // member's annotation variables), N2, N3, N4.
    "fn f(g: fn() / {iO} -> Int) -> Int { 1 }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(m: String) -> Unit }\nfn f(x: log) / {log} -> Int { 0 }\npub fn main() -> Int { 0 }\n",
    "type Two(a, e) { Two(fn(a) / {e} -> a) }\nfn f(t: Two(Int, Int)) -> Int { 0 }\npub fn main() -> Int { 0 }\n",
    "type Box(a) { Box(a) }\nfn f(b: Box(fn() -> Int)) -> Int { 0 }\npub fn main() -> Int { 0 }\n",
    "type Task(e) { Task(fn() / {e} -> Int) }\ntype U { U(Task({})) }\npub fn main() -> Int { 0 }\n",
    "type T { T(fn() / {r} -> Int) }\nfn g(t: T) { match t { T(h) -> h } }\npub fn main() -> Int { 0 }\n",
    "type Box(a) { Box(a) }\ntype U(e) { U(Box({})) }\npub fn main() -> Int { 0 }\n",
    "type Task(e) { Task(fn() / {e} -> Int) }\ntype Box(a) { Box(a) }\ntype U(a) { U(Box(Task(a))) }\nfn f(u: U({})) -> Int { 0 }\npub fn main() -> Int { 0 }\n",
    "type Task(e) { Task(fn() / {e} -> Int) }\ntype W(a) { W(Task(a(Int)), a) }\npub fn main() -> Int { 0 }\n",
    "type U(a) { U(V(a)) }\ntype V(b) { V(W(b)) }\ntype W(c) { W(fn() / {c} -> Int) }\nfn f(u: U({})) -> Int { 0 }\npub fn main() -> Int { 0 }\n",
    "type Task(e) { Task(fn() / {e} -> Int) }\ntype U { U(Task({}, {})) }\npub fn main() -> Int { 0 }\n",
    "effect multi Flip { fn flip() -> Bool }\nfn h() { handle { 1 } with multi { return(x) -> x } }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(m: String) -> Unit }\nfn f() / {Log | log} -> Int { 1 }\npub fn main() -> Int { 0 }\n",
    "fn b(x: e) -> Int { a(fn() { 0 }) }\nfn a(g: fn() / {e} -> Int) -> Int { b(1) }\npub fn main() -> Int { 0 }\n",
    "type Task(e) { Task(fn() / {e} -> Int) }\nfn f(t: Task(e(Int))) -> Int { 0 }\npub fn main() -> Int { 0 }\n",
    "type Task(e) { Task(fn() / {e} -> Int) }\nfn f(t: Task(Task({}))) -> Int { 0 }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(m: String) -> Unit }\nfn b(y) -> Int { a(y) }\nfn a(x: e) / {e} -> Int { b(x) }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(m: String) -> Unit }\nfn a(x: e) / {e} -> Int { b(x) }\nfn b(y: e) / {e} -> Int { a(y) }\npub fn main() -> Int { 0 }\n",
    "effect Zed { fn zed() -> Int }\nfn f(g: fn() / {zed} -> Int) -> Int { 0 }\npub fn main() -> Int { 0 }\n",
    "effect Abc { fn abc() -> Int }\nfn f(g: fn() / {abc} -> Int) -> Int { 0 }\npub fn main() -> Int { 0 }\n",
];

/// Stage F4c's focused programs: parametric effects. A label's arguments
/// agree wherever it meets itself (E0423 at a conflict, a duplicate along a
/// chain settled), are instantiated once per perform and once per handler,
/// are written in annotations (fresh when unwritten, E0400 on a wrong count)
/// and declarations (E0432 when unwritten but the effect's own), carry rows
/// (row parameters of effects, E0424 and E0401 through them, a label kept
/// whole out of its own tail), and a declared row's written arguments and
/// named tail are checked against what the body performs.
pub const FOCUSED_F4C: &[&str] = &[
    "effect State(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn counter() { put(get() + 1) }\nfn run() { handle { counter()\n get() } with { get() -> resume(0)  put(v) -> resume(Unit) } }\npub fn main() -> Int { run() }\n",
    "effect State(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn bad() { put(1)\n put(\"x\") }\npub fn main() -> Int { 0 }\n",
    "effect State(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn a() { put(1) }\nfn b() { put(\"x\") }\nfn both() { a()\n b() }\npub fn main() -> Int { 0 }\n",
    "effect State(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn f() / {State(Int)} -> Int { get() }\nfn g() / {State(String)} -> Int { put(1)\n 0 }\npub fn main() -> Int { 0 }\n",
    "effect State(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn f() / {State} -> Int { 0 }\nfn g() / {State(Int, Int)} -> Int { 0 }\nfn h(k: fn() / {State(Bool)} -> Int) -> Int { k() }\npub fn main() -> Int { 0 }\n",
    "effect E(t) { fn op(x: t) -> Unit }\nfn f() { op(f) }\npub fn main() -> Int { 0 }\n",
    "effect E(t) { fn op(x: t) -> Unit }\nfn g(k) { op(k)\n k() }\npub fn main() -> Int { 0 }\n",
    "effect Async(e) { fn spawn(f: fn() / {Async(e) | e} -> Int) -> Int }\nfn task() { spawn(fn() { 1 }) }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(m: String) -> Unit }\neffect Async(e) { fn spawn(f: fn() / {Async(e) | e} -> Int) -> Int }\nfn task() { spawn(fn() { log(\"a\")\n 1 }) }\nfn run(t: fn() / {Async(e) | e} -> Int) / {Async(e) | e} -> Int { t() }\npub fn main() -> Int { 0 }\n",
    "effect Box(a) { fn take() -> a }\ntype Pair(a) { Pair(fn() / {Box(a)} -> a) }\nfn mk() { Pair(fn() { take() }) }\npub fn main() -> Int { 0 }\n",
    "effect Box(a) { fn take() -> a }\ntype Pair(a) { Pair(fn() / {Box} -> a) }\ntype Q { Q(fn() / {Box(Int, Int)} -> Int) }\ntype W { W(fn() / {Nope} -> Int) }\npub fn main() -> Int { 0 }\n",
    "effect Rd(e) { fn ask() -> fn() / {e} -> Int }\nfn go() { let f = ask()\n f() }\npub fn main() -> Int { 0 }\n",
    "effect Rd(e) { fn ask() -> fn() / {e} -> Int }\neffect Log { fn log(m: String) -> Unit }\nfn go() { handle { let f = ask()\n f() } with { ask() -> resume(fn() { log(\"x\")\n 1 }) } }\npub fn main() -> Int { 0 }\n",
    "effect State(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn run(init) { handle { put(init)\n get() } with { get() -> resume(init)  put(v) -> resume(Unit) } }\nfn use() { run(3) + 1 }\npub fn main() -> Int { use() }\n",
    "effect State(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn run() { handle { put(1)\n get() } with { get() -> resume(\"no\")  put(v) -> resume(Unit) } }\npub fn main() -> Int { 0 }\n",
    "effect State(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn f(k: fn() / {State(Int)} -> Int) / {State(Int)} -> Int { k()\n put(2)\n get() }\nfn g() / {State(Int)} -> Int { f(fn() { get() }) }\npub fn main() -> Int { 0 }\n",
    "effect State(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn f(k: fn() / {State(Int) | e} -> Int) / {State(Int) | e} -> Int { k() }\nfn g() { f(fn() { put(\"s\")\n 1 }) }\npub fn main() -> Int { 0 }\n",
    "effect State(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn apply(k) { k()\n put(1) }\nfn h() { apply(fn() { put(True) }) }\npub fn main() -> Int { 0 }\n",
    "effect State(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn r(n) { if n == 0 { get() } else { put(n)\n r(n - 1) } }\npub fn main() -> Int { 0 }\n",
    "effect State(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn a(n) { if n == 0 { put(1) } else { b(n) } }\nfn b(n) { put(\"x\")\n a(n - 1) }\npub fn main() -> Int { 0 }\n",
    "effect E(a, b) { fn op(x: a) -> b }\nfn f() { op(1) }\nfn g() { let x: String = op(True)\n x }\npub fn main() -> Int { 0 }\n",
    "effect E(t) { fn op(x: t) -> t }\nfn id_eff(x) { op(x) }\nfn two() { let a = id_eff(1)\n let b = id_eff(\"s\")\n a }\npub fn main() -> Int { 0 }\n",
    "effect E(t) { fn op(x: t) -> t }\nfn h() { handle { let g = fn(y) { op(y) }\n g(1) } with { op(x) -> resume(x) } }\npub fn main() -> Int { h() }\n",
    "effect E(t) { fn op(x: t) -> t }\nfn h() { handle { op(1) } with { op(x) -> { let k = fn() { resume(x) }\n let z = k\n z() } } }\npub fn main() -> Int { h() }\n",
    "effect State(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn f() / {State(Int) | e} -> Int { get() }\nfn g() / {State(Int) | e} -> Int { put(\"x\")\n 0 }\npub fn main() -> Int { 0 }\n",
    "effect Ask(e) { fn ask() -> fn() / {e} -> Int }\nfn t() / {e} -> Int { let k = ask()\n k() }\npub fn main() -> Int { 0 }\n",
    "effect St(s) { fn get() -> s }\nfn f(k: fn() / {St(Int)} -> Int) { k() }\nfn g(k: fn() / {St(e)} -> Int) { k() }\nfn h(k: fn() / {St({IO})} -> Int) { k() }\npub fn main() -> Int { 0 }\n",
    "effect R(e) { fn go(f: fn() / {e} -> Int) -> Int }\nfn f(k: fn() / {R({IO})} -> Int) { k() }\nfn g(k: fn() / {R(Int)} -> Int) { k() }\nfn h(k: fn() / {R(e)} -> Int) / {R(e)} -> Int { k() }\npub fn main() -> Int { 0 }\n",
    "effect R(e) { fn go(f: fn() / {e} -> Int) -> Int }\neffect Log { fn log(m: String) -> Unit }\nfn run() { handle { go(fn() { log(\"a\")\n 2 }) } with { go(f) -> resume(f()) } }\npub fn main() -> Int { 0 }\n",
    "effect State(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn outer() { handle { handle { put(1)\n get() } with { get() -> resume(2)  put(v) -> resume(Unit) } } with { get() -> resume(\"a\")  put(v) -> resume(Unit) } }\npub fn main() -> Int { 0 }\n",
    "effect State(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn f() { let k = fn() { get() + 1 }\n let j = fn() { put(\"a\") }\n k() }\npub fn main() -> Int { 0 }\n",
    "effect E(t) { fn op() -> t }\ntype T(e) { T(fn() / {E(Int) | e} -> Int) }\nfn run(t: T(e)) / {E(Int) | e} -> Int { match t { T(f) -> f() } }\nfn mk() { T(fn() { op() }) }\npub fn main() -> Int { 0 }\n",
    "effect E(t) { fn op() -> t }\nfn f(g: fn() / {E(fn() / {E(Int)} -> Int)} -> Int) { g() }\npub fn main() -> Int { 0 }\n",
    "effect multi Amb(t) { fn pick(a: t, b: t) -> t }\nfn h() { handle { pick(1, 2) + pick(3, 4) } with multi { pick(a, b) -> resume(a) + resume(b) } }\npub fn main() -> Int { h() }\n",
    "effect State(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn main_like() / {State(Int)} -> Int { get() }\npub fn main() -> Int { main_like() }\n",
    // Written for controls K46 (two), K51, K56, K63, K62 and K45, which
    // survived the rest.
    "effect E(t) { fn op(x: t) -> Unit }\nfn g(k) { op(k)  k() }\nfn self_op() { op(1)  2 }\npub fn main() -> Int { handle { g(self_op) } with { E.op(x) -> resume(Unit) } }\n",
    "effect E(t) { fn op(x: t) -> Unit }\nfn g(k) { op(k)  k() }\nfn self_op() / {E(Int)} -> Int { op(1)  2 }\nfn h() { g(self_op) }\npub fn main() -> Int { 0 }\n",
    "effect State(s) { fn get() -> s }\nfn f(k: fn() / {State} -> Int) -> Int { k() }\npub fn main() -> Int { 0 }\n",
    "effect Async(e) { fn spawn(f: fn() / {Async(e) | e} -> Int) -> Int }\nfn run(t: fn() / {Async(e) | e} -> Int) / {e} -> Int { t() }\npub fn main() -> Int { 0 }\n",
    "effect E(t) { fn op(x: t) -> Unit }\nfn f(k: fn() / {E(fn(Int) / {} -> Int)} -> Int, j: fn() / {E(fn(String) / {} -> Int)} -> Int) { if True { k } else { j } }\npub fn main() -> Int { 0 }\n",
    "effect E(t) { fn op(x: t) -> Unit }\nfn id(x) { x }\npub fn main() -> Int { let g = id(fn() { 1 })\n g()\n let z = if True { g } else { fn() { op(5)\n 2 } }\n 0 }\n",
    "effect St(s) { fn get() -> s }\neffect Async(e) { fn spawn(f: fn() / {Async(e) | e} -> Int) -> Int }\nfn run(t: fn() / {Async(e) | e} -> Int) -> Int { 0 }\nfn g(k: fn() / {Async({St(String) | w}), St(Int) | w} -> Int) { run(k) }\npub fn main() -> Int { 0 }\n",
    // The review's: K49, M25, M09, M22, M33, M34, M37/M38, M47, M18, K57/M56,
    // then its defects D1 (a settled chain compressed, M12 too) and D2 (a
    // clause of another effect at its own effect's parameters).
    "effect P(a) { fn pp(x: a) -> a }\nfn f(k) { let g = fn(y) { k()\n pp(y) }\n g(1)\n g(\"s\") }\npub fn main() -> Int { 0 }\n",
    "effect Go(e) { fn go(f: fn() / {e} -> Int) -> Int }\neffect Log { fn log(m: String) -> Unit }\nfn f(k) { let g = fn(y) { k()\n go(y) }\n g(fn() { log(\"a\")\n 1 }) }\npub fn main() -> Int { 0 }\n",
    "effect Log { fn log(m: String) -> Unit }\neffect E(t) { fn op(x: t) -> Unit }\nfn f(k: fn() / {E(fn() / {Log} -> Int)} -> Int, j: fn() / {E(fn() / {Log} -> Int)} -> Int) { if True { k } else { j } }\npub fn main() -> Int { 0 }\n",
    "effect Ef(t) { fn op(x: t) -> Unit }\nfn f(k: fn() / {Ef(fn(fn() / {e} -> Int) -> Int) | e} -> Int) / {e} -> Int { k() }\nfn g(k: fn() / {Ef(fn(fn() / {e} -> Int) -> Int) | e} -> Int) { k() }\npub fn main() -> Int { 0 }\n",
    "effect E(t) { fn op(x: t) -> t }\neffect F(a) { fn fo(k: fn() / {E} -> Int) -> a }\npub fn main() -> Int { 0 }\n",
    "effect E(t) { fn op(x: t) -> t }\ntype T { T(fn() / {E(Int, Int)} -> Int) }\nfn mk() { T(fn() { 1 }) }\npub fn main() -> Int { 0 }\n",
    "effect A(x) { fn a(k: fn() / {B(x)} -> Int) -> Int }\neffect B(y) { fn b(f: fn() / {y} -> Int) -> Int }\npub fn main() -> Int { 0 }\n",
    "effect Async(e) { fn spawn(f: fn() / {Async(e) | e} -> Int) -> Int }\nfn run(t) / {e} -> Int { spawn(t)\n let j: fn() / {e} -> Int = t\n t() }\npub fn main() -> Int { 0 }\n",
    "effect Amb(t) { fn pick(a: t, b: t) -> t }\nfn f(k: fn(Unit) / {Amb(Bool) | e} -> Int) { k(pick(fn(z) { 1 }, k)) }\npub fn main() -> Int { 0 }\n",
    "effect P(a, b) { fn pp(x: a) -> b }\neffect Rd(e) { fn ask() -> fn() / {e} -> Int }\nfn f1(x0) / {IO} -> Unit { pp(x0)\n x0() }\nfn f2(x0, x1: fn(Int) -> e) / {Rd({Rd({P(Int, {P | r}, Int) | r})}), IO | e} -> Int { pp(handle { ask()() } with { P.pp(p0) -> { let kk = fn() { resume({ let l2: String = 1\n f1(p0) }) }\n kk() } }) }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask() -> Int }\neffect Fx(t) { fn fx(x: t) -> Int }\nfn f() { handle { 1 } with { ask() -> { let k = fn() { resume(fx(1)) }\n k() }  return(r) -> fx(\"s\") } }\npub fn main() -> Int { 0 }\n",
    "effect State(s) { fn get() -> s  fn put(v: s) -> Unit }\neffect E(t) { fn op(x: t) -> t }\nfn f() { handle { get() + 1 } with { get() -> resume(1)  put(v) -> resume(Unit)  op(x) -> resume(x) } }\npub fn main() -> Int { 0 }\n",
];

/// Stage F4d's focused programs: `with any`. A clause's operation is an
/// `Op(t / {..})` of a rigid answer type, `op.*` on it, `op.perform`
/// forwarding; the handled row is data (E0434, `IO` too, at the handle and
/// where a scheme generalizing it is instantiated); an answer type escaping
/// its clause (E0435) into the result, a binding outside, or an effect's
/// argument; equality on an operation.
pub const FOCUSED_F4D: &[&str] = &[
    "effect Ask { fn ask(q: String) -> Int }\nfn f(body) { handle { body() } with any { op(o) -> resume(op.decode(o, \"1\")) } }\npub fn main() -> Int { f(fn() { ask(\"x\") }) }\n",
    "effect Ask { fn ask(q: String) -> Int }\nfn f(body) { handle { body() } with any { op(o) -> { let s = op.show(o)\n let n = op.name(o)\n resume(op.decode(o, s <> n)) } } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\nfn fwd(body) { handle { body() } with any { op(o) -> resume(op.perform(o)) } }\nfn run() { fwd(fn() { ask(\"q\") }) }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(f: fn() -> Int) -> Int }\nfn f() { handle { ask(fn() { 1 }) } with any { op(o) -> resume(op.decode(o, \"1\")) } }\npub fn main() -> Int { 0 }\n",
    "fn f() { handle { io.println(\"x\")\n 1 } with any { op(o) -> resume(op.decode(o, \"1\")) } }\npub fn main() / {IO} -> Int { f() }\n",
    "effect Ask { fn ask(q: String) -> Int }\nfn f() { handle { ask(\"a\") } with any { op(o) -> o } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\nfn f() { let keep = 0\n handle { ask(\"a\") } with any { op(o) -> { let x = resume(op.decode(o, \"1\"))\n x } } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\nfn g(body) { handle { body() } with any { op(o) -> resume(op.decode(o, \"1\")) } }\nfn h() { g(fn() { ask(\"z\") }) }\nfn k() { g(fn() { 5 }) }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\neffect Bad { fn bad(f: fn() -> Int) -> Int }\nfn g(body) { handle { body() } with any { op(o) -> resume(op.decode(o, \"1\")) } }\nfn h() { g(fn() { bad(fn() { 1 }) }) }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\nfn g(body) { handle { body() } with any { op(o) -> resume(op.decode(o, \"1\")) } }\nfn h() { let gg = g\n gg(fn() { ask(\"z\") }) }\npub fn main() -> Int { 0 }\n",
    "effect St(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn f(body) { handle { body() } with any { op(o) -> resume(op.decode(o, op.encode(o, op.perform(o)))) } }\nfn h() { f(fn() { put(1)\n get() }) }\npub fn main() -> Int { 0 }\n",
    "effect St(s) { fn get() -> s }\nfn f() { handle { let g = fn() { get() }\n g() + 1 } with any { op(o) -> resume(op.decode(o, \"0\")) } }\npub fn main() -> Int { 0 }\n",
    "effect St(s) { fn get() -> s }\nfn f() { handle { let g: fn() -> Int = fn() { 1 }\n get() } with any { op(o) -> resume(op.decode(o, \"0\")) } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\nfn f() { handle { ask(\"a\") } with any { op(o) -> { let k = fn() { resume(op.decode(o, \"1\")) }\n k() } } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\neffect Log { fn log(m: String) -> Unit }\nfn f() { handle { ask(\"a\") } with any { op(o) -> { log(op.show(o))\n resume(op.decode(o, \"1\")) } } }\npub fn main() -> Int { f() }\n",
    "effect Ask { fn ask(q: String) -> Int }\nfn f() { handle { ask(\"a\") } with any { op(o) -> resume(1) } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\nfn f() { handle { ask(\"a\") } with any { op(o) -> resume(op.decode(o, \"1\"))  return(x) -> x + 1 } }\npub fn main() -> Int { f() }\n",
    "effect Ask { fn ask(q: String) -> Int }\nfn f(o) { op.show(o) }\nfn g() { handle { ask(\"a\") } with any { op(o) -> { let s = f(o)\n resume(op.decode(o, s)) } } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\nfn f() { handle { ask(\"a\") } with any { op(o) -> if o == o { resume(op.decode(o, \"1\")) } else { 0 } } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\nfn f() { let r = handle { ask(\"a\") } with any { op(o) -> op.encode(o, 1) }\n r }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\neffect T { fn t() -> Int }\nfn f() { handle { handle { ask(\"a\") + t() } with any { op(o) -> resume(op.decode(o, \"1\")) } } with any { op(p) -> resume(op.decode(p, \"2\")) } }\npub fn main() -> Int { f() }\n",
    "effect E(t) { fn op1(x: t) -> t }\nfn f() { handle { op1(fn() { 1 }) } with any { op(o) -> resume(op.decode(o, \"1\")) } }\nfn g() { handle { op1(3) } with any { op(o) -> resume(op.decode(o, \"1\")) } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\nfn f(body) { handle { body() } with any { op(o) -> op.decode(o, \"1\") } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\nfn f(keep) { handle { ask(\"a\") } with any { op(o) -> { let v = op.decode(o, \"1\")\n let u = if True { keep } else { v }\n resume(v) } } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\neffect Box(t) { fn put(x: t) -> Unit }\nfn f() { handle { ask(\"a\") } with any { op(o) -> { put(op.decode(o, \"1\"))\n resume(1) } } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\nfn f(body) { handle { body() } with any { op(o) -> resume(op.decode(o, \"1\")) } }\nfn g(body2) { f(body2) }\nfn h() { g(fn() { io.println(\"x\")\n 1 }) }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\nfn f(body) { handle { body() } with any { op(o) -> resume(op.perform(o)) } }\nfn h() { f(fn() { io.println(\"x\")\n 1 }) }\npub fn main() -> Int { 0 }\n",
    // Written for controls K66 and K73, which survived the rest.
    "fn f() { handle { io.println(\"x\")\n 1 } with any { op(o) -> 0 } }\npub fn main() / {IO} -> Int { f() }\n",
    "effect Ask { fn ask(q: String) -> Int }\neffect Box(t) { fn put(x: t) -> Unit }\nfn f() { handle { handle { ask(\"a\") } with any { op(o) -> { put(op.decode(o, \"1\"))\n resume(1) } } } with { put(x) -> resume(Unit) } }\npub fn main() -> Int { 0 }\n",
    // The review's mutants N1, N4, N5/N11, N7/N12, N13, N19, N23, N33, N38,
    // N44, N45; then its two defects: a name bound again in its block hides
    // its earlier binding from E0435's walk; and a chain's duplicates,
    // settled, are compressed away (Rust's path compression), so a later
    // settling does not report them again.
    "effect Bad { fn bad(f: fn() -> Int) -> Int }\nfn f(b) { fn() { handle { b() } with any { op(o) -> resume(op.perform(o)) } } }\nfn g() { f(fn() { bad(fn() { 1 }) })() }\npub fn main() -> Int { 0 }\n",
    "effect Bx(t) { fn bx(x: t) -> Unit }\neffect Ask { fn ask(q: String) -> Int }\nfn f() { bx(1)\n handle { ask(\"q\") } with any { op(o) -> { bx(True)\n \"s\" } } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\neffect Log { fn log(m: String) -> Unit }\nfn f(n) { let r = handle { ask(\"q\") } with any { op(o) -> resume(op.decode(o, \"1\"))  return(x) -> if n == 0 { x } else { f(n - 1) } }\n log(\"x\")\n r }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\neffect Log { fn log(m: String) -> Unit }\nfn f() { handle { ask(\"q\") } with any { op(o) -> resume(op.perform(o)) }\n log(\"x\") }\npub fn main() -> Int { 0 }\n",
    "fn f() / {} -> Int { handle { io.println(\"x\")\n 1 } with any { op(o) -> 0 } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\neffect St(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn f(z) { handle { ask(\"q\") } with any { op(o) -> { let u = if True { z } else { fn() { put(op.decode(o, \"1\")) } }\n resume(1) } } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\neffect Bad { fn bad(f: fn() -> Int) -> Int }\nfn rep(body) { handle { body() } with any { op(o) -> resume(op.decode(o, \"1\")) } }\nfn a() { let r = rep\n r(fn() { bad(fn() { 1 }) }) }\npub fn main() -> Int { 0 }\n",
    "effect St(s) { fn get() -> s  fn put(v: s) -> Unit }\nfn g(b) { handle { b() } with any { op(o) -> resume(op.decode(o, \"1\")) } }\nfn h() { g(fn() { put(fn() { 1 }) }) }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\ntype Opt(a) { No, So(a) }\nfn f(z) { handle { ask(\"q\") } with any { op(o) -> { let u = if True { z } else { So(op.decode(o, \"1\")) }\n resume(op.decode(o, \"1\")) } } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\neffect Log { fn log(m: String) -> Unit }\nfn id(x) { x }\nfn f() { handle { ask(\"q\") } with any { op(o) -> { let g = id(fn() { 1 })\n g()\n let z = if True { g } else { fn() { log(\"a\")\n 2 } }\n resume(op.decode(o, \"1\")) } } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\neffect Log { fn log(m: String) -> Unit }\nfn id(x) { x }\nfn f() { handle { ask(\"q\") } with any { op(o) -> resume(op.decode(o, \"1\"))  return(x) -> { let g = id(fn() { 1 })\n g()\n let z = if True { g } else { fn() { log(\"a\")\n 2 } }\n x } } }\npub fn main() -> Int { 0 }\n",
    "effect Ask { fn ask(q: String) -> Int }\nfn mk() { mk() }\nfn f() -> Int { let x: a = mk()\n let x = 0\n handle { ask(\"q\") } with any { op(o) -> { let u: a = op.decode(o, \"1\")\n resume(u) } } }\npub fn main() -> Int { 0 }\n",
    "effect Bx(t) { fn bx(x: t) -> Unit }\neffect Ask { fn ask(q: String) -> Int }\nfn f(g) { bx(1)\n handle { g()\n bx(True) } with { ask(q) -> resume(1) } }\npub fn main() -> Int { 0 }\n",
    "effect Bx(t) { fn bx(x: t) -> Unit }\nfn f() { bx(1)\n handle { bx(True) } with any { op(p) -> op.perform(p) }\n io.println(\"x\") }\npub fn main() -> Int { 0 }\n",
];

/// The checker's imports: the parser in Elya, its lexer, and the affine pass
/// (its facts, from stage F5).
pub fn reader(name: &str) -> Option<(String, String)> {
    let file = match name {
        "parse" => "selfhost/parse.elya",
        "lex" => "selfhost/lex.elya",
        "affine" => "selfhost/affine.elya",
        _ => return None,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("selfhost").is_dir())?
        .to_path_buf();
    let text = std::fs::read_to_string(root.join(file)).ok()?;
    Some((file.to_owned(), text))
}

const BASE: &[&str] = &["Int", "Float", "Bool", "String", "Unit"];

/// The stage a program needs, 1 to 9: F1 first order; F2a (2) declared
/// types, constructors, `match`, the pure builtins, annotations naming types
/// and type variables; F2b (3) higher order: lambdas, functions as values,
/// calls of locals or of expressions, function types in annotations, all of
/// which bring effect rows into types; F4 effects; F5 linearity, `Buffer`,
/// `Array`, `borrow`; F6 imports.
/// Numbered: 1 F1, 2 F2a, 3 F2b, 4 F4a (effects: declarations, performs,
/// handlers, `resume`, labelled rows), 5 F4b (`multi`, named row variables,
/// rows as type arguments), 6 F4c (parametric effects), 7 F4d (`with any`,
/// `op.*`), 8 F5, 9 F6.
pub fn stage(m: &Module) -> u8 {
    let fns = m
        .decls
        .iter()
        .filter_map(|d| match &d.node {
            Decl::Fn(f) => Some(f.name.clone()),
            _ => None,
        })
        .collect();
    let ctors = m
        .decls
        .iter()
        .flat_map(|d| match &d.node {
            Decl::Type(t) => t
                .variants
                .iter()
                .map(|v| (v.node.name.clone(), v.node.fields.len()))
                .collect(),
            _ => Vec::new(),
        })
        .collect();
    let mut s = Stage(1, fns, ctors);
    if !m.imports.is_empty() {
        s.at(9);
    }
    for d in &m.decls {
        match &d.node {
            Decl::Effect(e) => {
                s.at(if !e.params.is_empty() {
                    6
                } else if e.is_multi {
                    5
                } else {
                    4
                });
                for op in &e.ops {
                    for a in &op.node.param_tys {
                        s.ann(a);
                    }
                    s.ann(&op.node.ret);
                }
            }
            Decl::Type(t) => {
                s.at(if t.is_linear { 8 } else { 2 });
                for v in &t.variants {
                    for f in &v.node.fields {
                        s.ann(f);
                    }
                }
            }
            Decl::Fn(f) => {
                if let Some(r) = &f.effect_row {
                    s.row(r);
                }
                for p in &f.params {
                    s.binder(&p.node.name);
                    if p.node.borrow {
                        s.at(8);
                    }
                    if let Some(a) = &p.node.ann {
                        s.ann(a);
                    }
                }
                if let Some(a) = &f.ret_ann {
                    s.ann(a);
                }
                s.block(&f.body.node);
            }
        }
    }
    s.0
}

/// The stage so far, the top-level functions' names, and the constructors
/// with their arities: below F2b a call's callee is a function or a
/// constructor, and a function or an n-ary constructor is only ever called.
struct Stage(u8, Vec<String>, Vec<(String, usize)>);

impl Stage {
    fn at(&mut self, n: u8) {
        self.0 = self.0.max(n);
    }

    fn arity(&self, name: &str) -> Option<usize> {
        self.2
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, k)| *k)
    }

    /// A parameter, `let` or pattern variable named like a top-level function
    /// shadows it, so a call of that name may be a call of a local (the
    /// review's D1): conservatively, higher order.
    fn binder(&mut self, name: &str) {
        if self.1.iter().any(|f| f == name) {
            self.at(3)
        }
    }

    /// A written row: labels without arguments are F4a; a named row variable
    /// is F4b; a label with arguments (a parametric effect) is F4c.
    fn row(&mut self, r: &RowAnn) {
        self.at(4);
        if r.tail.is_some() {
            self.at(5);
        }
        for l in &r.labels {
            if let Some(args) = &l.args {
                self.at(6);
                // A label's arguments are annotations too (`St(Buffer)`).
                for x in args {
                    self.ann(x);
                }
            }
        }
    }

    fn ann(&mut self, a: &Spanned<TypeAnn>) {
        let t = &a.node;
        // A row written as a type argument (row parameters, F4b).
        if t.as_row_arg().is_some() {
            self.at(5);
        }
        if let Some(r) = &t.row {
            self.row(r);
        }
        if t.name == "Array" || t.name == "Buffer" {
            self.at(8);
        } else if t.name == "fn" {
            self.at(3);
        } else if !BASE.contains(&t.name.as_str()) || !t.args.is_empty() {
            // A declared type, a type variable, or a base type given
            // arguments (an error the annotations report; the review's D2).
            self.at(2);
        }
        for x in &t.args {
            self.ann(x);
        }
    }

    fn block(&mut self, b: &Block) {
        for st in b.stmts.iter() {
            match &st.node {
                Stmt::Let { name, ann, value } => {
                    self.binder(name);
                    if let Some(a) = ann {
                        self.ann(a);
                    }
                    self.expr(value);
                }
                Stmt::Expr(e) => self.expr(e),
            }
        }
        if let Some(t) = &b.tail {
            self.expr(t);
        }
    }

    fn expr(&mut self, e: &Spanned<Expr>) {
        match &e.node {
            Expr::Int(_) | Expr::Float(_) | Expr::Str(_) | Expr::Bool(_) | Expr::Unit => {}
            // A function, or a constructor with fields, used as a value.
            Expr::Var(n) => {
                if self.1.contains(n) || self.arity(n).is_some_and(|k| k > 0) {
                    self.at(3)
                } else if self.arity(n).is_some() {
                    self.at(2)
                }
            }
            Expr::Error => {}
            Expr::Qualified { module, .. } => match module.as_str() {
                "int" | "float" | "string" => self.at(2),
                "array" | "buffer" => self.at(8),
                // An operation of a `with any` clause.
                "op" => self.at(7),
                _ => self.at(4),
            },
            Expr::Call { callee, args } => {
                match &callee.node {
                    Expr::Var(n) => {
                        if self.arity(n).is_some() {
                            self.at(2)
                        } else if !self.1.contains(n) {
                            // A local, or an operation.
                            self.at(3)
                        }
                    }
                    Expr::Qualified { .. } => self.expr(callee),
                    // A call of a call's result, or of any other
                    // expression, is higher order.
                    _ => {
                        self.at(3);
                        self.expr(callee)
                    }
                }
                args.iter().for_each(|a| self.expr(a));
            }
            Expr::Unary { expr, .. } => self.expr(expr),
            Expr::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            Expr::If {
                cond,
                then_block,
                else_block,
            } => {
                self.expr(cond);
                self.block(&then_block.node);
                self.block(&else_block.node);
            }
            Expr::Block(b) => self.block(b),
            Expr::Handle { body, handler } => {
                self.at(if handler.any {
                    7
                } else if handler.multi {
                    5
                } else {
                    4
                });
                self.expr(body);
                for c in &handler.clauses {
                    for p in &c.node.params {
                        self.binder(&p.node.name);
                        if let Some(a) = &p.node.ann {
                            self.ann(a);
                        }
                    }
                    self.expr(&c.node.body);
                }
                if let Some(r) = &handler.ret {
                    self.binder(&r.binder);
                    self.expr(&r.body);
                }
            }
            Expr::Resume { arg } => {
                self.at(4);
                self.expr(arg);
            }
            Expr::Match { scrutinee, arms } => {
                self.at(2);
                self.expr(scrutinee);
                for a in arms.iter() {
                    self.pattern(&a.node.pat);
                    self.expr(&a.node.body);
                }
            }
            Expr::Lambda { params, body } => {
                self.at(3);
                for p in params {
                    if let Some(a) = &p.node.ann {
                        self.ann(a);
                    }
                }
                self.block(&body.node);
            }
        }
    }

    fn pattern(&mut self, p: &Spanned<Pattern>) {
        match &p.node {
            Pattern::Var(x) => self.binder(x),
            Pattern::Ctor { args, .. } => args.iter().for_each(|a| self.pattern(a)),
            Pattern::Wild | Pattern::Lit(_) => {}
        }
    }
}
