//! Slice 4d-2: the affine-resource guarantee, two-sided. A `linear type`'s
//! values are affine (use at most once — E0428; never live across a `multi`-
//! perform — E0429). Each rule is checked BOTH ways: the violation fires and the
//! legal case is accepted.
//!
//! Since 2026-11-06 (spec 2026-11-06-elya-affine-across-functions) the
//! guarantee crosses functions: a type holding a `linear` one is not
//! duplicable, and a generic function's duplication travels in its type (D1,
//! E0437); uses are counted by arm and by how often a closure or a clause runs
//! (D2); what a multi-shot point holds is checked by its row, through row
//! variables (D3). The programs are the spec's §0 table (U = read twice,
//! refused; S = sound, accepted).

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

#[test]
fn a_callee_duplicating_its_parameter_is_e0428() {
    // Slice 4d-2 pinned this program as accepted (its tracked soundness gap,
    // `affine-callee-duplication-obligation`: `pair_use` uses its parameter
    // twice). Closed 2026-11-06: a parameter is counted like any binding, and
    // `keep` makes it a `Tok` (an expectation changed, as 4d-2 meant it to).
    let src = "linear type Tok { Tok }\n\
               fn keep(t) { match t { Tok -> \"k\" } }\n\
               fn pair_use(x) { let _ = keep(x)  keep(x) }\n\
               pub fn main() { let t = Tok\n io.println(pair_use(t)) }\n";
    let e = err(src);
    assert!(e.contains("E0428"), "{e}");
}

/// The prelude of the spec's measurements: a `File` read twice is used twice.
const P: &str = "linear type File { File(Int) }\n\
                 fn read(f: File) -> Int { match f { File(n) -> n } }\n";
const FLIP: &str = "effect multi Flip { fn flip() -> Bool }\n";
const RESUME_TWICE: &str = " with multi { Flip.flip() -> resume(True) + resume(False) }";
const EACH: &str = "type List(a) { Nil, Cons(a, List(a)) }\n\
                    fn each(xs, k) { match xs { Nil -> 0  Cons(h, t) -> k(h) + each(t, k) } }\n";

fn refused(code: &str, body: &str) {
    let src = format!("{P}{body}\n");
    match check_source("t.elya", &src) {
        Ok(()) => panic!("should be refused, {code}:\n{src}"),
        Err(e) => assert!(e.contains(code), "should be {code}:\n{src}\n{e}"),
    }
}

fn accepted(body: &str) {
    let src = format!("{P}{body}\n");
    if let Err(e) = check_source("t.elya", &src) {
        panic!("should check clean:\n{src}\n{e}");
    }
}

#[test]
fn a_parameter_is_counted_like_a_let() {
    // U1, U2, U7, U8; S5.
    refused(
        "E0428",
        "fn dup(f: File) -> Int { read(f) + read(f) }\npub fn main() -> Int { dup(File(1)) }",
    );
    refused(
        "E0428",
        "fn dup(f) { read(f) + read(f) }\npub fn main() -> Int { dup(File(1)) }",
    );
    refused(
        "E0428",
        "fn loop(f: File, n: Int) -> Int { if n == 0 { read(f) } else { loop(f, n - 1) + read(f) } }\n\
         pub fn main() -> Int { loop(File(1), 2) }",
    );
    refused(
        "E0428",
        "fn dup(f: File) -> Int {\n  let g = f\n  read(g) + read(f)\n}\npub fn main() -> Int { dup(File(1)) }",
    );
    accepted("fn once(f: File) -> Int { read(f) }\npub fn main() -> Int { once(File(1)) }");
}

#[test]
fn the_arms_of_a_branch_count_the_larger() {
    // S1, S2, S4; and a use after a branch that used it.
    accepted(
        "fn pick(c: Bool) -> Int {\n  let f = File(1)\n  if c { read(f) } else { read(f) + 1 }\n}\n\
         pub fn main() -> Int { pick(True) }",
    );
    accepted(
        "type C { A, B }\nfn pick(c: C) -> Int {\n  let f = File(1)\n  match c { A -> read(f)  B -> read(f) + 1 }\n}\n\
         pub fn main() -> Int { pick(A) }",
    );
    accepted(
        "fn pick(c: Bool) -> Int {\n  let f = File(1)\n  if c { read(f) } else { 0 }\n}\n\
         pub fn main() -> Int { pick(True) }",
    );
    refused(
        "E0428",
        "fn pick(c: Bool) -> Int {\n  let f = File(1)\n  let n = if c { read(f) } else { 0 }\n  n + read(f)\n}\n\
         pub fn main() -> Int { pick(True) }",
    );
}

