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
