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
