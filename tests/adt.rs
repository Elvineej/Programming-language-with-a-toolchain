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
