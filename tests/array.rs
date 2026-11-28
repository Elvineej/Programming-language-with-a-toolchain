// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! Self-hosting, part 2b (2026-11-14, spec
//! `docs/superpowers/specs/2026-11-14-elya-arrays-and-borrowing-design.md`): a
//! linear `Array(a)` written in place and read by borrowing; a borrow counts
//! no use, takes place at its call, and never after the use that consumes.
//!
//! Negative controls: see the plan,
//! `docs/superpowers/plans/2026-11-14-elya-arrays-and-borrowing.md`.

#[allow(dead_code)] // each suite uses some of the programs
#[path = "programs/array.rs"]
mod programs;
use programs::*;

fn run(src: &str) -> elya::RunOutcome {
    elya::run_source_outcome_with("main.elya", src, &mut |_| None)
}

fn errors(src: &str) -> String {
    match elya::check_source_with("main.elya", src, &mut |_| None) {
        Ok(()) => String::new(),
        Err(e) => e,
    }
}

/// A program around `body`, with an array `xs` of one element in scope.
fn with_xs(decls: &str, body: &str) -> String {
    format!(
        "fn one() -> Array(Int) {{ array.push(array.new(), 1) }}\n\
         fn eat(xs: Array(Int)) -> Int {{ array.length(xs) }}\n\
         fn tot(borrow a: Array(Int)) -> Int {{ array.length(a) }}\n\
         {decls}\npub fn main() -> Int {{\n  let xs = one()\n{body}\n}}\n"
    )
}

#[test]
fn arrays_hold_what_was_pushed_and_set() {
    let o = run(ARRAYS);
    assert!(o.error.is_none(), "{:?}", o.error);
    assert_eq!(o.output, ARRAYS_OUT);
    assert_eq!(o.line, Some(Ok(ARRAYS_VALUE.to_string())));
}

#[test]
fn an_index_out_of_range_stops_by_name() {
    for (src, message) in OUT_OF_RANGE {
        let o = run(src);
        let e = o.error.expect("fails");
        assert!(e.contains(message), "{src}: {e}");
        assert_eq!(o.output, "before\n", "{src}");
    }
}

#[test]
fn a_read_borrows_and_borrows_before_the_consuming_use_are_accepted() {
    for (decls, body) in [
        // Read any number of times, then consumed.
        ("", "  let a = array.get(xs, 0)\n  let b = array.get(xs, 0) + array.length(xs)\n  a + b + eat(xs)"),
        // A read in a later argument of the call that consumes: the array
        // is given away when that call is made.
        ("", "  array.length(array.push(xs, array.get(xs, 0)))"),
        ("", "  array.length(array.set(xs, array.length(xs) - 1, array.get(xs, 0) + 1))"),
        // Read in both arms, consumed in both arms after.
        (
            "",
            "  let n = if array.length(xs) == 1 { array.get(xs, 0) } else { array.length(xs) }\n  let ys = if n == 1 { array.push(xs, 2) } else { array.set(xs, 0, 5) }\n  array.length(ys) + n",
        ),
        // A `borrow` parameter read twice and passed on.
        (
            "fn twice(borrow a: Array(Int)) -> Int { tot(a) + array.get(a, 0) + tot(a) }",
            "  twice(xs) + twice(xs) + eat(xs)",
        ),
        // `borrow` on a duplicable type changes nothing; such a function is a value.
        (
            "fn inc(borrow n: Int) -> Int { n + n }\nfn app(f: fn(Int) -> Int) -> Int { f(1) }",
            "  app(inc) + inc(eat(xs))",
        ),
        // A generic borrow, at an array and at a number.
        (
            "fn first(borrow ys: Array(a)) -> a { array.get(ys, 0) }\nfn same(borrow x: a) -> a { x }",
            "  first(xs) + same(3) + eat(xs)",
        ),
        // A lambda that reads its capture, run once.
        ("", "  let f = fn() { array.length(xs) }\n  f()"),
    ] {
        assert_eq!(errors(&with_xs(decls, body)), "", "{decls}\n{body}");
    }
}

