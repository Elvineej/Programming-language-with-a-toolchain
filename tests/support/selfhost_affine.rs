//! The affine pass in Elya's focused programs (self-hosting part 4e): each
//! kind of fact, and each rule of the walks that make them: counts in
//! sequence and across arms, `let`-bound lambdas counted where called, a
//! lambda called where it is written, clauses and return clauses, pattern
//! bindings; a read after the consuming use, a `borrow` parameter given
//! away, a function with `borrow` parameters used as a value; what each
//! point holds (with the values waiting on the left of a call or an
//! operator), `handle` bodies and clauses as bodies of their own, a
//! constructor call not a point, `op.perform` one, and the resumes found in
//! the walk's order.

#[allow(dead_code)]
pub const FOCUSED: &[&str] = &[
    "fn f(x, c) { if c { x + x } else { x } }\npub fn main() -> Int { f(1, True) }\n",
    "fn f(x) { let g = fn() { x }\n let h = fn() { g() }\n h() + h() }\npub fn main() -> Int { f(1) }\n",
    "fn f(x) { let g = fn() { x }\n let k = g\n g() }\npub fn main() -> Int { 0 }\n",
    "fn f(x) { (fn() { x })() }\nfn g(x) { let k = fn() { x }\n k }\npub fn main() -> Int { 0 }\n",
    "effect E { fn e() -> Int }\nfn f(v) { handle { e() } with { E.e() -> resume(v)  return(r) -> r + r } }\npub fn main() -> Int { 0 }\n",
    "type P { P(Int, Int) }\nfn f(p, q) { match p { P(a, b) -> a + a + q  } }\nfn g(p, q) { match p { P(a, _) -> q  P(_, b) -> q + q } }\npub fn main() -> Int { 0 }\n",
    "fn total(borrow xs: Array(Int), i: Int, acc: Int) -> Int { if i == array.length(xs) { acc } else { total(xs, i + 1, acc + array.get(xs, i)) } }\nfn use(xs: Array(Int)) -> Int { let n = total(xs, 0, 0)\n n }\npub fn main() -> Int { 0 }\n",
    "fn bad(borrow xs: Array(Int)) -> Array(Int) { xs }\nfn late(xs: Array(Int)) -> Int { let ys = array.push(xs, 1)\n array.length(xs) }\npub fn main() -> Int { 0 }\n",
    "fn len2(borrow xs: Array(Int)) -> Int { array.length(xs) }\nfn apply(f, xs) { f(xs) }\nfn go(xs: Array(Int)) -> Int { apply(len2, xs) }\npub fn main() -> Int { 0 }\n",
    "fn f(xs: Array(Int)) -> Array(Int) { array.set(xs, 0, array.get(xs, 0) + 1) }\nfn g(xs: Array(Int), c: Bool) -> Int { if c { let ys = array.push(xs, 1)\n 0 } else { 1 }\n array.length(xs) }\npub fn main() -> Int { 0 }\n",
    "fn h(xs: Array(Int)) -> Int { let k = fn() { array.length(xs) }\n let n = array.length(xs)\n k() + n }\npub fn main() -> Int { 0 }\n",
    "effect multi F { fn flip() -> Int }\nfn h(a, b, k, u) {\nlet g = fn() { k }\nlet x = flip()\nlet y = handle { let _ = flip()\n u } with multi { F.flip() -> resume(1) }\na + x + y + g()\n}\npub fn main() -> Int { 0 }\n",
    "effect multi F { fn flip() -> Int }\ntype T { T(Int) }\nfn h(a) { let t = T(flip())\n let v = handle { flip() + a } with multi { flip() -> resume(1) + resume(2) }\n match t { T(n) -> n + v + a } }\npub fn main() -> Int { 0 }\n",
    "effect multi F { fn flip() -> Int }\nfn h(a, b) { a + flip() + b }\nfn k(a, b) { op.perform(a) + flip() }\nfn m(a) { let f = fn(x) { x + a }\n f(flip()) }\npub fn main() -> Int { 0 }\n",
    "effect multi F { fn flip() -> Int }\nfn h(a) { handle { flip() } with multi { flip() -> { let g = fn() { resume(1) }\n g() + a } } }\npub fn main() -> Int { 0 }\n",
    "effect multi F { fn flip() -> Int }\nfn h(a, c) { if c { flip() + a } else { match a { x -> flip() + x } } }\npub fn main() -> Int { 0 }\n",
    "fn borrow2(borrow a: Array(Int), borrow b: Array(Int)) -> Int { array.length(a) + array.length(b) }\nfn go(xs: Array(Int)) -> Int { borrow2(xs, xs) }\nfn go2(xs: Array(Int)) -> Int { let f = fn(ys) { borrow2(ys, xs) }\n f(xs) }\npub fn main() -> Int { 0 }\n",
    "effect E { fn e() -> Int }\nfn h(xs: Array(Int)) -> Int { handle { e() + array.length(xs) } with { e() -> { let k = fn() { resume(1) }\n k() } } }\npub fn main() -> Int { 0 }\n",
    "fn f(x) { let a = fn() { x }\n let b = fn() { a() + a() }\n b() }\npub fn main() -> Int { 0 }\n",
    "fn f(x, y) { let a = x\n let b = y + y + y\n a + x }\npub fn main() -> Int { 0 }\n",
    // Written for controls D5 and D8, which survived the rest.
    "effect E { fn e() -> Int }\nfn h(xs: Array(Int)) -> Int { handle { e() + array.length(xs) + array.length(xs) } with { e() -> { let k = fn() { resume(1) }\n k() } } }\npub fn main() -> Int { 0 }\n",
    "effect multi F { fn flip() -> Int }\nfn h(a) { handle { flip() } with multi { flip() -> resume(1)  return(x) -> x + a } }\npub fn main() -> Int { 0 }\n",
];
