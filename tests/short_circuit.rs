//! `&&` and `||` short-circuit (spec 2026-11-15-elya-short-circuit): `a && b`
//! is `if a { b } else { False }` and `a || b` is `if a { True } else { b }`,
//! in both evaluators, natively (crates/codegen/tests/native_short_circuit.rs)
//! and to the affine checker.
//!
//! Negative controls: see the plan,
//! `docs/superpowers/plans/2026-11-15-elya-short-circuit.md`.

#[allow(dead_code)] // each suite uses some of the programs
#[path = "programs/short_circuit.rs"]
mod programs;
use programs::*;

use elya::parse::parse_module;
use elya::Session;

fn run(src: &str) -> elya::RunOutcome {
    elya::run_source_outcome_with("main.elya", src, &mut |_| None)
}

fn value(src: &str) -> String {
    let o = run(src);
    assert!(o.error.is_none(), "{:?}\n{src}", o.error);
    o.line.expect("a line").expect("data")
}

/// The tree evaluator's output, or its runtime error (it has no effects but
/// `IO`, which these programs keep to).
fn tree(src: &str) -> Result<String, String> {
    let (m, _, _) = elya::front_end_with("main.elya", src, &mut |_| None);
    elya::eval::run_module_tree(&m)
        .map(|i| i.output().to_string())
        .map_err(|e| e.diag.message)
}

#[test]
fn the_right_operand_runs_only_when_it_decides() {
    assert_eq!(value(SKIPPED), SKIPPED_VALUE);
    assert_eq!(tree(SKIPPED), Ok(String::new()));
}

#[test]
fn effects_on_the_right_happen_only_when_it_runs() {
    let o = run(ORDER);
    assert!(o.error.is_none(), "{:?}", o.error);
    assert_eq!(o.output, ORDER_OUT);
    assert_eq!(tree(ORDER), Ok(ORDER_OUT.to_string()));
}

#[test]
fn an_index_guard_keeps_the_read_in_bounds() {
    assert_eq!(value(GUARD), GUARD_VALUE);
}

#[test]
fn the_right_operand_is_in_tail_position() {
    // The CEK machine's continuation is on the heap, so the measure is its
    // peak depth (tests/tce.rs): the same at a hundred calls as at ten
    // thousand. A frame left under the right operand grows it by one a call.
    let peak = |n: &str| {
        let src = TAIL.replace("1000000", n);
        let (m, d) = parse_module(&Session::new(), &src);
        assert!(d.is_empty(), "parse: {d:?}");
        elya::eval::run_module(&m).expect("runs").peak_kont_depth()
    };
    let (small, big) = (peak("100"), peak("10000"));
    assert_eq!(
        small, big,
        "peak continuation depth at 100 and at 10000 calls"
    );
    assert_eq!(value(&TAIL.replace("1000000", "10000")), TAIL_VALUE);
}

/// The codes the checker reports, in order.
fn codes(src: &str) -> Vec<String> {
    let (_, diags, _) = elya::front_end_with("main.elya", src, &mut |_| None);
    diags.iter().map(|d| d.code.to_string()).collect()
}

/// To the affine checker `a && b` is the `if` it means: a linear value given
/// away on the right is given away on one path only.
#[test]
fn the_affine_checker_sees_the_if() {
    let eat = "fn eat(xs: Array(Int)) -> Bool { array.length(xs) > 0 }\n";
    let make = "  let xs = array.push(array.new(), 1)\n";
    let cases: &[(&str, &str, &str)] = &[
        // given away on the right, read after
        (
            "flag && eat(xs)",
            "if flag { eat(xs) } else { False }",
            "array.length(xs)",
        ),
        (
            "flag || eat(xs)",
            "if flag { True } else { eat(xs) }",
            "array.length(xs)",
        ),
        // given away on the right, not used after
        ("flag && eat(xs)", "if flag { eat(xs) } else { False }", "0"),
        // given away on the left: on every path
        (
            "eat(xs) && flag",
            "if eat(xs) { flag } else { False }",
            "array.length(xs)",
        ),
        ("eat(xs) || flag", "if eat(xs) { True } else { flag }", "0"),
        // read on the left, given away on the right
        (
            "array.length(xs) > 0 && eat(xs)",
            "if array.length(xs) > 0 { eat(xs) } else { False }",
            "0",
        ),
    ];
    for (op, iff, after) in cases {
        let prog = |e: &str| {
            format!("{eat}fn f(flag: Bool) -> Int {{\n{make}  let b = {e}\n  if b {{ {after} }} else {{ {after} }}\n}}\npub fn main() -> Int {{ f(True) }}\n")
        };
        let (a, b) = (prog(op), prog(iff));
        assert_eq!(codes(&a), codes(&b), "`{op}` against `{iff}`\n{a}");
    }
    // The first is refused, as the `if` is.
    let first = format!("{eat}fn f(flag: Bool) -> Int {{\n{make}  let b = flag && eat(xs)\n  if b {{ array.length(xs) }} else {{ 0 }}\n}}\npub fn main() -> Int {{ f(True) }}\n");
    assert!(
        codes(&first).iter().any(|c| c == "E0438"),
        "{:?}",
        codes(&first)
    );
}

