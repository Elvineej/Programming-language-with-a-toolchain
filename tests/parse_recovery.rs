//! Parser error recovery inside a body (2026-10-23, spec
//! `docs/superpowers/specs/2026-10-23-elya-parser-recovery-design.md`). One
//! mistake is one diagnostic: the rest of the body and the program are still
//! checked, and the function with the mistake is still a known name.
//!
//! Negative controls (each reverted; `src/parse.rs` compared byte for byte after):
//! - K1, D2 off (a failed statement fails its block again):
//!   `one_mistake_in_a_body_is_one_error` loses `zz` (p7 reports `yy` only), the
//!   lambda test loses `w1` and `w2`, and the D4 test loses `w1`. (D3's stubs
//!   keep the cascade E0200s away even then.)
//! - K2, D4 off (every `fn` starts a declaration): the header-mistake program
//!   gains "E0100 expected function name" at the lambda's `fn`.
//! - K3, D1 off (`eof_error` silent): p5 checks clean (`[]`).
//! - The review's F1/F2/F4 were red before their fixes (follow-on errors after
//!   `linear`, a repeated "expected `}`", a truncated `type` checking clean).

/// The diagnostics `check` reports, as `code message` lines, in order.
fn diags(src: &str) -> Vec<String> {
    let rendered = match elya::check_source("t.elya", src) {
        Ok(()) => return Vec::new(),
        Err(e) => e,
    };
    let mut plain = String::new();
    let mut esc = false;
    for c in rendered.chars() {
        match (esc, c) {
            (false, '\u{1b}') => esc = true,
            (true, 'm') => esc = false,
            (true, _) => {}
            (false, c) => plain.push(c),
        }
    }
    plain
        .lines()
        .filter_map(|l| {
            let l = l.trim_start();
            let rest = l.strip_prefix('[')?;
            let (code, msg) = rest.split_once("] Error: ")?;
            Some(format!("{code} {msg}"))
        })
        .collect()
}

#[test]
fn one_mistake_in_a_body_is_one_error() {
    let p1 = "fn add(a: Int, b: Int) -> Int {\n  let x = (a + b\n  let y = x * 2\n  y + 1\n}\n\
              fn twice(n: Int) -> Int { add(n, n) }\n\
              pub fn main() -> Int { twice(3) + undefined_name }\n";
    assert_eq!(
        diags(p1),
        [
            "E0100 expected `)`",
            "E0200 unresolved name `undefined_name`"
        ]
    );
    let p2 = "fn f(a: Int) -> Int {\n  let = a + 1\n  a * 2\n}\n\
              fn g(n: Int) -> Int { let h = fn(x) { x + f(n) }  h(1) }\n\
              pub fn main() -> Int { g(1) + oops }\n";
    assert_eq!(
        diags(p2),
        [
            "E0100 expected binding name",
            "E0200 unresolved name `oops`"
        ]
    );
    let p3 = "type Opt { None, Some(Int) }\n\
              fn get(o: Opt) -> Int {\n  match o { Some(+) -> 1  None -> 0 }\n}\n\
              pub fn main() -> Int { get(None) + nope }\n";
    assert_eq!(
        diags(p3),
        ["E0100 expected a pattern", "E0200 unresolved name `nope`"]
    );
    let p4 = "effect E { fn e() -> Int }\n\
              fn run() -> Int {\n  handle { e() } with { E.e( -> resume(1) }\n}\n\
              pub fn main() -> Int { run() + missing }\n";
    assert_eq!(
        diags(p4),
        [
            "E0100 expected parameter name",
            "E0200 unresolved name `missing`"
        ]
    );
    let p7 = "fn a() -> Int {\n  let p = 1 +\n  let q = 2\n  p + q + zz\n}\n\
              fn b() -> Int { a() * 2 + yy }\n\
              pub fn main() -> Int { b() }\n";
    assert_eq!(
        diags(p7),
        [
            "E0100 expected an expression",
            "E0200 unresolved name `zz`",
            "E0200 unresolved name `yy`"
        ]
    );
}

