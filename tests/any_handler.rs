//! Operation-generic handlers, `with any` (2026-10-16, spec
//! `docs/superpowers/specs/2026-10-16-elya-any-handlers-design.md`). A handler that
//! handles every operation its body performs (but `IO`), whatever the effect, with the
//! operation as a typed value `o : Op(t)`: `op.name`, `op.show`, `op.perform`,
//! `op.encode` and `op.decode` on it, and `resume` at its answer type `t`.

use elya::parse::parse_module;
use elya::{check_source, run_source, Session};

fn parse_errors(src: &str) -> Vec<String> {
    let (_, d) = parse_module(&Session::new(), src);
    d.iter()
        .map(|d| format!("{}: {}", d.code, d.message))
        .collect()
}

fn errors(src: &str) -> String {
    match check_source("t.elya", src) {
        Ok(()) => String::new(),
        Err(e) => e,
    }
}

const WORLD: &str = "effect World { fn roll(sides: Int) -> Int }\n";

#[test]
fn any_handlers_parse() {
    for src in [
        // The operation clause and a return clause.
        "fn f(body) { handle { body() } with any {\n  op(o) -> resume(op.perform(o))\n  return(x) -> x\n} }\n",
        // The operation clause alone.
        "fn f(body) { handle { body() } with any { op(o) -> resume(op.perform(o)) } }\n",
    ] {
        let d = parse_errors(src);
        assert!(d.is_empty(), "{src}: {d:?}");
    }
    let (m, _) = parse_module(
        &Session::new(),
        "fn f(body) { handle { body() } with any { op(o) -> resume(op.perform(o)) } }\n",
    );
    let text = format!("{m:?}");
    assert!(text.contains("any: true"), "{text}");
}

#[test]
fn malformed_any_handlers_are_refused() {
    for (src, why) in [
        (
            "fn f(body) { handle { body() } with any { World.roll(s) -> resume(1) } }\n",
            "a named clause",
        ),
        (
            "fn f(body) { handle { body() } with any { roll(s) -> resume(1) } }\n",
            "an unqualified named clause",
        ),
        (
            "fn f(body) { handle { body() } with any { op(o) -> resume(1)  op(p) -> resume(2) } }\n",
            "two operation clauses",
        ),
        (
            "fn f(body) { handle { body() } with any { op(o, p) -> resume(1) } }\n",
            "two binders",
        ),
        (
            "fn f(body) { handle { body() } with any { op() -> resume(1) } }\n",
            "no binder",
        ),
        (
            "fn f(body) { handle { body() } with multi any { op(o) -> resume(1) } }\n",
            "multi",
        ),
        (
            "fn f(body) { handle { body() } with any { return(x) -> x } }\n",
            "no operation clause",
        ),
    ] {
        let d = parse_errors(src);
        assert!(
            d.iter().any(|e| e.starts_with("E0100")),
            "{why}: {d:?}"
        );
    }
    // A handler without `any` may still have an operation named `op`.
    let ok = format!(
        "effect E {{ fn op(x: Int) -> Int }}\nfn f() -> Int {{ handle {{ op(1) }} with {{ op(x) -> resume(x) }} }}\n{}",
        "pub fn main() -> Int { f() }\n"
    );
    assert_eq!(errors(&ok), "");
    let _ = (WORLD, run_source);
}

/// A forwarding recorder and a replayer, both generic: the recorder relays the body's
/// row (it performs every operation again), the replayer discharges it.
const RECORD: &str = "fn record(body) {\n\
  handle { body() } with any {\n\
    op(o) -> { let v = op.perform(o)  resume(v) }\n\
    return(x) -> x\n\
  }\n\
}\n\
fn replay(body) {\n\
  handle { body() } with any {\n\
    op(o) -> resume(op.decode(o, \"4\"))\n\
    return(x) -> x\n\
  }\n\
}\n";

fn schemes(src: &str) -> String {
    let (m, d) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    let (ss, diags) = elya::types::infer_schemes(&Session::new(), &m);
    assert!(diags.is_empty(), "type: {diags:?}");
    let mut lines: Vec<String> = ss.into_iter().map(|(n, s)| format!("{n} : {s}")).collect();
    lines.sort();
    lines.join("\n")
}

