//! The Elya compiler (Slice 1: tree-walking interpreter).

pub mod ast;
pub mod diag;
pub mod eval;
pub mod lex;
pub mod parse;
pub mod resolve;
pub mod span;
pub mod types;

use crate::diag::{render, Diagnostic};
use crate::span::SourceMap;

/// Shared, explicitly-threaded compiler state. No globals live outside this.
#[derive(Debug, Default)]
pub struct Session {}

impl Session {
    pub fn new() -> Self {
        Session {}
    }
}

/// Full pipeline: returns program output, or rendered diagnostics on failure.
pub fn run_source(name: &str, text: &str) -> Result<String, String> {
    let session = Session::new();
    let sm = SourceMap::new(name, text);
    let (module, mut diags) = parse::parse_module(&session, text);
    diags.extend(resolve::check(&session, &module));
    if let Some(rendered) = fail_if_errors(&diags, &sm) {
        return Err(rendered);
    }
    match eval::run_module(&module) {
        Ok(interp) => Ok(interp.output().to_string()),
        Err(e) => Err(render(&[e.diag], &sm)),
    }
}

/// Front-end only (no execution).
pub fn check_source(name: &str, text: &str) -> Result<(), String> {
    let session = Session::new();
    let sm = SourceMap::new(name, text);
    let (module, mut diags) = parse::parse_module(&session, text);
    diags.extend(resolve::check(&session, &module));
    if diags.is_empty() {
        diags.extend(types::infer(&session, &module));
    }
    match fail_if_errors(&diags, &sm) {
        Some(rendered) => Err(rendered),
        None => Ok(()),
    }
}

fn fail_if_errors(diags: &[Diagnostic], sm: &SourceMap) -> Option<String> {
    if diags.is_empty() {
        None
    } else {
        Some(render(diags, sm))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_source_executes_hello_world() {
        let out = run_source(
            "h.elya",
            "pub fn main() / {IO} {\n  io.println(\"Hello, Elya!\")\n}\n",
        )
        .unwrap();
        assert_eq!(out, "Hello, Elya!\n");
    }

    #[test]
    fn run_source_reports_diagnostics() {
        let err = run_source("b.elya", "fn main() { x }\n").unwrap_err();
        assert!(err.contains("E0200"), "err: {err}");
    }

    #[test]
    fn check_source_is_ok_for_valid_program() {
        assert!(check_source("h.elya", "fn main() { io.println(\"x\") }\n").is_ok());
    }
}
