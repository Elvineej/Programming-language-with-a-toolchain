//! The checker's verdict as text (self-hosting, the checker's half of part 4):
//! what a checker written in Elya will print and be compared against, byte for
//! byte, as `ast_dump` is for the parser.
//!
//! Three sections, in order:
//! - `scheme <name> <scheme>`: every top-level function's generalized type, by
//!   name (sorted: the order inference visits its groups in is not part of the
//!   verdict);
//! - `type <start> <end> <type>`: every node's type, by span, its variables
//!   named afresh per line (`a`, `b`, ... in order of appearance), so two
//!   implementations that number variables differently still agree;
//! - the diagnostics, `elya::ast_dump::dump_diagnostics`' lines.

use crate::ast::Module;
use crate::types::TyPrinter;
use std::fmt::Write;

/// The checker's verdict on `module` (resolved and merged, as the front end
/// leaves it).
pub fn dump(module: &Module) -> String {
    let (mut schemes, diags, typed) = crate::types::infer_for_dump(module);
    schemes.sort();
    let mut out = String::new();
    for (name, scheme) in &schemes {
        writeln!(out, "scheme {name} {scheme}").expect("a String takes writes");
    }
    for (span, ty) in &typed {
        let t = TyPrinter::new().render(ty);
        writeln!(out, "type {} {} {t}", span.start, span.end).expect("a String takes writes");
    }
    out.push_str(&crate::ast_dump::dump_diagnostics(&diags));
    out
}

/// `affine::Facts` as text (spec 2026-11-15-elya-checker-in-elya D2): one
/// fact a line, each table sorted by its key's span, every span `start end`.
/// What the affine pass in Elya prints.
pub fn affine_facts(f: &crate::affine::Facts) -> String {
    use crate::affine::Why;
    use crate::span::Span;
    fn sp(s: Span) -> String {
        format!("{} {}", s.start, s.end)
    }
    fn sorted<V>(m: &std::collections::HashMap<Span, V>) -> Vec<(&Span, &V)> {
        let mut v: Vec<_> = m.iter().collect();
        v.sort_by_key(|(k, _)| **k);
        v
    }
    let mut out = String::new();
    for (b, m) in sorted(&f.many) {
        let why = match m.why {
            Why::Twice => "twice".to_string(),
            Why::Closure(s) => format!("closure {}", sp(s)),
            Why::Clause(s) => format!("clause {}", sp(s)),
        };
        let _ = writeln!(out, "many {} {} {why}", sp(*b), sp(m.at));
    }
    for (p, entries) in sorted(&f.held) {
        for (b, u) in entries {
            let _ = writeln!(out, "held {} {} {}", sp(*p), sp(*b), sp(*u));
        }
    }
    for (h, rs) in sorted(&f.resumes) {
        for r in rs {
            let _ = writeln!(out, "resumes {} {}", sp(*h), sp(*r));
        }
    }
    for (b, (read, by)) in sorted(&f.late) {
        let _ = writeln!(out, "late {} {} {}", sp(*b), sp(*read), sp(*by));
    }
    for (b, u) in sorted(&f.given) {
        let _ = writeln!(out, "given {} {}", sp(*b), sp(*u));
    }
    for (u, (name, modes)) in sorted(&f.borrow_values) {
        let bits: String = modes.iter().map(|&m| if m { '1' } else { '0' }).collect();
        let _ = writeln!(out, "borrow-value {} {name} {bits}", sp(*u));
    }
    out
}