#[test]
fn an_any_handler_relays_or_discharges_the_whole_row() {
    let s = schemes(&format!("{WORLD}{RECORD}"));
    assert!(
        s.contains("record : forall a b. fn(fn() / {b} -> a) / {b} -> a"),
        "{s}"
    );
    assert!(
        s.contains("replay : forall a b. fn(fn() / {b} -> a) -> a"),
        "{s}"
    );
    // The recorder still needs a World handler outside; the replayer does not.
    let rec = format!(
        "{WORLD}{RECORD}pub fn main() -> Int {{ handle {{ record(fn() {{ roll(6) }}) }} with {{ World.roll(s) -> resume(4) }} }}\n"
    );
    assert_eq!(errors(&rec), "");
    let unhandled =
        format!("{WORLD}{RECORD}pub fn main() -> Int {{ record(fn() {{ roll(6) }}) }}\n");
    assert!(
        errors(&unhandled).contains("E0420"),
        "{}",
        errors(&unhandled)
    );
    let rep = format!("{WORLD}{RECORD}fn pure() / {{}} -> Int {{ replay(fn() {{ roll(6) }}) }}\npub fn main() -> Int {{ pure() }}\n");
    assert_eq!(errors(&rep), "");
}

#[test]
fn io_is_never_handled_by_an_any_handler() {
    // A concrete IO in the handled body stays in the residual: a pure function
    // may not hold it.
    let quiet = "fn quiet() / {} -> Int {\n\
  handle { io.println(\"x\")  roll(6) } with any {\n\
    op(o) -> resume(op.decode(o, \"4\"))\n\
  }\n\
}\npub fn main() -> Int { quiet() }\n";
    let e = errors(&format!("{WORLD}{quiet}"));
    assert!(e.contains("E0421") || e.contains("E0423"), "{e}");
    let loud = quiet.replace("/ {} -> Int", "/ {IO} -> Int");
    assert_eq!(errors(&format!("{WORLD}{loud}")), "");
    // The same when the clause never resumes: the body's IO happened anyway.
    let abort = quiet.replace("resume(op.decode(o, \"4\"))", "0");
    let e = errors(&format!("{WORLD}{abort}"));
    assert!(e.contains("E0421") || e.contains("E0423"), "{e}");
}

#[test]
fn an_answer_type_cannot_escape_its_clause() {
    for (src, why) in [
        (
            "fn leak(body) { handle { body() } with any { op(o) -> op.perform(o) } }\n",
            "through the handle's result",
        ),
        (
            "fn leak(body, seed) {\n  handle { body() } with any {\n    op(o) -> { let v = op.perform(o)  if 1 == 1 { resume(seed) } else { resume(v) } }\n  }\n}\n",
            "through a binding from outside the clause",
        ),
        (
            "fn leak(body) {\n  let run = handle { body() } with any {\n    op(o) -> fn(prev) { let v = op.perform(o)  resume(prev)(v) }\n    return(x) -> fn(prev) { x }\n  }\n  run\n}\n",
            "through the state-passing shape's parameter",
        ),
    ] {
        let e = errors(&format!("{WORLD}{src}pub fn main() {{ io.println(\"x\") }}\n"));
        assert!(e.contains("E0435"), "{why}: {e}");
    }
}

#[test]
fn op_builtins_take_an_operation() {
    let e = errors("pub fn main() { io.println(op.show(3)) }\n");
    assert!(e.contains("E0400"), "{e}");
    // A function given an operation relays the operation's row when it performs it.
    let s = schemes(&format!(
        "{WORLD}fn again(o) {{ op.perform(o) }}\nfn rec(body) {{ handle {{ body() }} with any {{ op(o) -> resume(again(o)) }} }}\n"
    ));
    assert!(
        s.contains("rec : forall a b. fn(fn() / {b} -> a) / {b} -> a"),
        "{s}"
    );
}

const MAKE: &str = "effect Mk { fn make() -> fn(Int) -> Int }\n";

