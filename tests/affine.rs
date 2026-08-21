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

#[test]
fn affine_across_multi_perform_is_e0429() {
    // Violation: `t` is used after a perform of a `multi` effect (its captured
    // continuation could re-run and use `t` again).
    let src = "effect multi Flip { fn flip() -> Bool }\n\
               linear type Tok { Tok }\n\
               fn use1(t) { match t { Tok -> \"ok\" } }\n\
               pub fn main() {\n\
                 let t = Tok\n\
                 io.println(handle {\n\
                   let _ = flip()\n\
                   use1(t)\n\
                 } with multi { Flip.flip() -> resume(True) })\n\
               }\n";
    let e = err(src);
    assert!(
        e.contains("E0429"),
        "use across a multi-perform must be E0429: {e}"
    );
}

#[test]
fn affine_across_oneshot_perform_is_accepted() {
    // Legal side (the teeth): the SAME shape over a ONE-SHOT effect is fine — a
    // one-shot continuation resumes at most once, so no capture hazard. This pair
    // with the test above proves the check keys on multi-ness, not "any perform."
    let src = "effect Ask { fn ask() -> Bool }\n\
               linear type Tok { Tok }\n\
               fn use1(t) { match t { Tok -> \"ok\" } }\n\
               pub fn main() {\n\
                 let t = Tok\n\
                 io.println(handle {\n\
                   let _ = ask()\n\
                   use1(t)\n\
                 } with { Ask.ask() -> resume(True) })\n\
               }\n";
    assert!(
        check_source("t.elya", src).is_ok(),
        "{:?}",
        check_source("t.elya", src)
    );
}
