//! A `let` generalized by levels quantifies what asking the environment did
//! (2026-11-05, spec `docs/superpowers/specs/2026-10-31-elya-scalability-design.md`
//! D14). Every program here is checked under `elya::types::check_levels`,
//! which computes each `let`'s quantified variables both ways, by levels and
//! from the environment's open entries (D2), and panics where they differ:
//! the examples, the ui tests, the row-parameter program, and programs written
//! for what levels must get right. With `ELYA_CHECK_LEVELS=1` set, every
//! inference in a test run is checked the same way (done over the root and
//! codegen suites when levels came in; the complexity tests then fail by
//! design, the environment read again at every `let`).
//!
//! Negative controls (each reverted; the file compared byte for byte after):
//! - K23, a non-value `let` leaving its type's variables where they were
//!   made: the written programs fail (a closure over its value quantified
//!   them);
//! - K24, `bind` not lowering what it binds to: the written programs and the
//!   open shapes fail;
//! - K25, `bind_row` not lowering: the same;
//! - K27, a `let` quantifying its own level too: the same;
//! - K30, a `let` not lowering what it leaves unquantified: the review's
//!   programs fail;
//! - K31, an annotation variable's rows not kept: the review's programs and
//!   both refusal tests fail;
//! - K32, a `let`'s annotation checked outside its value's level: the
//!   review's programs fail;
//! - K33, `resume` types left out of what is kept: the same.

fn no_files(_: &str) -> Option<(String, String)> {
    None
}

/// Every `let` of `src` generalized both ways (a panic if they differ); what
/// checking it reported.
fn check(name: &str, src: &str) -> Result<(), String> {
    elya::types::check_levels(|| elya::check_source_with(name, src, &mut no_files))
}

/// On a thread with room for the deepest example (the evaluator-free front
/// end still recurses on the host stack).
fn on_a_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("levels and the environment agree");
}

fn files_in(dir: &str) -> Vec<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
    let mut out: Vec<_> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("{}: {e}", root.display()))
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "elya"))
        .collect();
    out.sort();
    out
}

#[test]
fn the_examples_ui_tests_and_row_parameters_generalize_alike() {
    on_a_big_stack(|| {
        let mut n = 0;
        for dir in ["examples", "tests/ui", "tests/row_parameters"] {
            for path in files_in(dir) {
                let text = std::fs::read_to_string(&path).expect("read");
                // A ui test may fail to check: that is what it tests.
                let _ = check(&path.display().to_string(), &text);
                n += 1;
            }
        }
        assert!(n >= 28, "only {n} files");
    });
}