#[test]
fn handled_operations_must_be_data() {
    // A function answer, at the handle itself and through a generic replayer.
    let at_handle = format!(
        "{MAKE}fn f() -> Int {{\n  let g = handle {{ make() }} with any {{ op(o) -> resume(op.decode(o, \"1\")) }}\n  g(1)\n}}\npub fn main() -> Int {{ f() }}\n"
    );
    assert!(
        errors(&at_handle).contains("E0434"),
        "{}",
        errors(&at_handle)
    );
    let through =
        format!("{MAKE}{RECORD}pub fn main() -> Int {{ replay(fn() {{ make() }})(1) }}\n");
    assert!(errors(&through).contains("E0434"), "{}", errors(&through));
    // Data answers are fine: declared types of data, at their arguments.
    let ok = format!(
        "type Opt(a) {{ No, Yes(a) }}\neffect Ask {{ fn ask(q: String) -> Opt(Int) }}\n{RECORD}\
         pub fn main() -> Int {{ match replay(fn() {{ ask(\"n\") }}) {{ No -> 0  Yes(n) -> n }} }}\n"
    );
    assert_eq!(errors(&ok), "");
    // A declared type with a function inside is not.
    let boxed = format!(
        "type Box {{ Box(fn(Int) -> Int) }}\neffect Ask {{ fn ask() -> Box }}\n{RECORD}\
         pub fn main() -> Int {{ match replay(fn() {{ ask() }}) {{ Box(g) -> g(1) }} }}\n"
    );
    assert!(errors(&boxed).contains("E0434"), "{}", errors(&boxed));
}

#[test]
fn io_reaching_a_generic_handler_is_e0434() {
    // IO is never handled (D2), so a generic replayer may not be given it.
    let src = format!(
        "{WORLD}{RECORD}pub fn main() -> Int {{ replay(fn() {{ io.println(\"x\")  roll(6) }}) }}\n"
    );
    assert!(errors(&src).contains("E0434"), "{}", errors(&src));
}

#[test]
fn a_generic_caller_is_checked_at_its_own_callers() {
    let wrap = "fn wrap(body) { replay(body) }\n";
    let ok =
        format!("{WORLD}{MAKE}{RECORD}{wrap}pub fn main() -> Int {{ wrap(fn() {{ roll(6) }}) }}\n");
    assert_eq!(errors(&ok), "");
    let bad = format!(
        "{WORLD}{MAKE}{RECORD}{wrap}pub fn main() -> Int {{ wrap(fn() {{ make() }})(1) }}\n"
    );
    assert!(errors(&bad).contains("E0434"), "{}", errors(&bad));
}

/// Two unrelated effects, one program; a generic recorder that prints each question
/// and its encoded answer through `Log`, forwarding to the real handlers outside.
const TWO: &str = "effect World { fn roll(sides: Int) -> Int }\n\
effect Ask { fn ask(q: String, n: Int) -> Opt(String) }\n\
effect Log { fn say(line: String) -> Unit }\n\
type Opt(a) { No, Yes(a) }\n\
fn program() / {World, Ask} -> Int {\n\
  let r = roll(6)\n\
  match ask(\"name \\\"q\\\"\", 0 - 3) { No -> r  Yes(s) -> r * 10 }\n\
}\n\
fn traced(body) {\n\
  handle { body() } with any {\n\
    op(o) -> {\n\
      let v = op.perform(o)\n\
      let _ = say(op.name(o) <> \" | \" <> op.show(o) <> \" = \" <> op.encode(o, v))\n\
      resume(v)\n\
    }\n\
  }\n\
}\n";

#[test]
fn a_generic_recorder_forwards_shows_and_encodes() {
    let src = format!(
        "{TWO}pub fn main() {{\n\
           handle {{\n\
             let x = handle {{\n\
               handle {{ traced(fn() {{ program() }}) }} with {{ World.roll(s) -> resume(4) }}\n\
             }} with {{ Ask.ask(q, n) -> resume(Yes(q <> \"!\")) }}\n\
             say(int.to_string(x))\n\
           }} with {{ Log.say(line) -> {{ io.println(line)  resume(Unit) }} }}\n\
         }}\n"
    );
    assert_eq!(errors(&src), "");
    assert_eq!(
        run_source("t.elya", &src).expect("runs"),
        "World.roll | World.roll(6) = 4\n\
         Ask.ask | Ask.ask(\"name \\\"q\\\"\", -3) = Yes(\"name \\\"q\\\"!\")\n\
         40\n"
    );
}

