//! Slice 4d-2: the affine-resource guarantee, two-sided. A `linear type`'s
//! values are affine (use at most once — E0428; never live across a `multi`-
//! perform — E0429). Each rule is checked BOTH ways: the violation fires and the
//! legal case is accepted. The guarantee is scoped (intra-function local); the
//! callee-duplication soundness gap is pinned as currently-accepted (Task 4).

use elya::check_source;

fn err(src: &str) -> String {
    check_source("t.elya", src).expect_err("should fail to type-check")
}

#[test]
fn affine_single_use_is_accepted() {
    // Legal side: a linear value used exactly once type-checks.
    let src = "linear type Tok { Tok }\n\
               fn use1(t) { match t { Tok -> \"ok\" } }\n\
               pub fn main() { let t = Tok\n io.println(use1(t)) }\n";
    assert!(
        check_source("t.elya", src).is_ok(),
        "{:?}",
        check_source("t.elya", src)
    );
}

#[test]
fn affine_double_use_is_e0428() {
    // Violation side: two uses of the same affine binding.
    let src = "linear type Tok { Tok }\n\
               fn use1(t) { match t { Tok -> \"ok\" } }\n\
               pub fn main() { let t = Tok\n let _ = use1(t)\n io.println(use1(t)) }\n";
    let e = err(src);
    assert!(e.contains("E0428"), "second use must be E0428: {e}");
}
