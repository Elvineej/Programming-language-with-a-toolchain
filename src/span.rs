//! Byte spans, spanned nodes, and offset→line/col lookup.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub const EMPTY: Span = Span { start: 0, end: 0 };

    pub fn new(start: u32, end: u32) -> Span {
        Span { start, end }
    }

    pub fn merge(self, other: Span) -> Span {
        Span::new(self.start.min(other.start), self.end.max(other.end))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

pub fn spanned<T>(node: T, span: Span) -> Spanned<T> {
    Spanned { node, span }
}

/// Owns one source file's text and precomputed line offsets.
#[derive(Debug)]
pub struct SourceMap {
    name: String,
    text: String,
    line_starts: Vec<u32>,
}

impl SourceMap {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> SourceMap {
        let text = text.into();
        let mut line_starts = vec![0u32];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push((i + 1) as u32);
            }
        }
        SourceMap {
            name: name.into(),
            text,
            line_starts,
        }
    }

    /// 1-based (line, column) for a byte offset.
    pub fn location(&self, offset: u32) -> (u32, u32) {
        let line_idx = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let col = offset - self.line_starts[line_idx] + 1;
        (line_idx as u32 + 1, col)
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_maps_offset_to_line_and_col() {
        let sm = SourceMap::new("t.elya", "ab\ncd\n");
        assert_eq!(sm.location(0), (1, 1)); // 'a'
        assert_eq!(sm.location(1), (1, 2)); // 'b'
        assert_eq!(sm.location(3), (2, 1)); // 'c'
        assert_eq!(sm.location(4), (2, 2)); // 'd'
    }

    #[test]
    fn merge_covers_both_spans() {
        let a = Span::new(2, 5);
        let b = Span::new(7, 9);
        assert_eq!(a.merge(b), Span::new(2, 9));
    }
}