#[test]
fn a_generic_replayer_decodes_answers() {
    // Answers come from a list of literals, in order; the program runs without any
    // World or Ask handler.
    let src = format!(
        "{TWO}type Log2 {{ End, Ent(String, Log2) }}\n\
         fn replay(body, log) {{\n\
           let run = handle {{ body() }} with any {{\n\
             op(o) -> fn(l) {{ match l {{ Ent(a, rest) -> resume(op.decode(o, a))(rest)  End -> 0 - 1 }} }}\n\
             return(x) -> fn(l) {{ x }}\n\
           }}\n\
           run(log)\n\
         }}\n\
         pub fn main() {{\n\
           io.println(int.to_string(replay(fn() {{ program() }}, Ent(\"5\", Ent(\" Yes( \\\"a\\\\\\\"b\\\" ) \", End)))))\n\
           io.println(int.to_string(replay(fn() {{ program() }}, Ent(\"-2\", Ent(\"No\", End)))))\n\
         }}\n"
    );
    assert_eq!(errors(&src), "");
    assert_eq!(run_source("t.elya", &src).expect("runs"), "50\n-2\n");
}

#[test]
fn decoding_a_value_of_the_wrong_type_fails_by_name() {
    for (answer, why) in [
        ("\\\"4\\\"", "a String for an Int"),
        ("True", "a Bool for an Int"),
        ("Yes(1)", "a constructor for an Int"),
        ("4 4", "trailing input"),
        ("", "nothing"),
    ] {
        let src = format!(
            "{WORLD}fn replay(body) {{ handle {{ body() }} with any {{ op(o) -> resume(op.decode(o, \"{answer}\")) }} }}\n\
             pub fn main() {{ io.println(int.to_string(replay(fn() {{ roll(6) }}))) }}\n"
        );
        assert_eq!(errors(&src), "", "{why}");
        let err = run_source("t.elya", &src).expect_err(why);
        assert!(err.contains("op.decode"), "{why}: {err}");
    }
    // The answer type's constructors are checked through the declared type.
    let src = format!(
        "{TWO}type Other {{ Yes2(Int) }}\n\
         fn replay(body) {{ handle {{ body() }} with any {{ op(o) -> resume(op.decode(o, if op.name(o) == \"World.roll\" {{ \"1\" }} else {{ \"Yes(4)\" }})) }} }}\n\
         pub fn main() {{ io.println(int.to_string(replay(fn() {{ program() }}))) }}\n"
    );
    let err = run_source("t.elya", &src).expect_err("Yes(4) is not an Opt(String)");
    assert!(err.contains("op.decode"), "{err}");
}

/// A forwarding handler's tail is its function's own row, which a call unifies with
/// the caller's ambient: the caller's other effects (here one taking a function, and
/// IO) are not the body's, and are not charged to the handler (spec D5).
#[test]
fn a_forwarding_recorder_is_not_charged_with_its_callers_effects() {
    let src = format!(
        "{WORLD}{RECORD}effect Spawn {{ fn spawn(f: fn() -> Int) -> Int }}\n\
         pub fn main() -> Int {{\n\
           handle {{\n\
             handle {{\n\
               let a = spawn(fn() {{ 1 }})\n\
               io.println(\"x\")\n\
               a + record(fn() {{ roll(6) }})\n\
             }} with {{ Spawn.spawn(f) -> resume(f()) }}\n\
           }} with {{ World.roll(s) -> resume(4) }}\n\
         }}\n"
    );
    assert_eq!(errors(&src), "");
    // The same body through the discharging replayer is still checked.
    let bad = src.replace(
        "record(fn() { roll(6) })",
        "replay(fn() { let _ = spawn(fn() { 2 })  roll(6) })",
    );
    assert!(errors(&bad).contains("E0434"), "{}", errors(&bad));
}

// ---- the review of 2026-10-16 (each red first) ------------------------------

