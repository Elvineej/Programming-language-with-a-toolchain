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
