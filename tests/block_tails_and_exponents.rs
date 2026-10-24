//! Block tails and float exponents (2026-10-24, spec
//! `docs/superpowers/specs/2026-10-24-elya-block-tails-and-exponents-design.md`):
//! a block without a value is E0103 (D1); float literals take an exponent
//! (D2), `1e19` is E0003 and an out-of-range one E0002.
//!
//! Negative controls (each reverted; the files compared byte for byte after):
//! - K1, D1 off: `a_block_ending_in_let_is_e0103` and `an_empty_block_is_e0103`
//!   fail (the main ending in `let` checks clean).
//! - K2, D2's range check off: `exponent_literal_mistakes_are_named` fails
//!   (`1.0e400` checks clean).

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
            let rest = l.trim_start().strip_prefix('[')?;
            let (code, msg) = rest.split_once("] Error: ")?;
            Some(format!("{code} {msg}"))
        })
        .collect()
}

/// What the program printed; panics on an error.
fn output(src: &str) -> String {
    let o = elya::run_source_outcome_with("t.elya", src, &mut |_: &str| None);
    assert!(o.error.is_none(), "{:?}", o.error);
    o.output
}

const ENDS_IN_LET: &str = "E0103 a block cannot end with `let`";

#[test]
fn a_block_ending_in_let_is_e0103() {
    for (kind, src) in [
        ("main", "pub fn main() { io.println(\"x\")\n  let y = 2\n}\n"),
        ("a function at Int", "fn f() -> Int { let y = 2 }\npub fn main() -> Int { f() }\n"),
        ("an if arm", "pub fn main() -> Int { if 1 == 1 { let z = 3 } else { 0 } }\n"),
        ("a lambda", "pub fn main() -> Int { let g = fn() { let q = 1 }\n  g()\n  7 }\n"),
        (
            "a clause",
            "effect E { fn e() -> Int }\npub fn main() -> Int { handle { e() } with { E.e() -> { let w = resume(1) } } }\n",
        ),
        ("a block expression", "pub fn main() -> Int { let a = { let b = 1 }\n  2 }\n"),
    ] {
        assert_eq!(diags(src), [ENDS_IN_LET], "{kind}");
    }
    // Ending in the bound name is the fix.
    assert_eq!(
        diags("pub fn main() -> Int { let y = 2\n  y }\n"),
        Vec::<String>::new()
    );
}

#[test]
fn an_empty_block_is_e0103() {
    assert_eq!(
        diags("fn f() { }\npub fn main() -> Int { 1 }\n"),
        ["E0103 a block has no value"]
    );
    assert_eq!(
        diags("pub fn main() { if 1 == 1 { } else { Unit } }\n"),
        ["E0103 a block has no value"]
    );
}

#[test]
fn exponent_literals_read_as_their_floats() {
    let src = "pub fn main() {
  io.println(float.to_string(1.0e19))
  io.println(float.to_string(2.5E-3))
  io.println(float.to_string(-1.5e+2))
  io.println(float.to_string(1_0.0e1_0))
  io.println(float.to_string(0.0e-999))
  let tiny = 5.0e-324
  io.println(if tiny >. 0.0 { \"positive\" } else { \"zero\" })
  io.println(float.to_string(tiny /. 2.0))
  io.println(float.to_string(1.7976931348623157e308))
}
";
    let out = output(src);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[..6],
        [
            "10000000000000000000.0",
            "0.0025",
            "-150.0",
            "100000000000.0",
            "0.0",
            "positive"
        ]
    );
    assert_eq!(lines[6], "0.0"); // half the smallest subnormal rounds to zero
    assert!(
        lines[7].starts_with("179769313486231570000"),
        "{}",
        lines[7]
    );
    assert_eq!(lines.len(), 8);
}

#[test]
fn exponent_literal_mistakes_are_named() {
    assert_eq!(
        diags("pub fn main() -> Int { let a = 1e19\n  1 }\n"),
        ["E0003 an exponent needs a float literal"]
    );
    let e =
        elya::check_source("t.elya", "pub fn main() -> Int { let a = 2E-5\n  1 }\n").unwrap_err();
    assert!(e.contains("write `2.0E-5`"), "{e}");
    // The review's F2: a `_` before the `e` is not carried into the help.
    let e =
        elya::check_source("t.elya", "pub fn main() -> Int { let a = 1_e5\n  1 }\n").unwrap_err();
    assert!(e.contains("write `1.0e5`"), "{e}");
    // The review's F1: an integer with an exponent out of range is both.
    assert_eq!(
        diags("pub fn main() -> Int { let a = 1e400\n  1 }\n"),
        [
            "E0003 an exponent needs a float literal",
            "E0002 float literal out of range"
        ]
    );
    for big in ["1.0e400", "1.0e-400", "9.9e308"] {
        assert_eq!(
            diags(&format!("pub fn main() -> Int {{ let a = {big}\n  1 }}\n")),
            ["E0002 float literal out of range"],
            "{big}"
        );
    }
}
