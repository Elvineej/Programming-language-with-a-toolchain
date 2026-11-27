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
    // Review 2026-11-24: one program per surviving mutant (named).
    // c_args_order
    "fn g2(a, b) { 0 }\nfn h(b) { g2(b + b, b + b) }\npub fn main() -> Int { 0 }\n",
    // c_back_order
    "fn h(c) { c + c\n c + c }\npub fn main() -> Int { 0 }\n",
    // c_clause_order
    "effect E { fn e() -> Int\n fn ask(n: Int) -> Int }\nfn h(k) { handle { 0 } with { e() -> k  ask(p) -> k } }\npub fn main() -> Int { 0 }\n",
    // c_match_order
    "fn h(x) { match x + x { _ -> x + x } }\npub fn main() -> Int { 0 }\n",
    // c_ret_order
    "effect E { fn e() -> Int }\nfn h(a) { handle { a + a } with { e() -> resume(0)  return(r) -> a + a } }\npub fn main() -> Int { 0 }\n",
    // caps_no_extra
    "effect multi F { fn flip() -> Int }\nfn f(x) { let g = fn() { x }\n let h = fn() { g() }\n let y = flip()\n h() }\npub fn main() -> Int { 0 }\n",
    // clause_no_param_rm
    "effect multi F { fn flip() -> Int\n fn pick(n: Int) -> Int }\nfn h(a) { let x = flip()\n handle { 0 } with multi { flip() -> resume(1)  pick(n) -> resume(n) } }\npub fn main() -> Int { 0 }\n",
    // clause_params_outside
    "effect E { fn e() -> Int\n fn ask(n: Int) -> Int }\nfn f() { handle { e() } with { e() -> resume(0)  ask(xs) -> array.length(xs) + array.length(xs) } }\npub fn main() -> Int { 0 }\n",
    // escape_first_only
    "effect E { fn e() -> Int\n fn ask(n: Int) -> Int }\nfn apply(f) { f() }\nfn h(xs: Array(Int)) -> Int { handle { e() + array.length(xs) + array.length(xs) } with { e() -> resume(0)  ask(n) -> apply(fn() { resume(n) }) } }\npub fn main() -> Int { 0 }\n",
    // given_after_nonvar
    "fn g2(a, b) { 0 }\nfn f(xs: Array(Int)) -> Int { let y = g2(xs, xs)\n array.length(xs) }\npub fn main() -> Int { 0 }\n",
    // handle_keeps_pending
    "effect multi F { fn flip() -> Int }\nfn g2(a, b) { 0 }\nfn h(a) { g2(a, handle { flip() } with multi { flip() -> resume(1) }) }\npub fn main() -> Int { 0 }\n",
    // handle_no_cl_join
    "effect multi F { fn flip() -> Int }\nfn h(b) { let x = flip()\n handle { 0 } with multi { flip() -> resume(b) } }\npub fn main() -> Int { 0 }\n",
    // handle_point_no_pending
    "effect multi F { fn flip() -> Int }\nfn g2(a, b) { 0 }\nfn h(a) { g2(a, handle { 0 } with multi { flip() -> resume(1) }) }\npub fn main() -> Int { 0 }\n",
    // is_point_other_false
    "effect multi F { fn flip() -> Int }\nfn h(a) { (fn() { 0 })() + a }\npub fn main() -> Int { 0 }\n",
    // any_multi_first
    "effect E { fn e() -> Int }\neffect multi F { fn flip() -> Int }\nfn h(a) { flip() + a }\npub fn main() -> Int { 0 }\n",
    // ctors_first_type
    "effect multi F { fn flip() -> Int }\ntype A { A(Int) }\ntype B { B(Int) }\nfn h(a) { B(flip()) + a }\npub fn main() -> Int { 0 }\n",
    // pv_order
    "type T { T(Int, Int) }\nfn h(p) { match p { T(a, a) -> a + a } }\npub fn main() -> Int { 0 }\n",
    // rl_match_scrut
    "effect E { fn e() -> Int }\nfn apply(f) { f() }\nfn h(xs: Array(Int)) -> Int { handle { e() + array.length(xs) + array.length(xs) } with { e() -> match apply(fn() { resume(1) }) { _ -> 0 } } }\npub fn main() -> Int { 0 }\n",
    // rl_no_ret
    "effect E { fn e() -> Int }\neffect G { fn g() -> Int }\nfn apply(f) { f() }\nfn h(xs: Array(Int)) -> Int { handle { e() + array.length(xs) + array.length(xs) } with { e() -> handle { g() } with { g() -> 0  return(r) -> apply(fn() { resume(r) }) } } }\npub fn main() -> Int { 0 }\n",
    // l_arms_rev
    "effect multi F { fn flip() -> Int }\nfn h(c) { handle { flip() } with multi { flip() -> match c { 1 -> resume(1)  _ -> resume(2) } } }\npub fn main() -> Int { 0 }\n",
    // l_clauses_rev
    "effect multi F { fn flip() -> Int\n fn pick(n: Int) -> Int }\nfn h() { handle { flip() } with multi { flip() -> resume(1)  pick(n) -> resume(n) } }\npub fn main() -> Int { 0 }\n",
    // l_if_order
    "effect multi F { fn flip() -> Int }\nfn h(c) { handle { flip() } with multi { flip() -> if c { resume(1) } else { resume(2) } } }\npub fn main() -> Int { 0 }\n",
    // lam_no_param_rm
    "effect multi F { fn flip() -> Int }\nfn h(a) { let k = flip()\n fn(x) { x } }\npub fn main() -> Int { 0 }\n",
    // lambda_barrier_after_params, outer_le
    "fn f() { fn(xs) { array.length(xs) + array.length(xs) } }\npub fn main() -> Int { 0 }\n",
    // late_overwrite
    "fn f(xs: Array(Int)) -> Int { let ys = array.push(xs, 1)\n array.length(xs) + array.length(xs) }\npub fn main() -> Int { 0 }\n",
    // o_handle_body_fresh
    "effect E { fn e() -> Int }\nfn g(a) { 0 }\nfn f(xs: Array(Int)) -> Int { let y = g(xs)\n handle { array.length(xs) } with { e() -> resume(0) } }\npub fn main() -> Int { 0 }\n",
    // o_if_cond_after
    "fn gb(a: Array(Int)) -> Bool { True }\nfn f(xs: Array(Int)) -> Int { if gb(xs) { array.length(xs) } else { 0 } }\npub fn main() -> Int { 0 }\n",
    // o_if_prio
    "fn g(a) { 0 }\nfn f(xs: Array(Int), c: Bool) -> Int { let y = if c { g(xs) } else { g(xs) }\n array.length(xs) }\npub fn main() -> Int { 0 }\n",
    // o_match_prio
    "fn g(a) { 0 }\nfn f(xs: Array(Int), c: Int) -> Int { let y = match c { 1 -> g(xs)  _ -> g(xs) }\n array.length(xs) }\npub fn main() -> Int { 0 }\n",
    // o_reads_before_given
    "fn f(y: Array(Int)) -> Int { array.get(y, y) }\npub fn main() -> Int { 0 }\n",
    // o_ret_fresh
    "effect E { fn e() -> Int }\nfn g(a) { 0 }\nfn f(xs: Array(Int)) -> Int { handle { g(xs) } with { e() -> resume(0)  return(r) -> array.length(xs) } }\npub fn main() -> Int { 0 }\n",
    // o_var_first_wins_not
    "fn g(a) { 0 }\nfn f(xs: Array(Int)) -> Int { let a = g(xs)\n let b = g(xs)\n array.length(xs) }\npub fn main() -> Int { 0 }\n",
    // pend_norev
    "effect multi F { fn flip() -> Int }\nfn g3(a, b, c) { 0 }\nfn h(a) { g3(a, a, flip()) }\npub fn main() -> Int { 0 }\n",
    // pend_override
    "effect multi F { fn flip() -> Int }\nfn g2(a, b) { 0 }\nfn h(a) { g2(a, flip()) + a }\npub fn main() -> Int { 0 }\n",
    // perform_not_point
    "effect multi F { fn flip() -> Int }\nfn h(a, o) { op.perform(o) + a }\npub fn main() -> Int { 0 }\n",
    // resume_outermost, ret_inside_clauses
    "effect multi F { fn flip() -> Int }\nfn h() { handle { flip() } with multi { flip() -> handle { resume(1) } with multi { flip() -> resume(2) } } }\npub fn main() -> Int { 0 }\n",
    // ret_barrier
    "effect E { fn e() -> Int }\nfn f(xs: Array(Int)) -> Int { handle { e() } with { e() -> resume(0)  return(r) -> array.length(xs) + array.length(xs) } }\npub fn main() -> Int { 0 }\n",
    // rl_no_handle_cl
    "effect E { fn e() -> Int }\neffect G { fn gg() -> Int }\nfn apply(f) { f() }\nfn h(xs: Array(Int)) -> Int { handle { e() + array.length(xs) + array.length(xs) } with { e() -> handle { gg() } with { gg() -> apply(fn() { resume(1) }) } } }\npub fn main() -> Int { 0 }\n",
    // rl_resume_arg
    "effect E { fn e() -> Int }\nfn apply(f) { f() }\nfn h(xs: Array(Int)) -> Int { handle { e() + array.length(xs) + array.length(xs) } with { e() -> resume(apply(fn() { resume(1) })) } }\npub fn main() -> Int { 0 }\n",
    // v_no_clauses
    "effect E { fn e() -> Int }\nfn len2(borrow xs: Array(Int)) -> Int { 0 }\nfn h() { handle { e() } with { e() -> resume(len2) } }\npub fn main() -> Int { 0 }\n",
    // v_no_lambda
    "fn len2(borrow xs: Array(Int)) -> Int { 0 }\nfn h() { fn() { len2 } }\npub fn main() -> Int { 0 }\n",
    // v_no_ret
    "effect E { fn e() -> Int }\nfn len2(borrow xs: Array(Int)) -> Int { 0 }\nfn h() { handle { e() } with { e() -> resume(0)  return(r) -> len2 } }\npub fn main() -> Int { 0 }\n",
    // vo_lambda
    "effect multi F { fn flip() -> Int }\nfn h(a) { g2(fn() { a }, flip()) }\nfn g2(a, b) { 0 }\npub fn main() -> Int { 0 }\n",
    // vo_unit_pushed
    "effect multi F { fn flip() -> Int }\nfn h(a) { g2(Unit, flip()) }\nfn g2(a, b) { 0 }\npub fn main() -> Int { 0 }\n",
];