/// The parse is unchanged: still a `Binary` (the dump the parser in Elya is
/// held to).
#[test]
fn the_tree_still_holds_the_operator() {
    let src = "fn f(a: Bool, b: Bool) -> Bool { a && b || a }\n";
    let (m, d) = parse_module(&Session::new(), src);
    assert!(d.is_empty());
    let dump = elya::ast_dump::dump(&m, src);
    assert!(
        dump.contains("Binary") && dump.contains("&&") && dump.contains("||"),
        "{dump}"
    );
}

#[test]
fn an_effect_on_the_right_is_performed_only_when_it_runs() {
    assert_eq!(value(MULTI), MULTI_VALUE);
}

/// The affine checker's codes on `&&` and `||` equal its codes on the `if`s
/// they mean, with a multi-shot point, a `linear` type and a `borrow`
/// parameter (the review's pairs, 2026-11-15).
#[test]
fn the_affine_checker_sees_the_if_with_points_and_borrows() {
    let pre = "linear type File { File(Int) }\nfn read(f: File) -> Int { match f { File(n) -> n } }\neffect multi Flip { fn flip() -> Bool }\nfn eat(xs: Array(Int)) -> Bool { array.length(xs) > 0 }\n";
    let rt = " with multi { Flip.flip() -> resume(True) + resume(False) }";
    let main = |body: &str| {
        format!("{pre}pub fn main() -> Int {{\n  let f = File(1)\n  handle {{ if {body} {{ 1 }} else {{ 0 }} }}{rt}\n}}\n")
    };
    let pairs = [
        // the point on the right, the linear value used on the left
        (main("read(f) > 0 && flip()"), main("(if read(f) > 0 { flip() } else { False })"), "none"),
        // the point on the left, the linear value used on the right
        (main("flip() && read(f) > 0"), main("(if flip() { read(f) > 0 } else { False })"), "E0429"),
        // the linear value used on both sides
        (
            main("read(f) > 0 || read(f) > 1"),
            main("(if read(f) > 0 { True } else { read(f) > 1 })"),
            "E0428",
        ),
        // a borrow parameter given away on the right
        (
            format!("{pre}fn h(borrow xs: Array(Int), flag: Bool) -> Bool {{ flag && eat(xs) }}\npub fn main() -> Int {{\n  let xs = array.new()\n  if h(xs, True) {{ 1 }} else {{ 0 }}\n}}\n"),
            format!("{pre}fn h(borrow xs: Array(Int), flag: Bool) -> Bool {{ if flag {{ eat(xs) }} else {{ False }} }}\npub fn main() -> Int {{\n  let xs = array.new()\n  if h(xs, True) {{ 1 }} else {{ 0 }}\n}}\n"),
            "E0439",
        ),
    ];
    for (op, iff, want) in &pairs {
        let (a, b) = (codes(op), codes(iff));
        assert_eq!(a, b, "{op}\nagainst\n{iff}");
        if *want == "none" {
            assert!(a.is_empty(), "{a:?}\n{op}");
        } else {
            assert!(a.iter().any(|c| c == want), "{a:?} lacks {want}\n{op}");
        }
    }
}