#[test]
fn a_read_after_the_consuming_use_is_refused() {
    for (decls, body, code, words) in [
        // In sequence.
        (
            "",
            "  let ys = array.push(xs, 2)\n  array.length(xs) + array.length(ys)",
            "E0438",
            "`xs` is read here after it was given away",
        ),
        // ChatGPT's counterexamples to the first draft of D3: a borrow takes
        // place at its call, after the call's arguments.
        ("", "  array.get(xs, eat(xs))", "E0438", "given away here"),
        (
            "fn walk(borrow a: Array(Int), b: Array(Int), i: Int) -> Int { array.length(b) + i }",
            "  walk(xs, xs, 1)",
            "E0438",
            "read here",
        ),
        (
            "",
            "  array.get(xs, if array.length(xs) == 1 { eat(xs) } else { 0 })",
            "E0438",
            "",
        ),
        (
            "effect Ask { fn ask() -> Int }",
            "  array.get(xs, handle { ask() } with { Ask.ask() -> resume(eat(xs)) })",
            "E0428",
            "used in an operation clause",
        ),
        // After an `if` one arm of which consumed.
        (
            "",
            "  let n = if array.length(xs) == 1 { eat(xs) } else { 0 }\n  n + array.length(xs)",
            "E0438",
            "",
        ),
        // Given to a lambda, then read: the lambda may run whenever.
        (
            "",
            "  let f = fn() { eat(xs) }\n  let n = array.length(xs)\n  n + f()",
            "E0438",
            "",
        ),
        // Through a function value the argument is consumed.
        (
            "",
            "  let g = fn(a: Array(Int)) { tot(a) }\n  let n = g(xs)\n  n + array.length(xs)",
            "E0438",
            "",
        ),
        (
            "",
            "  let g = fn(a: Array(Int)) { tot(a) }\n  g(xs) + g(xs)",
            "E0428",
            "used more than once",
        ),
        // A borrow in a lambda is a use of the capture.
        (
            "",
            "  let f = fn() { array.length(xs) }\n  f() + f()",
            "E0428",
            "used more than once",
        ),
        (
            "",
            "  let f = fn() { array.length(xs) }\n  f() + eat(xs)",
            "E0428",
            "",
        ),
    ] {
        let e = errors(&with_xs(decls, body));
        assert!(e.contains(code), "{body}: wanted {code} in {e}");
        assert!(e.contains(words), "{body}: wanted `{words}` in {e}");
    }
    // Held across a point that may resume twice.
    let e = errors(
        "effect multi Flip { fn flip() -> Bool }\n\
         pub fn main() {\n  let xs = array.push(array.new(), 1)\n  io.println(int.to_string(handle {\n    let _ = flip()\n    array.length(xs)\n  } with multi { Flip.flip() -> resume(True) }))\n}\n",
    );
    assert!(e.contains("E0429"), "{e}");
}