#[test]
fn a_closure_counts_as_often_as_it_is_called() {
    // U3, U5, a closure called by a closure called twice; S3, S7, a closure
    // never called.
    refused(
        "E0428",
        "pub fn main() -> Int {\n  let f = File(1)\n  let g = fn() { read(f) }\n  g() + g()\n}",
    );
    refused(
        "E0428",
        "pub fn main() -> Int {\n  let f = File(1)\n  let g = fn() { f }\n  read(g()) + read(g())\n}",
    );
    refused(
        "E0428",
        "pub fn main() -> Int {\n  let f = File(1)\n  let g = fn() { read(f) }\n  let k = fn() { g() }\n  k() + k()\n}",
    );
    accepted("pub fn main() -> Int {\n  let f = File(1)\n  let g = fn() { read(f) }\n  g()\n}");
    accepted(
        "fn pick(c: Bool) -> Int {\n  let f = File(1)\n  let g = fn() { read(f) }\n  if c { g() } else { g() + 1 }\n}\n\
         pub fn main() -> Int { pick(True) }",
    );
    accepted("pub fn main() -> Int {\n  let f = File(1)\n  let g = fn() { read(f) }\n  read(f)\n}");
}

#[test]
fn a_closure_passed_on_runs_many_times() {
    // U16, U19; an alias and a returned closure (refused by choice, D4).
    refused(
        "E0428",
        "fn twice(g) { g() + g() }\npub fn main() -> Int {\n  let f = File(1)\n  twice(fn() { read(f) })\n}",
    );
    refused(
        "E0428",
        "type Box(a) { Box(a) }\npub fn main() -> Int {\n  let f = File(1)\n  let b = Box(fn() { read(f) })\n  match b { Box(g) -> g() + g() }\n}",
    );
    refused(
        "E0428",
        "pub fn main() -> Int {\n  let f = File(1)\n  let g = fn() { read(f) }\n  let h = g\n  h()\n}",
    );
    refused(
        "E0428",
        "fn mk() {\n  let f = File(1)\n  fn() { read(f) }\n}\npub fn main() -> Int {\n  let g = mk()\n  g()\n}",
    );
}

#[test]
fn what_holds_an_affine_value_is_affine() {
    // U4, U9, a non-generic holder; a container of `Int`s used twice.
    refused(
        "E0428",
        "type Box(a) { Box(a) }\nfn open(b: Box(File)) -> Int { match b { Box(f) -> read(f) } }\n\
         pub fn main() -> Int {\n  let f = File(1)\n  let b = Box(f)\n  open(b) + open(b)\n}",
    );
    refused(
        "E0428",
        "type Box(a) { Box(a) }\npub fn main() -> Int {\n  match Box(File(1)) { Box(f) -> read(f) + read(f) }\n}",
    );
    refused(
        "E0428",
        "type H { H(File) }\nfn take(h: H) -> Int { match h { H(f) -> read(f) } }\n\
         pub fn main() -> Int {\n  let h = H(File(1))\n  take(h) + take(h)\n}",
    );
    accepted(
        "type Box(a) { Box(a) }\nfn get(b: Box(Int)) -> Int { match b { Box(n) -> n } }\n\
         pub fn main() -> Int {\n  let b = Box(1)\n  get(b) + get(b)\n}",
    );
}

#[test]
fn an_operation_clause_runs_once_per_perform() {
    // U11, U20, a clause parameter used twice; S9, a clause parameter used once.
    refused(
        "E0428",
        "effect Give { fn give() -> Int }\nfn two() / {Give} -> Int { give() + give() }\n\
         pub fn main() -> Int {\n  let f = File(1)\n  handle { two() } with { Give.give() -> resume(read(f)) }\n}",
    );
    refused(
        "E0428",
        "effect Give { fn give() -> Int }\npub fn main() -> Int {\n  let f = File(1)\n  \
         handle { read(f) } with { Give.give() -> resume(1)  return(x) -> x + read(f) }\n}",
    );
    refused(
        "E0428",
        "effect Put { fn put(f: File) -> Int }\npub fn main() -> Int {\n  \
         handle { put(File(1)) } with { Put.put(f) -> resume(read(f) + read(f)) }\n}",
    );
    accepted(
        "effect Give { fn give() -> Int }\npub fn main() -> Int {\n  let f = File(1)\n  \
         handle { give() } with { Give.give() -> resume(1)  return(x) -> x + read(f) }\n}",
    );
    accepted(
        "effect Put { fn put(f: File) -> Int }\npub fn main() -> Int {\n  \
         handle { put(File(1)) } with { Put.put(f) -> resume(read(f)) }\n}",
    );
}