/// (what it is, the program).
const CASES: &[(&str, &str)] = &[
    (
        "let-polymorphism",
        "pub fn main() -> Int {\n\
         \x20 let id = fn(x) { x }\n\
         \x20 let s = id(\"a\")\n\
         \x20 id(1) + string.length(s)\n\
         }\n",
    ),
    (
        "a closure over a non-value let (the value restriction)",
        "type L(a) { Nil, Cons(a, L(a)) }\n\
         fn nil() { Nil }\n\
         pub fn main() -> Int {\n\
         \x20 let r = nil()\n\
         \x20 let g = fn() { r }\n\
         \x20 let h = fn(x) { Cons(x, g()) }\n\
         \x20 match h(1) { Nil -> 0  Cons(y, _) -> y }\n\
         }\n",
    ),
    (
        "a closure over a parameter, the parameter bound after it",
        "fn f(p) {\n\
         \x20 let g = fn() { p }\n\
         \x20 let q = g() + 1\n\
         \x20 let h = fn(y) { y + q }\n\
         \x20 h(p)\n\
         }\n\
         pub fn main() -> Int { f(1) }\n",
    ),
    (
        "a parameter's type fixed inside a let",
        "fn f(p) {\n\
         \x20 let h = fn(y) { p(y) }\n\
         \x20 h(1) + p(2)\n\
         }\n\
         pub fn main() -> Int { f(fn(x) { x + 1 }) }\n",
    ),
    (
        "a parameter's row grown inside a let",
        "effect Log { fn log(s: String) -> Unit }\n\
         fn needs_log(h: fn() / {Log | e} -> Int) / {Log | e} -> Int { log(\"n\")  h() }\n\
         fn f(k) / {Log} -> Int {\n\
         \x20 let a = k()\n\
         \x20 let g = fn() { needs_log(k) }\n\
         \x20 g() + a\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 handle { f(fn() { 3 }) } with { Log.log(s) -> resume(Unit) }\n\
         }\n",
    ),
    (
        "lets nested in lambdas nested in lets",
        "type T(a, b, c) { T(a, b, c) }\n\
         pub fn main() -> Int {\n\
         \x20 let outer = fn(a) {\n\
         \x20   let inner = fn(b) { let three = fn(c) { T(a, b, c) }  three }\n\
         \x20   inner\n\
         \x20 }\n\
         \x20 let t = outer(1)(\"x\")(True)\n\
         \x20 match t { T(x, _, _) -> x }\n\
         }\n",
    ),
    (
        "closures over a function parameter (rows)",
        "fn f(k) {\n\
         \x20 let g1 = fn() { k() + 1 }\n\
         \x20 let g2 = fn() { g1() + k() }\n\
         \x20 let k2 = k\n\
         \x20 g2() + k2()\n\
         }\n\
         pub fn main() -> Int { f(fn() { 1 }) }\n",
    ),
    (
        "a let in a handler clause (the resume types)",
        "effect Ask { fn ask() -> Int }\n\
         fn f() / {Ask} -> Int { ask() + ask() }\n\
         pub fn main() -> Int {\n\
         \x20 handle { f() } with {\n\
         \x20   Ask.ask() -> { let again = fn(x) { resume(x) }  again(20) }\n\
         \x20 }\n\
         }\n",
    ),
    (
        "annotation variables",
        "type P(a, b) { P(a, b) }\n\
         fn keep(x: a) -> a {\n\
         \x20 let g = fn() { x }\n\
         \x20 let both = fn(y) { P(g(), y) }\n\
         \x20 match both(1) { P(z, _) -> z }\n\
         }\n\
         pub fn main() -> Int { keep(7) }\n",
    ),
    (
        "an effectful closure under a handler, generalized inside",
        "effect Log { fn log(s: String) -> Unit }\n\
         fn run(k) / {Log} -> Int {\n\
         \x20 let twice = fn(m) { log(m)  log(m)  k() }\n\
         \x20 let again = twice\n\
         \x20 again(\"a\") + twice(\"b\")\n\
         }\n\
         pub fn main() -> Int {\n\
         \x20 handle { run(fn() { 3 }) } with { Log.log(s) -> resume(Unit) }\n\
         }\n",
    ),
];

#[test]
fn programs_written_for_levels_generalize_alike() {
    on_a_big_stack(|| {
        for (what, src) in CASES {
            // Checked clean, so inference ran to its end.
            check(what, src).unwrap_or_else(|e| panic!("{what}: {e}"));
        }
    });
}

/// The shapes `tests/infer_complexity.rs` times, small, checked.
#[test]
fn the_open_binding_shapes_generalize_alike() {
    on_a_big_stack(|| {
        let mut closures = String::from("fn f(k) {\n");
        let mut aliases = String::from("fn f(k) {\n  let k0 = k\n  let a0 = 0\n");
        for i in 1..30 {
            closures.push_str(&format!("  let g{i} = fn() {{ k() + {i} }}\n"));
            aliases.push_str(&format!(
                "  let k{i} = k{}\n  let a{i} = (k{i}() + a{}) % 1000003\n",
                i - 1,
                i - 1
            ));
        }
        closures.push_str("  g29()\n}\npub fn main() -> Int { f(fn() { 1 }) }\n");
        aliases.push_str("  a29\n}\npub fn main() -> Int { f(fn() { 1 }) }\n");
        check("closures", &closures).expect("closures check");
        check("aliases", &aliases).expect("aliases check");
    });
}

