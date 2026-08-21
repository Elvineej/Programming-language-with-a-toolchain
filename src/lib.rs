//! The Elya compiler (Slice 1: tree-walking interpreter).

pub mod affine;
pub mod ast;
pub mod diag;
pub mod eval;
pub mod exhaust;
pub mod lex;
pub mod parse;
pub mod resolve;
pub mod span;
pub mod types;

use crate::diag::{render, Diagnostic, Severity};
use crate::span::SourceMap;

/// Shared, explicitly-threaded compiler state. No globals live outside this.
#[derive(Debug, Default)]
pub struct Session {}

impl Session {
    pub fn new() -> Self {
        Session {}
    }
}

/// The front-end passes (parse → resolve → infer → exhaustiveness), returning the
/// module and all diagnostics. Exhaustiveness runs only on an error-free program;
/// warnings never block a later pass.
fn front_end(session: &Session, text: &str) -> (ast::Module, Vec<Diagnostic>) {
    let (module, mut diags) = parse::parse_module(session, text);
    diags.extend(resolve::check(session, &module));
    if diags.is_empty() {
        let (idiags, affine_sites) = types::infer_with_sites(session, &module);
        diags.extend(idiags);
        if !diags.iter().any(|d| d.severity == Severity::Error) {
            diags.extend(exhaust::check(&module));
            diags.extend(affine::check(&module, &affine_sites));
        }
    }
    (module, diags)
}

/// Full pipeline: returns program output, or rendered diagnostics on failure.
pub fn run_source(name: &str, text: &str) -> Result<String, String> {
    let session = Session::new();
    let sm = SourceMap::new(name, text);
    let (module, diags) = front_end(&session, text);
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
    let (_module, diags) = front_end(&session, text);
    match fail_if_errors(&diags, &sm) {
        Some(rendered) => Err(rendered),
        None => Ok(()),
    }
}

/// Rendered non-fatal warnings for a source, or `None` if there are none. The
/// CLI surfaces these on a *successful* compile (the spec §11 obligation) so an
/// `E0426`/`E0431` lint reaches the user even though it does not fail the build.
pub fn warnings(name: &str, text: &str) -> Option<String> {
    let session = Session::new();
    let sm = SourceMap::new(name, text);
    let (_module, diags) = front_end(&session, text);
    let warns: Vec<Diagnostic> = diags
        .into_iter()
        .filter(|d| d.severity == Severity::Warning)
        .collect();
    if warns.is_empty() {
        None
    } else {
        Some(render(&warns, &sm))
    }
}

/// Compilation fails only on `Severity::Error`; warnings (lints, e.g. `E0426`)
/// are non-fatal. When there is an error, all diagnostics — warnings included —
/// are rendered together. (Surfacing warnings on a *successful* compile is a
/// tracked obligation for the CLI output path; spec §11.)
fn fail_if_errors(diags: &[Diagnostic], sm: &SourceMap) -> Option<String> {
    let has_error = diags.iter().any(|d| d.severity == Severity::Error);
    if has_error {
        Some(render(diags, sm))
    } else {
        None
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

    #[test]
    fn run_source_rejects_ill_typed_at_compile_time() {
        let err = run_source(
            "t.elya",
            "pub fn main() { let _ = 1 + \"a\"\n io.println(\"x\") }\n",
        )
        .unwrap_err();
        assert!(err.contains("E0400"), "{err}");
    }
}