#[test]
fn a_generic_function_carries_its_duplication_in_its_type() {
    // U15, U17, U18, through two generic functions: E0437 at the call; S8,
    // and the same `twice` at `Int`.
    refused(
        "E0437",
        "fn twice(g, x) { g(x) + g(x) }\npub fn main() -> Int { twice(read, File(1)) }",
    );
    refused(
        "E0437",
        "fn dupper() { fn(x) {\n  let _ = x\n  x\n} }\npub fn main() -> Int {\n  let d = dupper()\n  read(d(File(1)))\n}",
    );
    refused(
        "E0437",
        "fn mk(x) { fn() { x } }\npub fn main() -> Int {\n  let g = mk(File(1))\n  read(g()) + read(g())\n}",
    );
    refused(
        "E0437",
        "fn pair(x) {\n  let _ = x\n  x\n}\nfn pass(y) { pair(y) }\npub fn main() -> Int { read(pass(File(1))) }",
    );
    accepted("fn app(g, x) { g(x) }\npub fn main() -> Int { app(read, File(1)) }");
    accepted(
        "fn twice(g, x) { g(x) + g(x) }\nfn inc(n: Int) -> Int { n + 1 }\npub fn main() -> Int { twice(inc, 1) }",
    );
}

#[test]
fn a_multi_shot_point_holds_what_its_body_uses_after_it() {
    // U12 (a callee performs `Flip`), U13 (a parameter, in its own function),
    // a closure called after the perform; S6 (the `handle` delimits), a value
    // consumed before the perform.
    refused(
        "E0429",
        &format!(
            "{FLIP}fn coin() / {{Flip}} -> Bool {{ flip() }}\npub fn main() -> Int {{\n  let f = File(1)\n  \
             handle {{\n    let _ = coin()\n    read(f)\n  }}{RESUME_TWICE}\n}}"
        ),
    );
    refused(
        "E0429",
        &format!(
            "{FLIP}fn body(f: File) / {{Flip}} -> Int {{\n  let _ = flip()\n  read(f)\n}}\n\
             pub fn main() -> Int {{\n  handle {{ body(File(1)) }}{RESUME_TWICE}\n}}"
        ),
    );
    refused(
        "E0429",
        &format!(
            "{FLIP}pub fn main() -> Int {{\n  let f = File(1)\n  let g = fn() {{ read(f) }}\n  \
             handle {{\n    let _ = flip()\n    g()\n  }}{RESUME_TWICE}\n}}"
        ),
    );
    accepted(&format!(
        "{FLIP}pub fn main() -> Int {{\n  let f = File(1)\n  \
         let n = handle {{ if flip() {{ 1 }} else {{ 2 }} }}{RESUME_TWICE}\n  n + read(f)\n}}"
    ));
    accepted(&format!(
        "{FLIP}pub fn main() -> Int {{\n  let f = File(1)\n  \
         handle {{\n    let n = read(f)\n    if flip() {{ n }} else {{ 0 }}\n  }}{RESUME_TWICE}\n}}"
    ));
}

#[test]
fn what_a_row_variable_holds_waits_for_a_multi_effect() {
    // U14, U24: a generic function holds across a call of a function value;
    // the requirement rides on that value's row and applies where it is
    // instantiated with `Flip`. The same functions over a pure value,
    // accepted (S10).
    refused(
        "E0437",
        &format!(
            "{FLIP}fn coin() / {{Flip}} -> Bool {{ flip() }}\nfn after(g, x) {{\n  let _ = g()\n  x\n}}\n\
             pub fn main() -> Int {{\n  handle {{ read(after(coin, File(1))) }}{RESUME_TWICE}\n}}"
        ),
    );
    refused(
        "E0437",
        &format!(
            "{EACH}{FLIP}fn pick(f: File) / {{Flip}} -> Int {{\n  let n = read(f)\n  if flip() {{ n }} else {{ 0 }}\n}}\n\
             pub fn main() -> Int {{\n  handle {{ each(Cons(File(1), Cons(File(2), Nil)), pick) }}{RESUME_TWICE}\n}}"
        ),
    );
    accepted(
        "fn yes() -> Bool { True }\nfn after(g, x) {\n  let _ = g()\n  x\n}\n\
         pub fn main() -> Int { read(after(yes, File(1))) }",
    );
    accepted(&format!(
        "{EACH}pub fn main() -> Int {{ each(Cons(File(1), Cons(File(2), Nil)), read) }}"
    ));
}