/// S1: an escaped `resume` in a forwarding clause runs the clause again, which
/// performs the handled row: the lambda carries it (it was typed pure, and the
/// performs reached no handler at run time).
#[test]
fn an_escaped_resume_carries_the_forwarded_row() {
    let rec = "effect World { fn roll(s: Int) -> Int }\n\
               fn rec(body) {\n\
                 let run = handle { body() } with any {\n\
                   op(o) -> { let v = op.perform(o)  fn(u: Int) { resume(v)(u) } }\n\
                   return(x) -> fn(u: Int) { x }\n\
                 }\n\
                 run\n\
               }\n";
    let s = schemes(rec);
    assert!(
        s.contains("rec : forall a b. fn(fn() / {b} -> a) / {b} -> fn(Int) / {b} -> a"),
        "{s}"
    );
    let outside = format!(
        "{rec}pub fn main() -> Int {{\n\
           let g = handle {{ rec(fn() {{ roll(1) + roll(2) }}) }} with {{ World.roll(s) -> resume(s) }}\n\
           g(0)\n\
         }}\n"
    );
    assert!(errors(&outside).contains("E0420"), "{}", errors(&outside));
    let pure = format!(
        "{rec}fn pure_call(g: fn(Int) / {{}} -> Int) / {{}} -> Int {{ g(0) }}\n\
         pub fn main() -> Int {{\n\
           handle {{ pure_call(rec(fn() {{ roll(1) }})) }} with {{ World.roll(s) -> resume(s) }}\n\
         }}\n"
    );
    assert!(errors(&pure).contains("E04"), "{}", errors(&pure));
}

/// S2: an effect's type parameter is not known at run time, so `op.decode` cannot
/// check an answer of that type: it refuses by name instead of letting a String
/// in as an Int.
#[test]
fn decoding_at_an_effect_parameter_is_refused_by_name() {
    let src = "effect St(s) { fn get() -> s }\n\
               fn replay(body) { handle { body() } with any { op(o) -> resume(op.decode(o, \"\\\"hello\\\"\")) } }\n\
               fn prog() / {St(Int)} -> Int { get() + 1 }\n\
               pub fn main() { io.println(int.to_string(replay(fn() { prog() }))) }\n";
    assert_eq!(errors(src), "");
    let err = run_source("t.elya", src).expect_err("refused");
    assert!(
        err.contains("op.decode") && err.contains("parameter"),
        "{err}"
    );
}

/// S3: a `with any` handler inside a local `let`-generalized lambda is still
/// checked at each use (the local scheme generalized its handled tail before the
/// group-end check could mark it).
#[test]
fn a_local_generic_handler_is_checked_at_each_use() {
    for (src, why) in [
        (
            "effect World { fn roll(s: Int) -> Int }\n\
             fn quiet() / {} -> Int {\n\
               let r = fn(b) { handle { b() } with any { op(o) -> resume(op.decode(o, \"4\")) } }\n\
               let x = r(fn() { roll(6) })\n\
               r(fn() { io.println(\"hidden\")  x })\n\
             }\npub fn main() -> Int { quiet() }\n",
            "IO through a local handler",
        ),
        (
            "effect World { fn roll(s: Int) -> Int }\n\
             fn replay(body) { handle { body() } with any { op(o) -> resume(op.decode(o, \"4\")) } }\n\
             fn quiet() / {} -> Int {\n\
               let r = fn(b) { replay(b) }\n\
               let x = r(fn() { roll(6) })\n\
               r(fn() { io.println(\"hidden\")  x })\n\
             }\npub fn main() -> Int { quiet() }\n",
            "IO through a local wrapper of a top-level replayer",
        ),
        (
            "effect Mk { fn make() -> fn(Int) -> Int }\n\
             fn f() -> Int {\n\
               let r = fn(b) { handle { b() } with any { op(o) -> resume(op.decode(o, \"4\")) } }\n\
               let z = r(fn() { 0 })\n\
               let g = r(fn() { make() })\n\
               g(1)\n\
             }\npub fn main() -> Int { f() }\n",
            "a function answer through a local handler",
        ),
    ] {
        assert!(errors(src).contains("E0434"), "{why}: {}", errors(src));
    }
}