/// Programs from the review of part 4, each one where generalizing by level
/// first differed from the environment: a variable kept monomorphic (an
/// annotation variable's, a `resume` type's) left above the `let`'s level,
/// and quantified by a later `let`. Each checks clean, as it did before
/// levels.
const REVIEW: &[(&str, &str)] = &[
    (
        // The review of part 4, a1: an annotation variable on a let, its row met inside a later let.
        "an annotation variable on a let, its row met inside a later let",
        "fn m(n: Int) -> Int {\n\
         \x20 let f: b = fn(x) { x + 1 }\n\
         \x20 let w = fn() {\n\
         \x20   let g = fn(h: b) { h(1) }\n\
         \x20   g(f)\n\
         \x20 }\n\
         \x20 w()\n\
         }\n\
         pub fn main() -> Int { m(1) }\n",
    ),
    (
        // The review of part 4, c1: a group member called from a lambda in a let.
        "a group member called from a lambda in a let",
        "effect St(s) { fn put() -> Unit }\n\
         fn m1(p0, p1) / {} {\n\
         \x20 let h = fn(y) {\n\
         \x20   let f = put()\n\
         \x20   m1(7, y)\n\
         \x20 }\n\
         \x20 let k = fn() { h(0) }\n\
         \x20 0\n\
         }\n\
         pub fn main() -> Int { 0 }\n",
    ),
    (
        // The review of part 4, c4: the same through an effect argument.
        "the same through an effect argument",
        "effect St(s) { fn put(v: s) -> Unit }\n\
         fn m1(p0, p1) {\n\
         \x20 let h = fn(y) {\n\
         \x20   put(fn() { y })\n\
         \x20   m1(7, y)\n\
         \x20 }\n\
         \x20 let k = fn() { h(0) }\n\
         \x20 0\n\
         }\n\
         pub fn main() -> Int { 0 }\n",
    ),
    (
        // The review of part 4, c5: the same, handled, a clause binding a let.
        "the same, handled, a clause binding a let",
        "effect St(s) { fn put(v: s) -> Unit }\n\
         effect Log { fn log(s: String) -> Unit }\n\
         fn same(a, b) { a }\n\
         fn m1(p0, p1) {\n\
         \x20 let h = fn(y) {\n\
         \x20   put(fn() { y })\n\
         \x20   if p0 > 0 { m1(p0 - 1, y) } else { 0 }\n\
         \x20 }\n\
         \x20 let k = fn() { h(p1) }\n\
         \x20 let r = handle { h(2) } with { St.put(v) -> { let w = same(v, fn() { log(\"x\")  1 })  resume(Unit) } }\n\
         \x20 r + k()\n\
         }\n\
         pub fn main() -> Int { handle { handle { m1(1, 2) } with { St.put(v) -> resume(Unit) } } with { Log.log(s) -> resume(Unit) } }\n",
    ),
    (
        // The review of part 4, d2: the same at top level, run.
        "the same at top level, run",
        "effect St(s) { fn put(v: s) -> Unit }\n\
         effect Log { fn log(s: String) -> Unit }\n\
         fn same(a, b) { a }\n\
         fn m1(p0, p1) {\n\
         \x20 let h = fn(y) {\n\
         \x20   put(fn() { y })\n\
         \x20   if p0 > 0 { m1(p0 - 1, y) } else { 0 }\n\
         \x20 }\n\
         \x20 let k = fn() { h(p1) }\n\
         \x20 handle { k() } with {\n\
         \x20   St.put(v) -> {\n\
         \x20     let w = same(v, fn() { log(\"in\")  2 })\n\
         \x20     resume(Unit) + w()\n\
         \x20   }\n\
         \x20 }\n\
         }\n\
         pub fn main() / {IO} -> Unit {\n\
         \x20 let r = handle { handle { m1(1, 5) } with { St.put(v) -> resume(Unit) } } with { Log.log(s) -> { io.println(s)  resume(Unit) } }\n\
         \x20 io.println(int.to_string(r))\n\
         }\n",
    ),
    (
        // The review of part 4's mutant M4 (the annotation checked outside
        // the value's level) went unseen but for this: its fresh variables
        // are the value's own.
        "an annotation's own variables",
        "fn m0(p0: Int) -> Int {\n\
         \x20 let k: fn(Unit) -> a = fn(y) { 0 }\n\
         \x20 0\n\
         }\n\
         pub fn main() -> Int { m0(1) }\n",
    ),
    (
        // Its mutant M10 (`resume` types left out of what is kept): a
        // clause's lambda resuming with a function the handle answers.
        "a clause's lambda resuming with a function",
        "effect Flip { fn flip() -> Bool }\n\
         fn app() {\n\
         \x20 let h = fn() { 0 }\n\
         \x20 handle { h } with {\n\
         \x20   flip() -> {\n\
         \x20     let _ = fn() { resume(True) }\n\
         \x20     fn() { 0 }\n\
         \x20   }\n\
         \x20 }\n\
         }\n\
         pub fn main() -> Int { app()() }\n",
    ),
];