/// ChatGPT's second review: a continuation that escapes its handler (a
/// `resume` in a lambda a clause returns) lets what follows the `handle` run
/// while a borrow in its body is pending; resumed later, the read saw the
/// write (99). In such a handler's body a binding from outside is never
/// borrowed: the read counts as a use (spec D4).
#[test]
fn an_escaping_continuation_does_not_keep_a_borrow() {
    let outcome =
        "type Outcome { Done(Int), Later(fn() -> Outcome) }\neffect Pause { fn pause() -> Int }\n";
    let escaping = "with { Pause.pause() -> Later(fn() { resume(0) }) }";
    let later =
        "match pending { Done(n) -> n  Later(k) -> match k() { Done(n) -> n  Later(_) -> 0 } }";
    let first = "effect First { fn first() -> Int }\neffect Second { fn second() -> Int }";
    for (decls, body, code) in [
        // Round three, A1: a clause resumes directly, but its argument
        // performs to a nested handler that exports the continuation.
        (
            first,
            format!("  let pending = handle {{ Done(array.get(xs, first())) }} with {{ First.first() -> handle {{ resume(second()) }} with {{ Second.second() -> Later(fn() {{ resume(0) }}) }} }}\n  let u = array.set(xs, 0, 99)\n  let _ = array.length(u)\n  {later}"),
            "E0428",
        ),
        // A2: a nested handler's return clause exports it.
        (
            first,
            format!("  let pending = handle {{ Done(array.get(xs, first())) }} with {{ First.first() -> handle {{ 0 }} with {{ return(v) -> Later(fn() {{ resume(v) }}) }} }}\n  let u = array.set(xs, 0, 99)\n  let _ = array.length(u)\n  {later}"),
            "E0428",
        ),
        // A4: a `with any` clause is a clause.
        (
            "",
            format!("  let pending = handle {{ Done(array.get(xs, pause())) }} with any {{ op(o) -> Later(fn() {{ resume(op.decode(o, \"0\")) }}) }}\n  let u = array.set(xs, 0, 99)\n  let _ = array.length(u)\n  {later}"),
            "E0428",
        ),
        // C1: a `borrow` parameter through three functions.
        (
            "fn leaf(borrow a: Array(Int), i: Int) -> Int { array.get(a, i) }\nfn middle(borrow b: Array(Int), i: Int) -> Int { leaf(b, i) }\nfn probe3(borrow c: Array(Int)) -> Int { middle(c, pause()) }\nfn attempt(borrow a: Array(Int)) -> Outcome { handle { Done(probe3(a)) } with { Pause.pause() -> Later(fn() { resume(0) }) } }",
            format!("  let pending = attempt(xs)\n  let u = array.set(xs, 0, 99)\n  let _ = array.length(u)\n  {later}"),
            "E0439",
        ),
        // The review's Finding 1: the read pending across the perform.
        (
            "",
            format!("  let pending = handle {{ Done(array.get(xs, pause())) }} {escaping}\n  let u = array.set(xs, 0, 99)\n  let _ = array.length(u)\n  {later}"),
            "E0428",
        ),
        // Finding 2: the borrow in a callee's suspended frame.
        (
            "fn probe(borrow a: Array(Int)) -> Int {\n  let first = array.get(a, 0)\n  let _ = pause()\n  first * 100 + array.get(a, 0)\n}",
            format!("  let pending = handle {{ Done(probe(xs)) }} {escaping}\n  let u = array.set(xs, 0, 9)\n  let _ = array.length(u)\n  {later}"),
            "E0428",
        ),
        // The same, the handler in a function that borrows.
        (
            &format!("fn h(borrow a: Array(Int)) -> Outcome {{ handle {{ Done(array.get(a, pause())) }} {escaping} }}"),
            format!("  let pending = h(xs)\n  let u = array.set(xs, 0, 9)\n  let _ = array.length(u)\n  {later}"),
            "E0439",
        ),
    ] {
        let src = format!("{outcome}{}", with_xs(decls, &body));
        let e = errors(&src);
        assert!(e.contains(code), "{src}: wanted {code} in {e}");
    }
    // Held across a multi-shot point, even when the first resumption is cut
    // short by an abort (round three, B4).
    let e = errors(
        "effect multi Flip { fn flip() -> Int }\neffect Stop { fn stop() -> Int }\n\
         pub fn main() -> Int {\n  let xs = array.push(array.new(), 1)\n  handle {\n    handle {\n      let _ = array.get(xs, flip())\n      stop()\n    } with { Stop.stop() -> 0 }\n  } with multi { Flip.flip() -> {\n    let _ = resume(0)\n    resume(0)\n  } }\n}\n",
    );
    assert!(e.contains("E0429"), "{e}");
    // Accepted: a handler that only aborts (the read pending at the abort is
    // never made: round three, B2); one that resumes directly and answers a
    // closure (B3); an escaping one whose body reads nothing from outside;
    // one whose body owns the array.
    for (decls, body, value) in [
        (
            "effect Stop { fn stop() -> Int }",
            "  let n = handle { array.get(xs, stop()) } with { Stop.stop() -> 40 }\n  let ys = array.set(xs, 0, 99)\n  n + array.get(ys, 0)".to_string(),
            "139",
        ),
        (
            "effect Read { fn read() -> Int }",
            "  let f = handle { array.get(xs, read()) } with { Read.read() -> resume(0)  return(n) -> fn() { n } }\n  let ys = array.set(xs, 0, 99)\n  f() + array.get(ys, 0)".to_string(),
            "100",
        ),
        (
            "effect Fail { fn fail() -> Int }\nfn check(borrow a: Array(Int)) -> Int { if array.length(a) == 1 { fail() } else { array.get(a, 0) } }",
            "  let r = handle { check(xs) } with { Fail.fail() -> 0 - 1 }\n  let u = array.set(xs, 0, 9)\n  r + array.get(u, 0)".to_string(),
            "8",
        ),
        (
            "",
            format!("  let n = array.get(xs, 0)\n  let pending = handle {{ Done(n + pause()) }} {escaping}\n  let u = array.set(xs, 0, 9)\n  array.get(u, 0) + {later}"),
            "10",
        ),
        (
            "",
            format!("  let pending = handle {{\n    let ys = array.push(array.new(), 1)\n    let a = array.get(ys, pause())\n    let u = array.set(ys, 0, 9)\n    Done(a + array.get(u, 0) + eat(xs))\n  }} {escaping}\n  {later}"),
            "11",
        ),
    ] {
        let src = format!("{outcome}{}", with_xs(decls, &body));
        assert_eq!(errors(&src), "", "{src}");
        assert_eq!(run(&src).line, Some(Ok(value.to_string())), "{src}");
    }
}