#[test]
fn a_truncated_declaration_is_an_error() {
    let p5 = "pub fn main() -> Int { g() }\npub fn g(\n";
    assert_eq!(diags(p5), ["E0100 unexpected end of file"]);
    let p6 = "pub fn main() -> Int { 1 }\nfn h() -> Int { x +\n";
    assert_eq!(diags(p6), ["E0100 unexpected end of file"]);
    // A block, an effect and a type missing their `}` at the end of the file.
    assert_eq!(diags("pub fn main() -> Int { 1\n"), ["E0100 expected `}`"]);
    assert_eq!(
        diags("pub fn main() -> Int { 1 }\neffect E { fn e() -> Int\n"),
        ["E0100 expected `}`"]
    );
    assert_eq!(
        diags("pub fn main() -> Int { 1 }\ntype T { A, B\n"),
        ["E0100 expected `}`"]
    );
}

#[test]
fn a_lambda_body_recovers_on_its_own() {
    let src = "fn f(n: Int) -> Int {\n  let h = fn(x) {\n    let y = x +* 1\n    y + w1\n  }\n  h(n) + w2\n}\n\
               fn g() -> Int { f(1) + w3 }\n\
               pub fn main() -> Int { g() }\n";
    assert_eq!(
        diags(src),
        [
            "E0100 expected an expression",
            "E0200 unresolved name `w1`",
            "E0200 unresolved name `w2`",
            "E0200 unresolved name `w3`"
        ]
    );
}

#[test]
fn a_lambda_fn_does_not_restart_the_module() {
    // The mistake is in a call's arguments, so the statement skips to the end of
    // its line; the lambda's `fn` inside it must not start a declaration.
    let src = "fn f(n: Int) -> Int {\n  let k = g(n, ) + apply(fn(x) { x }, 1)\n  k + w1\n}\n\
               fn apply(h, v) { h(v) }\n\
               fn g(a: Int, b: Int) -> Int { a }\n\
               pub fn main() -> Int { f(1) }\n";
    assert_eq!(
        diags(src),
        ["E0100 expected an expression", "E0200 unresolved name `w1`"]
    );
    // A header mistake skips the declaration at module level; a lambda's `fn`
    // starting a line inside its body is not a declaration (a name follows a
    // declaration's `fn`), so the parse resumes at `apply`.
    let src =
        "fn f(a: Int,, b: Int) -> Int {\n  let k = apply(\n    fn(x) { x }, 1)\n  k + w1\n}\n\
               fn apply(h, v) { h(v) + w2 }\n\
               pub fn main() -> Int { f(1, 2) }\n";
    assert_eq!(
        diags(src),
        [
            "E0100 expected parameter name",
            "E0200 unresolved name `w2`"
        ]
    );
}

/// The review's findings 1, 2 and 4: a `linear type` is a declaration; one
/// missing `}` at the end of the file is one error however many blocks are open;
/// a truncated `type` fails `check`.
#[test]
fn the_end_of_the_file_is_reported_once() {
    assert_eq!(
        diags(
            "pub fn main() -> Int {\n  1\n\nlinear type H { H(Int) }\npub fn g() -> H { H(1) }\n"
        ),
        ["E0100 expected `}`"]
    );
    for src in [
        "pub fn main() -> Int { { { 1\n",
        "pub fn main() -> Int { match 1 {\n",
        "pub fn main() -> Int { if True { 1 } else { 2\n",
    ] {
        assert_eq!(diags(src), ["E0100 expected `}`"], "{src}");
    }
    for src in [
        "pub fn main() -> Int { 1 }\ntype T {\n",
        "pub fn main() -> Int { 1 }\ntype\n",
        "pub fn main() -> Int { 1 }\nlinear type H {\n",
    ] {
        assert_eq!(diags(src), ["E0100 unexpected end of file"], "{src}");
    }
}
