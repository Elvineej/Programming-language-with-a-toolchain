//! Slice 4a: ADT programs type-check and run — on BOTH the CEK machine and the
//! tree-walker, asserting `cek == tree` (the effect-free ADT differential oracle
//! is a hard gate from Task 2 onward, spec §3.4).

use elya::{check_source, eval, parse::parse_module, Session};

/// Type-check, then run on BOTH evaluators; assert they agree and return output.
fn run_both(src: &str) -> String {
    assert!(
        check_source("t.elya", src).is_ok(),
        "{:?}",
        check_source("t.elya", src)
    );
    let (m, d) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    let cek = eval::run_module(&m).unwrap().output().to_string();
    let tree = eval::run_module_tree(&m).unwrap().output().to_string();
    assert_eq!(
        cek, tree,
        "cek != tree divergence:\ncek={cek:?}\ntree={tree:?}"
    );
    cek
}

#[test]
fn nullary_adt_runs_end_to_end() {
    let src = "type Bool2 { T, F }\n\
               fn to_int(b) { match b { T -> 1  F -> 0 } }\n\
               pub fn main() { let _ = to_int(T)  io.println(\"ok\") }\n";
    assert_eq!(run_both(src), "ok\n"); // asserts cek == tree by construction
}

#[test]
fn list_length_and_option() {
    let src = "type Option(a) { None, Some(a) }\n\
               type List(a) { Nil, Cons(a, List(a)) }\n\
               fn length(xs) { match xs { Nil -> 0  Cons(_, t) -> 1 + length(t) } }\n\
               fn unwrap_or(o, d) { match o { None -> d  Some(x) -> x } }\n\
               pub fn main() {\n\
                 let xs = Cons(10, Cons(20, Nil))\n\
                 let _ = length(xs)\n\
                 io.println(unwrap_or(Some(\"hi\"), \"default\"))\n\
               }\n";
    assert_eq!(run_both(src), "hi\n");
}

#[test]
fn warnings_surface_on_successful_compile() {
    // A useless arm is an E0431 warning: the program compiles, and the warning
    // is retrievable (the CLI renders it to stderr on success).
    let src = "type Option(a) { None, Some(a) }\n\
               fn f(o) { match o { Some(x) -> x  Some(y) -> y  None -> 0 } }\n\
               pub fn main() { io.println(\"ok\") }\n";
    assert!(check_source("t.elya", src).is_ok(), "warning must not fail");
    let w = elya::warnings("t.elya", src).expect("expected an E0431 warning");
    assert!(w.contains("E0431"), "{w}");
    // A clean program surfaces nothing.
    assert!(elya::warnings("t.elya", "pub fn main() { io.println(\"ok\") }\n").is_none());
}

#[test]
fn lambda_constructs_and_runs() {
    // The closure is built and discarded; the program runs on both evaluators.
    let src = "pub fn main() {\n\
                 let _f = fn(n) { n + 1 }\n\
                 io.println(\"ok\")\n\
               }\n";
    assert_eq!(run_both(src), "ok\n");
}

#[test]
fn lambda_bound_identity_is_polymorphic() {
    // `id` is a syntactic value (a lambda) -> generalized -> typable at two types.
    // (Type-level only; running it needs the call path from Task 3.)
    let src = "pub fn main() {\n\
                 let id = fn(x) { x }\n\
                 let _ = id(1)\n\
                 let _ = id(\"a\")\n\
                 io.println(\"ok\")\n\
               }\n";
    assert!(
        check_source("t.elya", src).is_ok(),
        "{:?}",
        check_source("t.elya", src)
    );
}

#[test]
fn value_restriction_blocks_nonvalue_generalization() {
    // Mirror of `value_restriction_keeps_values_polymorphic`, but the RHS is a
    // *call* (a non-value): `r : List(?a)` stays a monotype, so using it at two
    // element types conflicts. It would only type-check if `r` were (unsoundly)
    // generalized — the value restriction with teeth.
    let src = "type List(a) { Nil, Cons(a, List(a)) }\n\
               pub fn main() {\n\
                 let r = (fn(xs) { xs })(Nil)\n\
                 let _ = Cons(1, r)\n\
                 let _ = Cons(\"a\", r)\n\
                 io.println(\"x\")\n\
               }\n";
    let err = check_source("t.elya", src).unwrap_err();
    assert!(
        err.contains("E0400"),
        "expected a type mismatch, got: {err}"
    );
}

#[test]
fn closure_call_and_capture_run() {
    let src = "pub fn main() {\n\
                 let inc = fn(n) { n + 1 }\n\
                 let by = 10\n\
                 let bump = fn(n) { n + by }\n\
                 let _ = inc(41)\n\
                 let r = bump(5)\n\
                 if r == 15 { io.println(\"ok\") } else { io.println(\"no\") }\n\
               }\n";
    assert_eq!(run_both(src), "ok\n");
}

#[test]
fn higher_order_map_over_list() {
    let src = "type List(a) { Nil, Cons(a, List(a)) }\n\
               fn map(xs, f) { match xs { Nil -> Nil  Cons(h, t) -> Cons(f(h), map(t, f)) } }\n\
               fn sum(acc, xs) { match xs { Nil -> acc  Cons(h, t) -> sum(acc + h, t) } }\n\
               pub fn main() {\n\
                 let xs = Cons(1, Cons(2, Cons(3, Nil)))\n\
                 let ys = map(xs, fn(n) { n * 10 })\n\
                 if sum(0, ys) == 60 { io.println(\"ok\") } else { io.println(\"no\") }\n\
               }\n";
    assert_eq!(run_both(src), "ok\n");
}

#[test]
fn literal_patterns_run() {
    let src = "fn classify(n) { match n { 0 -> \"zero\"  _ -> \"other\" } }\n\
               fn name(b) { match b { True -> \"t\"  False -> \"f\" } }\n\
               pub fn main() { io.println(classify(0))\n io.println(name(False)) }\n";
    assert_eq!(run_both(src), "zero\nf\n");
}

#[test]
fn tree_depth() {
    // Nested constructor patterns + recursion over a parametric Tree.
    let src = "type Tree(a) { Leaf, Node(Tree(a), a, Tree(a)) }\n\
               fn max(x, y) { if x < y { y } else { x } }\n\
               fn depth(t) { match t { Leaf -> 0  Node(l, _, r) -> 1 + max(depth(l), depth(r)) } }\n\
               fn show(n) { if n == 3 { \"three\" } else { \"other\" } }\n\
               pub fn main() {\n\
                 let t = Node(Node(Leaf, 1, Leaf), 2, Node(Node(Leaf, 3, Leaf), 4, Leaf))\n\
                 io.println(show(depth(t)))\n\
               }\n";
    assert_eq!(run_both(src), "three\n");
}