#[test]
fn the_review_programs_generalize_alike() {
    on_a_big_stack(|| {
        for (what, src) in REVIEW {
            check(what, src).unwrap_or_else(|e| panic!("{what}: {e}"));
        }
    });
}

/// An annotation variable is one type for the whole function, its row
/// included (the review of part 4): `b`, an arrow once `f` is bound, takes
/// `Ask` from the function `g` is applied to inside the handle, so `f(1)`,
/// outside it, may perform `Ask`. Refused (E0420). Before levels it checked
/// clean in a release build: the environment's answer quantified `b`'s row in
/// `f`'s scheme, and a debug build stopped at D2's assertion ("a quantified
/// row variable came to hold a row") when `Ask` arrived.
#[test]
fn an_annotation_variables_row_is_one_row_for_the_function() {
    on_a_big_stack(|| {
        let err = check(
            "b2",
            "effect Ask { fn ask() -> Int }\n\
         fn m(n: Int) -> Int {\n\
         \x20 let f: b = fn(x) { x + 1 }\n\
         \x20 let w = fn() {\n\
         \x20   let g = fn(h: b) { h(1) }\n\
         \x20   handle { g(fn(y) { ask() + y }) } with { Ask.ask() -> resume(2) }\n\
         \x20 }\n\
         \x20 w() + f(1)\n\
         }\n\
         pub fn main() -> Int { m(1) }\n",
        )
        .expect_err("Ask is performed outside the handle through b");
        assert!(err.contains("E0420"), "{err}");
    });
}

/// The review's a3: a pure function whose annotated `let` and a later one
/// hid a `Log` it performs. Refused (E0420), as before levels; checked clean
/// under the bug.
#[test]
fn an_effect_hidden_through_an_annotated_let_is_still_refused() {
    on_a_big_stack(|| {
        let err = check(
            "a3",
            "effect Log { fn log(s: String) -> Unit }\n\
         fn m(n: Int) -> Int {\n\
         \x20 let f: b = fn(x) { x + 1 }\n\
         \x20 let w = fn() {\n\
         \x20   let g = fn(h: b) { h(1) }\n\
         \x20   g(fn(x) { log(\"a\")  x })\n\
         \x20 }\n\
         \x20 w()\n\
         }\n\
         pub fn main() -> Int { m(1) }\n",
        )
        .expect_err("Log is never handled");
        assert!(err.contains("E0420"), "{err}");
    });
}
