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