/// The review's precision tests: shadowing, callees evaluated first, pattern
/// bindings, constructors and operations taking an array after a read.
#[test]
fn borrows_follow_bindings_not_names() {
    for (src, want) in [
        // A local `peek` shadows the borrowing one: its call consumes.
        ("fn peek(borrow a: Array(Int), i: Int) -> Int { array.get(a, i) }\npub fn main() -> Int {\n  let xs = array.push(array.new(), 1)\n  let peek = fn(a: Array(Int), i: Int) { array.get(array.set(a, i, 9), i) }\n  let _ = peek(xs, 0)\n  array.get(xs, 0)\n}\n", Err("E0438")),
        // The callee is evaluated before the arguments.
        ("fn h(xs: Array(Int)) -> fn(Int) -> Int {\n  let _ = array.length(array.push(xs, 2))\n  fn(i: Int) { i }\n}\npub fn main() -> Int {\n  let xs = array.push(array.new(), 1)\n  h(xs)(array.length(xs))\n}\n", Err("E0438")),
        ("fn make(n: Int) -> fn(Array(Int)) -> Int { fn(xs: Array(Int)) { n + array.length(xs) } }\npub fn main() -> Int {\n  let xs = array.push(array.new(), 1)\n  make(array.get(xs, 0))(xs)\n}\n", Ok("2")),
        // A pattern's binding is a binding.
        ("type Wrap { Wrap(Array(Int)) }\npub fn main() -> Int {\n  match Wrap(array.push(array.new(), 1)) { Wrap(xs) -> {\n    let next = array.set(xs, 0, 9)\n    let _ = array.length(next)\n    array.get(xs, 0)\n  } }\n}\n", Err("E0438")),
        ("type Wrap { Wrap(Array(Int)) }\npub fn main() -> Int {\n  match Wrap(array.push(array.new(), 1)) { Wrap(xs) -> {\n    let before = array.get(xs, 0)\n    before + array.get(array.set(xs, 0, 9), 0)\n  } }\n}\n", Ok("10")),
        // A constructor and an operation take the array when they are made.
        ("type Packed { Packed(Array(Int), Int) }\npub fn main() -> Int {\n  let xs = array.push(array.new(), 1)\n  match Packed(xs, array.length(xs)) { Packed(a, n) -> n + array.length(a) }\n}\n", Ok("2")),
        ("effect Put { fn put(xs: Array(Int), n: Int) -> Int }\npub fn main() -> Int {\n  let xs = array.push(array.new(), 1)\n  handle { put(xs, array.length(xs)) } with { Put.put(a, n) -> resume(n + array.get(array.set(a, 0, 9), 0)) }\n}\n", Ok("10")),
        // A local named like the builtin module.
        ("pub fn main() -> Int {\n  let array = fn(n: Int) { n + 1 }\n  let xs = array.push(array.new(), 1)\n  array.length(xs) + array(5)\n}\n", Ok("7")),
    ] {
        match want {
            Err(code) => {
                let e = errors(src);
                assert!(e.contains(code), "{src}: wanted {code} in {e}");
            }
            Ok(v) => {
                assert_eq!(errors(src), "", "{src}");
                assert_eq!(run(src).line, Some(Ok(v.to_string())), "{src}");
            }
        }
    }
}