#[test]
fn with_any_handles_no_operation_taking_an_affine_value() {
    // D5: `op.perform(o)` performs `put(File(1))` again, its `File` with it.
    refused(
        "E0437",
        "effect Put { fn put(f: File) -> Int }\n\
         fn again(body) { handle { body() } with any { op(o) -> {\n  let _ = op.perform(o)\n  resume(op.perform(o))\n} } }\n\
         pub fn main() -> Int {\n  handle { again(fn() { put(File(1)) }) } with { Put.put(f) -> resume(read(f)) }\n}",
    );
}

#[test]
fn what_a_call_has_evaluated_is_held_by_a_later_argument() {
    // The review's U-A: a resume re-runs the rest of the arguments and the
    // call, the earlier arguments' values (bound or not) and the callee
    // reused.
    let two = "fn two(f: File, b: Bool) -> Int { if b { read(f) } else { read(f) + 10 } }\n";
    refused(
        "E0429",
        &format!("{FLIP}{two}pub fn main() -> Int {{\n  handle {{ two(File(1), flip()) }}{RESUME_TWICE}\n}}"),
    );
    refused(
        "E0429",
        &format!(
            "{FLIP}{two}pub fn main() -> Int {{\n  let f = File(1)\n  handle {{ two(f, flip()) }}{RESUME_TWICE}\n}}"
        ),
    );
    refused(
        "E0429",
        &format!(
            "{FLIP}type Box {{ Box(File, Bool) }}\nfn open(b: Box) -> Int {{ match b {{ Box(f, _) -> read(f) }} }}\n\
             pub fn main() -> Int {{\n  let f = File(1)\n  handle {{ open(Box(f, flip())) }}{RESUME_TWICE}\n}}"
        ),
    );
    refused(
        "E0429",
        &format!(
            "{FLIP}{two}pub fn main() -> Int {{\n  let f = File(1)\n  let g = fn(b) {{ two(f, b) }}\n  \
             handle {{ g(flip()) }}{RESUME_TWICE}\n}}"
        ),
    );
    // A binary operation's left side waits too, but it is never affine: a
    // value consumed before the perform leaves an `Int`.
    accepted(&format!(
        "{FLIP}pub fn main() -> Int {{\n  let f = File(1)\n  handle {{ read(f) + if flip() {{ 1 }} else {{ 2 }} }}{RESUME_TWICE}\n}}"
    ));
}

#[test]
fn a_local_generic_held_on_an_outer_row_is_not_generalized() {
    // The review's U-B: `keep` holds `x` across `g()`, whose row is the
    // enclosing function's; generalizing `keep` over `x`'s type lost the
    // requirement when `keep` was used at `File` before the row took `Flip`.
    let coin = "fn coin() / {Flip} -> Bool { flip() }\n";
    refused(
        "E0437",
        &format!(
            "{FLIP}{coin}fn outer(g) {{\n  let keep = fn(x) {{\n    let _ = g()\n    x\n  }}\n  read(keep(File(1)))\n}}\n\
             pub fn main() -> Int {{\n  handle {{ outer(coin) }}{RESUME_TWICE}\n}}"
        ),
    );
    refused(
        "E043",
        &format!(
            "{FLIP}{coin}pub fn main() -> Int {{\n  let h = fn(g) {{\n    let keep = fn(x) {{\n      let _ = g()\n      x\n    }}\n    keep\n  }}\n  \
             let k = h(coin)\n  handle {{ read(k(File(1))) }}{RESUME_TWICE}\n}}"
        ),
    );
    accepted(&format!(
        "{FLIP}fn outer(g) {{\n  let keep = fn(x) {{\n    let _ = g()\n    x\n  }}\n  read(keep(File(1)))\n}}\n\
         pub fn main() -> Int {{ outer(fn() {{ True }}) }}"
    ));
}

#[test]
fn a_closure_counts_its_captures_where_it_is_called() {
    // The review's C-1: a closure called in one arm, its capture used in the
    // other, is one use.
    accepted(
        "fn pick(c: Bool) -> Int {\n  let f = File(1)\n  let g = fn() { read(f) }\n  if c { g() } else { read(f) }\n}\n\
         pub fn main() -> Int { pick(True) }",
    );
    accepted(
        "fn pick(c: Bool) -> Int {\n  let f = File(1)\n  let g = fn() { read(f) }\n  let k = fn() { g() }\n  if c { k() } else { g() }\n}\n\
         pub fn main() -> Int { pick(True) }",
    );
    refused(
        "E0428",
        "fn pick(c: Bool) -> Int {\n  let f = File(1)\n  let g = fn() { read(f) }\n  let n = if c { g() } else { 0 }\n  n + read(f)\n}\n\
         pub fn main() -> Int { pick(True) }",
    );
}