/// B1: a non-regular (nested) type is data; the check terminates.
#[test]
fn a_nested_type_is_checked_without_looping() {
    let src = "type Opt(a) { No, Yes(a) }\n\
               type Nest(a) { Z(a), S(Nest(Opt(a))) }\n\
               effect E { fn e() -> Nest(Int) }\n\
               fn replay(body) { handle { body() } with any { op(o) -> resume(op.decode(o, \"S(Z(Yes(1)))\")) } }\n\
               pub fn main() { io.println(match replay(fn() { e() }) { Z(n) -> \"z\"  S(m) -> \"s\" }) }\n";
    assert_eq!(errors(src), "");
    assert_eq!(run_source("t.elya", src).expect("runs"), "s\n");
    // ... and one with a function inside, at depth, is still refused.
    let bad = "type Opt(a) { No, Yes(a) }\n\
               type Nest(a) { Z(a), S(Nest(Opt(a))) }\n\
               effect E { fn e() -> Nest(fn(Int) -> Int) }\n\
               fn replay(body) { handle { body() } with any { op(o) -> resume(op.decode(o, \"1\")) } }\n\
               pub fn main() { io.println(match replay(fn() { e() }) { Z(n) -> \"z\"  S(m) -> \"s\" }) }\n";
    assert!(errors(bad).contains("E0434"), "{}", errors(bad));
}

/// B2: encoding and decoding do not recurse on the host stack: a 100 000-cell
/// list round-trips (lists of that length run everywhere else).
#[test]
fn a_long_list_round_trips() {
    let src = "type L { Nil, Cons(Int, L) }\n\
               effect Q { fn big(n: Int) -> L }\n\
               fn build(n: Int, acc: L) -> L { if n == 0 { acc } else { build(n - 1, Cons(n, acc)) } }\n\
               fn len(l: L, a: Int) -> Int { match l { Nil -> a  Cons(x, r) -> len(r, a + 1) } }\n\
               fn rt(body) {\n\
                 handle { body() } with any {\n\
                   op(o) -> { let v = op.perform(o)  resume(op.decode(o, op.encode(o, v))) }\n\
                 }\n\
               }\n\
               pub fn main() {\n\
                 let r = handle { rt(fn() { len(big(100000), 0) }) } with { Q.big(n) -> resume(build(n, Nil)) }\n\
                 io.println(int.to_string(r))\n\
               }\n";
    assert_eq!(errors(src), "");
    assert_eq!(run_source("t.elya", src).expect("runs"), "100000\n");
}

/// B3 and N1: floats encode as decimals (no exponent) and the three non-finite
/// words round-trip; `Unit` is spelled as Elya spells it.
#[test]
fn floats_and_unit_round_trip() {
    let src = "effect Q { fn f(x: Float) -> Float  fn u() -> Unit }\n\
               fn rt(body) {\n\
                 handle { body() } with any {\n\
                   op(o) -> { let v = op.perform(o)  let e = op.encode(o, v)  io.println(e)  resume(op.decode(o, e)) }\n\
                 }\n\
               }\n\
               pub fn main() {\n\
                 let big = 10000000000000000000000000000000000000000.0\n\
                 let inf = big *. big *. big *. big *. big *. big *. big *. big\n\
                 let _ = handle { rt(fn() { let a = f(big)  let b = f(0.0 -. inf)  let c = f(1.0)  u() }) } with {\n\
                   Q.f(x) -> resume(x)\n\
                   Q.u() -> resume(Unit)\n\
                 }\n\
                 io.println(\"ok\")\n\
               }\n";
    assert_eq!(errors(src), "");
    assert_eq!(
        run_source("t.elya", src).expect("runs"),
        "10000000000000000000000000000000000000000.0\n-inf\n1.0\nUnit\nok\n"
    );
}

/// N3: two operation values compare by what they are.
#[test]
fn an_operation_equals_itself() {
    let src = "effect World { fn roll(s: Int) -> Int }\n\
               fn f(body) { handle { body() } with any { op(o) -> if o == o { resume(op.decode(o, \"2\")) } else { 0 } } }\n\
               pub fn main() { io.println(int.to_string(f(fn() { roll(6) + 1 }))) }\n";
    assert_eq!(run_source("t.elya", src).expect("runs"), "3\n");
}
