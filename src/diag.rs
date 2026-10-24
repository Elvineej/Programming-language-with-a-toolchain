//! Structured diagnostics and human-readable rendering.

use crate::span::{SourceMap, Span};
use ariadne::{sources, Color, Config, IndexType, Label as AriadneLabel, Report, ReportKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub labels: Vec<Label>,
    pub helps: Vec<String>,
}

impl Diagnostic {
    pub fn error(code: &str, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            code: code.to_string(),
            message: message.into(),
            labels: Vec::new(),
            helps: Vec::new(),
        }
    }

    /// A non-fatal diagnostic (a lint). Compilation succeeds despite warnings;
    /// only `Severity::Error` blocks it (see `lib::fail_if_errors`).
    pub fn warning(code: &str, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            code: code.to_string(),
            message: message.into(),
            labels: Vec::new(),
            helps: Vec::new(),
        }
    }

    pub fn with_label(mut self, span: Span, message: impl Into<String>) -> Diagnostic {
        self.labels.push(Label {
            span,
            message: message.into(),
        });
        self
    }

    pub fn with_help(mut self, message: impl Into<String>) -> Diagnostic {
        self.helps.push(message.into());
        self
    }
}

/// Render diagnostics to a string. Tests assert on structured fields, not on
/// this output; this is for humans.
pub fn render(diags: &[Diagnostic], src: &SourceMap) -> String {
    let mut buf = Vec::new();
    // Each label in its own file, at its offset within that file (modules part 1
    // D5); a single-file program has one file at base 0.
    let at = |offset: u32| {
        let (name, _, base) = src.file_of(offset);
        (name.to_string(), (offset - base) as usize)
    };
    // Built once: every file's text, by name.
    let mut cache = sources(
        src.files()
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .collect::<Vec<_>>(),
    );
    for d in diags {
        let kind = match d.severity {
            Severity::Error => ReportKind::Error,
            Severity::Warning => ReportKind::Warning,
        };
        let (id, offset) = match d.labels.first() {
            Some(l) => at(l.span.start),
            None => (src.name().to_string(), 0),
        };
        // Every span is a byte offset (spec 2026-10-24-elya-diagnostics-at-the-edges
        // D1); `ariadne` counts characters unless told.
        let mut report = Report::build(kind, id, offset)
            .with_config(Config::default().with_index_type(IndexType::Byte))
            .with_code(&d.code)
            .with_message(&d.message);
        for l in &d.labels {
            let (file, start) = at(l.span.start);
            let len = l.span.end.saturating_sub(l.span.start) as usize;
            report = report.with_label(
                AriadneLabel::new((file, start..start + len))
                    .with_message(&l.message)
                    .with_color(Color::Red),
            );
        }
        for h in &d.helps {
            report = report.with_help(h);
        }
        let _ = report.finish().write(&mut cache, &mut buf);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::{SourceMap, Span};

    #[test]
    fn builder_sets_fields() {
        let d = Diagnostic::error("E0200", "unresolved name `foo`")
            .with_label(Span::new(0, 3), "not found in this scope")
            .with_help("did you mean `food`?");
        assert_eq!(d.code, "E0200");
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.labels.len(), 1);
        assert_eq!(d.helps.len(), 1);
    }

    #[test]
    fn render_mentions_code_and_message() {
        let sm = SourceMap::new("t.elya", "foo\n");
        let d = Diagnostic::error("E0200", "unresolved name `foo`")
            .with_label(Span::new(0, 3), "not found");
        let out = render(&[d], &sm);
        assert!(out.contains("E0200"), "render output: {out}");
        assert!(out.contains("unresolved name"), "render output: {out}");
    }
}
