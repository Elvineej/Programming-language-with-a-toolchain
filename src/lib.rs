//! The Elya compiler (Slice 1: tree-walking interpreter).

pub mod affine;
pub mod ast;
pub mod core;
pub mod diag;
pub mod eval;
pub mod exhaust;
pub mod lex;
pub mod literal;
pub mod modules;
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

/// Reads no file: the text entry points see one file, and a user import there is
/// E0600 (spec 2026-10-23-elya-modules-part-1 D7).
fn no_files(_: &str) -> Option<(String, String)> {
    None
}

/// The front-end passes over a whole program (load and merge the modules, then
/// resolve → infer → exhaustiveness), returning the merged module, all
/// diagnostics and the program's sources. The root and the modules it imports are
/// read by `read` (spec 2026-10-23-elya-modules-part-1 D7). Exhaustiveness runs
/// only on an error-free program; warnings never block a later pass.
pub fn front_end_with(
    name: &str,
    text: &str,
    read: &mut modules::Reader,
) -> (ast::Module, Vec<Diagnostic>, SourceMap) {
    let session = Session::new();
    let (program, mut diags) = modules::load(&session, name, text, read);
    let module = program.module;
    // A name a module diagnostic already refused is not reported again (the
    // modules review's R3: E0201 "unknown builtin" after E0600 or E0603).
    diags.extend(resolve::check(&session, &module).into_iter().filter(|d| {
        !d.labels
            .first()
            .is_some_and(|l| program.refused.contains(&l.span))
    }));
    if diags.is_empty() {
        let (idiags, affine_sites, fns) = types::infer_with_interfaces(&session, &module);
        diags.extend(idiags);
        if !diags.iter().any(|d| d.severity == Severity::Error) {
            diags.extend(exhaust::check(&module));
            diags.extend(affine::check(&module, &affine_sites));
            // Modules part 2: what each import grants (spec 2026-10-24).
            diags.extend(modules::check_grants(&program.grants, &module, &fns));
        }
    }
    (module, diags, program.sources)
}

/// Full pipeline: returns program output, or rendered diagnostics on failure.
pub fn run_source(name: &str, text: &str) -> Result<String, String> {
    let (module, diags, sm) = front_end_with(name, text, &mut no_files);
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
    run_source_outcome_with(name, text, &mut no_files)
}

/// [`run_source_outcome`] over a whole program (modules part 1 D7).
pub fn run_source_outcome_with(name: &str, text: &str, read: &mut modules::Reader) -> RunOutcome {
    let (module, diags, sm) = front_end_with(name, text, read);
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
    record_source_with(name, text, &mut no_files)
}

/// [`record_source`] over a whole program (modules part 1 D7).
pub fn record_source_with(
    name: &str,
    text: &str,
    read: &mut modules::Reader,
) -> (Option<String>, RunOutcome) {
    let (module, diags, sm) = front_end_with(name, text, read);
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

/// A trace's text as the host reads it: its entries, or the line (from 1) that
/// cannot be read and why (spec 2026-10-24-elya-trace-text D2). A leading
/// byte-order mark is skipped; the first line, trimmed, must be the header;
/// a blank line is skipped; every other line is split by [`split_entry`] and
/// both halves are trimmed. `elya/replay`'s `parse` reads the same way.
pub fn read_trace(trace: &str) -> Result<Vec<(String, String)>, (usize, &'static str)> {
    let trace = trace.strip_prefix('\u{feff}').unwrap_or(trace);
    let mut lines = trace.lines();
    if lines.next().map(str::trim) != Some(TRACE_HEADER) {
        return Err((1, "the first line must be `elya trace 1`"));
    }
    let mut entries = Vec::new();
    for (i, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let (q, a) = split_entry(line).map_err(|why| (i + 2, why))?;
        entries.push((q.trim().to_string(), a.trim().to_string()));
    }
    Ok(entries)
}

/// One trace entry's question and answer: the line split at the first ` = `
/// outside a string literal, a literal running from a `"` to the next
/// unescaped `"` (spec 2026-10-24-elya-trace-text D2). Neither half is trimmed.
/// `elya/replay`'s `parse` and the native runtime split the same way.
pub fn split_entry(line: &str) -> Result<(&str, &str), &'static str> {
    let b = line.as_bytes();
    let mut in_str = false;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' if in_str => i += 1,
            b'"' => in_str = !in_str,
            b' ' if !in_str && b[i..].starts_with(b" = ") => {
                return Ok((&line[..i], &line[i + 3..]));
            }
            _ => {}
        }
        i += 1;
    }
    Err(if in_str {
        "a string is not closed"
    } else {
        "expected `question = answer`"
    })
}

/// Runs with the host answered from `trace` (the text [`record_source`]
/// writes); a divergence or a leftover entry is a runtime error. Lines are
/// read with surrounding whitespace (and a leading byte-order mark) ignored. A
/// trace that cannot be read runs nothing.
pub fn replay_source(name: &str, text: &str, trace: &str) -> RunOutcome {
    replay_source_with(name, text, trace, &mut no_files)
}

/// [`replay_source`] over a whole program (modules part 1 D7).
pub fn replay_source_with(
    name: &str,
    text: &str,
    trace: &str,
    read: &mut modules::Reader,
) -> RunOutcome {
    let entries = match read_trace(trace) {
        Ok(entries) => entries,
        Err((1, _)) => {
            return RunOutcome::refused(format!(
                "error: not a trace: the first line must be `{TRACE_HEADER}`\n"
            ));
        }
        Err((n, why)) => return RunOutcome::refused(format!("error: trace line {n}: {why}\n")),
    };
    let (module, diags, sm) = front_end_with(name, text, read);
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
    check_source_with(name, text, &mut no_files)
}

/// [`check_source`] over a whole program (modules part 1 D7).
pub fn check_source_with(name: &str, text: &str, read: &mut modules::Reader) -> Result<(), String> {
    let (_module, diags, sm) = front_end_with(name, text, read);
    match fail_if_errors(&diags, &sm) {
        Some(rendered) => Err(rendered),
        None => Ok(()),
    }
}

/// Rendered non-fatal warnings for a source, or `None` if there are none. The
/// CLI surfaces these on a *successful* compile (the spec §11 obligation) so an
/// `E0426`/`E0431` lint reaches the user even though it does not fail the build.
pub fn warnings(name: &str, text: &str) -> Option<String> {
    warnings_with(name, text, &mut no_files)
}

/// [`warnings`] over a whole program (modules part 1 D7).
pub fn warnings_with(name: &str, text: &str, read: &mut modules::Reader) -> Option<String> {
    let (_module, diags, sm) = front_end_with(name, text, read);
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