#[test]
fn a_borrowed_parameter_is_only_read() {
    for (decls, code, words) in [
        (
            "fn keep(borrow a: Array(Int)) -> Array(Int) { a }",
            "E0439",
            "a borrowed value is given away here",
        ),
        (
            "fn used(borrow a: Array(Int)) -> Int { eat(a) }",
            "E0439",
            "`a` is a `borrow` parameter",
        ),
        (
            "fn grown(borrow a: Array(Int)) -> Int { array.length(array.push(a, 1)) }",
            "E0439",
            "",
        ),
        (
            "fn held(borrow a: Array(Int)) -> Int {\n  let f = fn() { array.length(a) }\n  f()\n}",
            "E0439",
            "",
        ),
        (
            "type Box { Box(Array(Int)) }\nfn boxed(borrow a: Array(Int)) -> Box { Box(a) }",
            "E0439",
            "",
        ),
        // A function that borrows is called by its name: its value says nothing.
        (
            "fn named() -> Int {\n  let g = tot\n  g(one())\n}",
            "E0439",
            "`tot` borrows its argument: it must be called by its name",
        ),
        (
            "fn app(f: fn(Array(Int)) -> Int) -> Int { f(one()) }\nfn passed() -> Int { app(tot) }",
            "E0439",
            "used as a value here",
        ),
        // A generic borrow given away, at an array.
        (
            "fn same(borrow x: a) -> a { x }\nfn at_array() -> Int { eat(same(one())) }",
            "E0437",
            "this would give away a borrowed value",
        ),
    ] {
        let e = errors(&with_xs(decls, "  eat(xs)"));
        assert!(e.contains(code), "{decls}: wanted {code} in {e}");
        assert!(e.contains(words), "{decls}: wanted `{words}` in {e}");
    }
}

#[test]
fn an_array_is_used_once_holds_no_affine_value_and_is_not_data() {
    for (src, code, words) in [
        (
            "pub fn main() -> Int {\n  let xs = array.push(array.new(), 1)\n  array.length(array.push(xs, 2)) + array.length(array.push(xs, 3))\n}\n",
            "E0428",
            "`xs` is an `Array`, which is `linear`",
        ),
        (
            "pub fn main() -> Int { array.length(array.push(array.new(), buffer.new())) }\n",
            "E0437",
            "this array would hold a `Buffer`",
        ),
        (
            "pub fn main() -> Int {\n  let xs: Array(Array(Int)) = array.new()\n  array.length(xs)\n}\n",
            "E0437",
            "an array's elements must not be affine",
        ),
        (
            "linear type File { File(Int) }\nfn made() -> Array(a) { array.new() }\npub fn main() -> Int { array.length(array.push(made(), File(1))) }\n",
            "E0437",
            "a `File` reaches a generic function here",
        ),
        (
            "pub fn main() -> Bool { array.push(array.new(), 1) == array.new() }\n",
            "E0436",
            "this compares arrays",
        ),
        (
            "type Holder { Holder(Array(Int)) }\npub fn main() -> Bool { Holder(array.new()) == Holder(array.new()) }\n",
            "E0436",
            "an `Array`",
        ),
        (
            "type Array(a) { Forged(Int, Int, Int, a) }\npub fn main() -> Int { array.get(Forged(0, 9, 9, 1), 5) }\n",
            "E0211",
            "`Array` is a builtin type",
        ),
        ("pub fn main() -> Array { array.new() }\n", "E0400", "expects 1 argument"),
        ("pub fn main() -> Int { array.get(\"text\", 0) }\n", "E0", "Array"),
        ("pub fn main() -> Int { array.length(array.push(array.push(array.new(), 1), \"two\")) }\n", "E0", "String"),
    ] {
        let e = errors(src);
        assert!(e.contains(code), "{src}: wanted {code} in {e}");
        assert!(e.contains(words), "{src}: wanted `{words}` in {e}");
    }
    // As main's value an array stops by name.
    let o = run("pub fn main() -> Array(Int) { array.push(array.new(), 1) }\n");
    assert_eq!(
        o.line,
        Some(Err(
            "main's value is an array, which is not data".to_string()
        ))
    );
}

#[test]
fn reading_an_array_by_index_is_linear_in_the_evaluator() {
    // Eight times the elements: linear is eight times the time; a list's
    // `nth` (or a push or a read that copied the array) sixty-four.
    let time = |n: u32| {
        let src = reads(n);
        let t = std::time::Instant::now();
        let o = run(&src);
        assert_eq!(o.line, Some(Ok(reads_value(n.into()))), "{:?}", o.error);
        t.elapsed().as_secs_f64()
    };
    let small = time(10_000).max(1e-3);
    let large = time(80_000);
    assert!(
        large / small < 14.0,
        "80,000 elements took {large:.3} s, 10,000 took {small:.3} s"
    );
}
