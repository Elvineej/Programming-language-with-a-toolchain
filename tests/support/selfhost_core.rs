//! Core in Elya (self-hosting part 5's first piece, spec
//! docs/superpowers/specs/2026-11-27-elya-backend-in-elya-design.md D3): what
//! `elya core --dump` prints, and the focused programs.

/// What `elya core --dump` prints for a program without imports that checks,
/// or `None` for one Core in Elya does not read yet (an import is part 4c's)
/// or that does not check (there is no Core to print).
#[allow(dead_code)]
pub fn want(text: &str) -> Option<String> {
    let session = elya::Session::new();
    if !elya::parse::parse_module(&session, text)
        .0
        .imports
        .is_empty()
    {
        return None;
    }
    let (module, diags, _) = elya::front_end_with("main.elya", text, &mut |_| None);
    if diags
        .iter()
        .any(|d| d.severity == elya::diag::Severity::Error)
    {
        return None;
    }
    let (_, table) = elya::types::infer_typed_table(&session, &module);
    Some(match elya::core::lower_module(&module, &table) {
        Ok(core) => format!(
            "{}\n",
            elya::core::pretty_typed(&core, &mut elya::types::TyPrinter::new())
        ),
        Err(e) => format!("refused {e:?}\n"),
    })
}

/// What Core in Elya prints for a program with a `Float` literal, whose
/// value its parser does not keep: such programs are not compared.
#[allow(dead_code)]
pub const FLOAT: &str = "refused Unsupported(\"Float in Elya\")\n";

#[allow(dead_code)]
pub const FOCUSED: &[&str] = &[
    // Strings: every escape, a raw tab and newline, a non-ASCII byte, in literals and patterns.
    "fn f(s) { match s { \"a\\\"b\\\\c\\n\\t\\r\" -> 1  \"\t\n\" -> 2  \"é\" -> 3  _ -> 4 } }\npub fn main() / {IO} { io.println(\"q\\\"\\\\\\n\" <> \"é\t\") }\n",
    // Unary and short-circuit operators.
    "fn f(a, b) { if !a && b || a { -1 } else { -(2 + 3) } }\npub fn main() -> Int { 0 }\n",
    // Every binary operator over Int and String.
    "fn f(a, b) { let c = a + b - a * b / 2 % 3\n let d = a == b || a != b || a < b || a <= b || a > b || a >= b\n if d { c } else { 0 } }\nfn g(s, t) { s <> t }\npub fn main() -> Int { 0 }\n",
    // A generic function, its variables shared across the module by one printer.
    "fn id(x) { x }\nfn pair(x, y) { fn(f) { f(x, y) } }\nfn k(x, y) { x }\npub fn main() -> Int { pair(1, \"a\")(k) }\n",
    // Ground types with function fields and rows.
    "effect Log { fn log(s: String) -> Unit }\ntype Cb { Cb(fn(Int) / {Log} -> Int, Int), Done }\nfn f(c) { match c { Cb(g, n) -> g(n)  Done -> 0 } }\npub fn main() -> Int { 0 }\n",
    // A perform, a local shadowing an operation, a lambda parameter shadowing it.
    "effect Ask { fn ask(n: Int) -> Int }\nfn f() / {Ask} -> Int { ask(1) }\nfn g(ask) { ask(2) }\nfn h() / {Ask} -> Int { let k = fn(ask) { ask(3) }\n k(fn(n) { n }) + ask(4) }\npub fn main() -> Int { 0 }\n",
    // A handler with a qualified and an unqualified clause, a return clause, an applied resume.
    "effect E { fn e() -> Int\n fn d(n: Int) -> Int }\nfn f() -> Int { handle { e() + d(1) } with { E.e() -> resume(1)  d(n) -> resume(n)  return(r) -> r * 2 } }\npub fn main() -> Int { f() }\n",
    // A multi handler, a plain one over a multi effect, `with any`.
    "effect multi Flip { fn flip() -> Bool }\nfn f() -> Int { handle { if flip() { 1 } else { 2 } } with multi { flip() -> resume(True) + resume(False) } }\nfn g() -> Int { handle { if flip() { 1 } else { 2 } } with { flip() -> resume(True) } }\npub fn main() -> Int { f() + g() }\n",
    "effect Log { fn log(s: String) -> Unit }\nfn f() -> Unit { handle { log(\"x\") } with any { op(o) -> resume(op.perform(o))  return(x) -> x } }\npub fn main() -> Int { 0 }\n",
    // Nested patterns, literals, a match in a let, statements discarded.
    "type L { N, C(Int, L) }\nfn f(xs) { let y = match xs { C(1, C(_, N)) -> 1  C(n, rest) -> n  N -> 0 }\n y + 1\n y }\npub fn main() -> Int { f(C(1, N)) }\n",
    // A nested block, an if in a block, unit literals and Bool patterns.
    "fn f(b) { { let x = 1\n { x } } + match b { True -> 1  False -> 0 } }\nfn g(u) { match u { Unit -> Unit } }\npub fn main() -> Int { f(True) }\n",
    // A resume applied to arguments: `let $k`.
    "effect Get { fn get() -> fn(Int) -> Int }\nfn f() -> Int { handle { get()(2) } with { get() -> resume(fn(x) { x + 1 }) } }\npub fn main() -> Int { f() }\n",
    // Builtins of every family.
    "pub fn main() / {IO} { let b = buffer.push(buffer.new(), \"x\")\n let a = array.push(array.new(), 1)\n io.println(buffer.finish(b) <> int.to_string(array.length(a) + int.wrapping_add(1, 2) + string.byte_at(\"ab\", 0))) }\n",
    // A match binder and a return binder shadowing an operation.
    "effect Ask { fn ask(n: Int) -> Int }\nfn m(x: fn(Int) -> Int) -> Int { match x { ask -> ask(5) } }\nfn r(x: fn(Int) -> Int) -> Int { handle { x } with { ask(n) -> resume(n)  return(ask) -> ask(6) } }\npub fn main() -> Int { 0 }\n",
    // A clause parameter shadowing an operation.
    "effect A { fn a(g: fn(Int) -> Int) -> Int }\neffect B { fn b(n: Int) -> Int }\nfn f() -> Int { handle { a(fn(x) { x }) } with { a(b) -> resume(b(1)) } }\npub fn main() -> Int { 0 }\n",
];
