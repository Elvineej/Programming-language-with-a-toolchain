//! Exhaustiveness in Elya's focused programs (self-hosting part 4d): nested
//! witnesses, unreachable arms, `Bool` and `Unit` finite and `Int` and
//! `String` infinite, the first missing constructor in declaration order with
//! its arity, ill-typed programs (constructors given the wrong number of
//! arguments, a column mixing types), a wildcard first, an empty `match`,
//! and a `match` in every place the walk reaches (lambdas, handles, clauses,
//! return clauses, resumes, `let`, conditions, operands, arguments,
//! scrutinees), in the walk's order.

#[allow(dead_code)]
pub const FOCUSED: &[&str] = &[
    "type L { N, C(Int, L) }\nfn f(xs) { match xs { N -> 0  C(_, N) -> 1 } }\npub fn main() -> Int { 0 }\n",
    "type O { No, So(Int) }\nfn f(o) { match o { So(x) -> x  So(y) -> y  No -> 0 } }\npub fn main() -> Int { 0 }\n",
    "fn f(b) { match b { True -> 1  False -> 0 } }\nfn g(b) { match b { False -> 1 } }\nfn h(b) { match b { True -> 1  True -> 2  _ -> 3 } }\npub fn main() -> Int { 0 }\n",
    "fn f(n) { match n { 0 -> 1  1 -> 2  0 -> 3 } }\nfn g(s) { match s { \"a\\\"b\\\\c\\n\" -> 1  \"a\\\"b\\\\c\\n\" -> 2  _ -> 3 } }\npub fn main() -> Int { 0 }\n",
    "type W { W(Unit, Bool) }\nfn f(u) { match u { Unit -> 1 } }\nfn g(w) { match w { W(Unit, True) -> 1 } }\nfn h(w) { match w { W(_, True) -> 1  W(Unit, False) -> 2  W(_, _) -> 3 } }\npub fn main() -> Int { 0 }\n",
    "type T { A, B(Int, Int), C }\nfn f(t) { match t { A -> 1  C -> 2 } }\nfn g(t) { match t { B(1, _) -> 1  A -> 2  C -> 3 } }\npub fn main() -> Int { 0 }\n",
    "type T { A, B(Int, Int), C }\nfn f(t) { match t { B(_) -> 1  A -> 2  C -> 3 } }\nfn g(t) { match t { B(_, _, _) -> 1  B(x, y) -> 2  A -> 2  C -> 3 } }\npub fn main() -> Int { 0 }\n",
    "type T { A, B }\ntype U { X, Y }\nfn f(t) { match t { A -> 1  X -> 2 } }\nfn g(n) { match n { 0 -> 1  True -> 2 } }\nfn h(n) { match n { True -> 1  0 -> 2 } }\npub fn main() -> Int { 0 }\n",
    "fn f(x) { match x { _ -> 1  0 -> 2  y -> 3 } }\nfn g(x) { match x { } }\npub fn main() -> Int { 0 }\n",
    "type P { P(Bool, Bool) }\nfn f(p) { match p { P(True, False) -> 1  P(False, True) -> 2  P(True, True) -> 3 } }\nfn g(p) { match p { P(True, _) -> 1  P(_, True) -> 2  P(False, False) -> 3  P(True, True) -> 4 } }\npub fn main() -> Int { 0 }\n",
    "effect E { fn e() -> Int }\nfn f(c) { let k = fn(x) { match x { 1 -> 0 } }\n handle { match c { True -> e() } } with { e() -> resume(match c { False -> 1 })  return(r) -> match r { 2 -> 3 } } }\npub fn main() -> Int { 0 }\n",
    "fn f(a, b) { if match a { True -> b } { -(match b { 1 -> 2 }) } else { g(match a { False -> 0 }, match b { 3 -> 4 }) } }\nfn g(x, y) { 0 }\npub fn main() -> Int { 0 }\n",
    "fn f(a) { match match a { 1 -> 2 } { 3 -> match a { 4 -> 5 }  3 -> 6 } }\npub fn main() -> Int { 0 }\n",
    "type L { N, C(Int, L) }\nfn f(xs) { match xs { C(_, C(_, C(_, _))) -> 1  N -> 0  C(_, N) -> 2 } }\nfn g(xs) { match xs { C(1, C(2, N)) -> 1  C(_, _) -> 2 } }\npub fn main() -> Int { 0 }\n",
    "type M { M(L, L) }\ntype L { N, C(Int, L) }\nfn f(m) { match m { M(N, N) -> 1  M(C(_, _), N) -> 2  M(N, C(_, _)) -> 3 } }\npub fn main() -> Int { 0 }\n",
    "type T { A, B }\nfn f(t) { match t { B -> 1 } }\nfn g(t: Int) -> Int { match t { A -> 1  B -> 2 } }\npub fn main() -> Int { let x = match 1 { 1 -> 1 }\n x }\n",
    "fn f(n) { match n { -1 -> 1  0 -> 2 } }\nfn g(s) { match s { \"\" -> 1  \"\\t\" -> 2 } }\npub fn main() -> Int { 0 }\n",
    // The first missing constructor, in declaration order (control E1).
    "type T { A, B(Int), C }\nfn f(t) { match t { A -> 1 } }\npub fn main() -> Int { 0 }\n",
    // `True` tried before `False` (E2).
    "type P { P(Bool, Bool) }\nfn f(p) { match p { P(True, True) -> 1  P(False, True) -> 2 } }\npub fn main() -> Int { 0 }\n",
    // A ragged row run out of columns matches nothing (E13).
    "type T { A, B(Int, Int) }\nfn f(t) { match t { B(_) -> 1  B(_, 1) -> 2  _ -> 3 } }\npub fn main() -> Int { 0 }\n",
    // Review 2026-11-24: one program per surviving mutant.
    "type T { A(Bool), B }\nfn f(t) { match t { A(True) -> 1 } }\npub fn main() -> Int { 0 }\n",
    "type L { N, C(Int, L) }\nfn f(xs) { match xs { N -> 0  C(1, N) -> 1  _ -> 2 } }\npub fn main() -> Int { 0 }\n",
    "type Q { Q(Bool) }\ntype P { P(Q, Bool) }\nfn f(p) { match p { P(Q(True), False) -> 1  P(Q(False), True) -> 2  _ -> 3 } }\npub fn main() -> Int { 0 }\n",
    "type T { A, B }\ntype U { X, Y }\nfn f(t) { match t { A -> 1  B -> 2  X -> 3  _ -> 4 } }\npub fn main() -> Int { 0 }\n",
    "fn g(x) { x }\nfn f(a) { (match a { 1 -> g })(match a { True -> 0 }) }\npub fn main() -> Int { 0 }\n",
    "fn f(a) { (match a { 1 -> 2 }) + (match a { True -> 3 }) }\npub fn main() -> Int { 0 }\n",
    "fn f(a) { match a { 1 -> match a { True -> 1 }\n  _ -> match a { 2 -> 3 } } }\npub fn main() -> Int { 0 }\n",
    "effect E { fn e() -> Int  fn d() -> Int }\nfn f(a) { handle { e() } with { e() -> resume(match a { 1 -> 2 })  d() -> resume(match a { True -> 3 }) } }\npub fn main() -> Int { 0 }\n",
    "fn f(a) { match a { 1 -> 2 }\n match a { True -> 3 }\n 0 }\npub fn main() -> Int { 0 }\n",
    "type T { A, B(Int, Int) }\ntype P { P(T, Bool) }\nfn f(p) { match p { P(B(_, _, True), False) -> 1  P(A, _) -> 2 } }\npub fn main() -> Int { 0 }\n",
];
