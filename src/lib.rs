//! The Elya compiler (Slice 1: tree-walking interpreter).

pub mod affine;
pub mod ast;
pub mod core;
pub mod diag;
pub mod eval;
pub mod exhaust;
pub mod lex;
pub mod literal;
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

/// What `elya run` shows (spec 2026-10-16-elya-differential-sweep): the
/// program's output, kept when the run fails (D1), and main's value as a native
/// binary prints it (D2: an `Int` in decimal, `Unit` as `0`; `None` for any
/// other type). `error` is the rendered diagnostic of a front-end or runtime
/// failure.
pub struct RunOutcome {
    pub output: String,
    pub value: Option<Result<i64, String>>,
    pub error: Option<String>,
}

pub fn run_source_outcome(name: &str, text: &str) -> RunOutcome {
    let session = Session::new();
    let sm = SourceMap::new(name, text);
    let (module, diags) = front_end(&session, text);
    if let Some(rendered) = fail_if_errors(&diags, &sm) {
        return RunOutcome::refused(rendered);
    }
    let (interp, result) = eval::run_module_keeping(&module, eval::World::Live);
    RunOutcome::of(interp.output().to_string(), result, &sm)
}

impl RunOutcome {
    /// Nothing ran: the front end, or the trace, refused.
    fn refused(error: String) -> RunOutcome {
        RunOutcome {
            output: String::new(),
            value: None,
            error: Some(error),
        }
    }

    /// A run's outcome: its output, kept whether it succeeded or failed.
    fn of(
        output: String,
        result: Result<eval::Value, eval::RuntimeError>,
        sm: &SourceMap,
    ) -> RunOutcome {
        match result {
            Ok(v) => RunOutcome {
                output,
                value: Some(match v {
                    eval::Value::Int(n) => Ok(n),
                    eval::Value::Unit => Ok(0),
                    _ => Err(
                        "main's value is not an `Int` or `Unit`, so it has no native value line"
                            .to_string(),
                    ),
                }),
                error: None,
            },
            Err(e) => RunOutcome {
                output,
                value: None,
                error: Some(render(&[e.diag], sm)),
            },
        }
    }
}

/// The trace format's first line (spec 2026-10-16-elya-cli-record-replay).
const TRACE_HEADER: &str = "elya trace 1";

/// Runs live and writes down every answer the host gave. The trace is
/// returned whether the run succeeds or fails (a failing run is the one most
/// worth replaying); a program that does not pass the front end has no trace
/// (`None`). The outcome keeps the program's output apart from its error (spec
/// 2026-10-23-elya-traced-run-output D1).
pub fn record_source(name: &str, text: &str) -> (Option<String>, RunOutcome) {
    let session = Session::new();
    let sm = SourceMap::new(name, text);
    let (module, diags) = front_end(&session, text);
    if let Some(rendered) = fail_if_errors(&diags, &sm) {
        return (None, RunOutcome::refused(rendered));
    }
    let (interp, result) = eval::run_module_keeping(&module, eval::World::Record(Vec::new()));
    let mut trace = format!("{TRACE_HEADER}\n");
    if let eval::World::Record(log) = &interp.world {
        for (q, a) in log {
            trace.push_str(&format!("{q} = {a}\n"));
        }
    }
    let output = interp.output().to_string();
    (Some(trace), RunOutcome::of(output, result, &sm))
}

/// [`record_source`] as `(trace, Ok(output))` or `(trace, Err(output and
/// diagnostics))`, the shape the trace tests compare against.
pub fn run_source_recording(name: &str, text: &str) -> (Option<String>, Result<String, String>) {
    let (trace, o) = record_source(name, text);
    let out = match o.error {
        None => Ok(o.output),
        Some(e) => Err(format!("{}{e}", o.output)),
    };
    (trace, out)
}

/// Runs with the host answered from `trace` (the text [`record_source`]
/// writes); a divergence or a leftover entry is a runtime error. Lines are
/// read with surrounding whitespace (and a leading byte-order mark) ignored. A
/// trace that cannot be read runs nothing.
pub fn replay_source(name: &str, text: &str, trace: &str) -> RunOutcome {
    let trace = trace.strip_prefix('\u{feff}').unwrap_or(trace);
    let mut lines = trace.lines();
    if lines.next().map(str::trim) != Some(TRACE_HEADER) {
        return RunOutcome::refused(format!(
            "error: not a trace: the first line must be `{TRACE_HEADER}`\n"
        ));
    }
    let mut entries = Vec::new();
    for (i, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        // Today's answers are Int and Unit; the LAST ` = ` splits, so a
        // question's arguments may contain one. A String answer would need
        // a quoted-aware split (spec §2).
        let Some((q, a)) = line.rsplit_once(" = ") else {
            return RunOutcome::refused(format!(
                "error: trace line {}: expected `question = answer`\n",
                i + 2
            ));
        };
        entries.push((q.trim().to_string(), a.trim().to_string()));
    }
    let session = Session::new();
    let sm = SourceMap::new(name, text);
    let (module, diags) = front_end(&session, text);
    if let Some(rendered) = fail_if_errors(&diags, &sm) {
        return RunOutcome::refused(rendered);
    }
    let (interp, result) =
        eval::run_module_keeping(&module, eval::World::Replay { entries, next: 0 });
    RunOutcome::of(interp.output().to_string(), result, &sm)
}

/// [`replay_source`] as `Ok(output)` or `Err(diagnostics)` (the output of a
/// failing replay is not in the error), the shape the trace tests compare against.
pub fn run_source_replaying(name: &str, text: &str, trace: &str) -> Result<String, String> {
    let o = replay_source(name, text, trace);
    match o.error {
        None => Ok(o.output),
        Some(e) => Err(e),
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
