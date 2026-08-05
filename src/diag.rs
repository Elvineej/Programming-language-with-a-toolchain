//! Structured diagnostics and human-readable rendering.

use crate::span::{SourceMap, Span};
use ariadne::{Color, Label as AriadneLabel, Report, ReportKind, Source};

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
    let id = src.name();
    for d in diags {
        let kind = match d.severity {
            Severity::Error => ReportKind::Error,
            Severity::Warning => ReportKind::Warning,
        };
        let offset = d.labels.first().map(|l| l.span.start as usize).unwrap_or(0);
        let mut report = Report::build(kind, id, offset)
            .with_code(&d.code)
            .with_message(&d.message);
        for l in &d.labels {
            report = report.with_label(
                AriadneLabel::new((id, l.span.start as usize..l.span.end as usize))
                    .with_message(&l.message)
                    .with_color(Color::Red),
            );
        }
        for h in &d.helps {
            report = report.with_help(h);
        }
        let _ = report
            .finish()
            .write((id, Source::from(src.text())), &mut buf);
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
