//! Byte spans, spanned nodes, and offset→line/col lookup.

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

/// One file of a program: its name, its first offset in the program's span space,
/// its text and its line offsets.
#[derive(Debug)]
struct File {
    name: String,
    base: u32,
    text: String,
    line_starts: Vec<u32>,
}

/// Owns a program's source files (one for a single-file program; one per module
/// since modules part 1). Each file has its own range of span offsets, one past
/// the end of the file before it, so a span names its file.
#[derive(Debug)]
pub struct SourceMap {
    files: Vec<File>,
}

impl SourceMap {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> SourceMap {
        let mut sm = SourceMap { files: Vec::new() };
        sm.push(name.into(), text.into(), 0);
        sm
    }

    fn push(&mut self, name: String, text: String, base: u32) {
        let mut line_starts = vec![0u32];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push((i + 1) as u32);
            }
        }
        self.files.push(File {
            name,
            base,
            text,
            line_starts,
        });
    }

    /// The offset the next file would start at.
    pub fn next_base(&self) -> u32 {
        self.files
            .last()
            .map_or(0, |f| f.base + f.text.len() as u32 + 1)
    }

    /// Adds a file at [`SourceMap::next_base`], returning that base.
    pub fn add_file(&mut self, name: impl Into<String>, text: impl Into<String>) -> u32 {
        let base = self.next_base();
        self.push(name.into(), text.into(), base);
        base
    }

    fn file(&self, offset: u32) -> &File {
        let i = self
            .files
            .partition_point(|f| f.base <= offset)
            .saturating_sub(1);
        &self.files[i]
    }

    /// The file holding `offset`: its name, its text and its base.
    pub fn file_of(&self, offset: u32) -> (&str, &str, u32) {
        let f = self.file(offset);
        (&f.name, &f.text, f.base)
    }

    /// Every file: name and text.
    pub fn files(&self) -> impl Iterator<Item = (&str, &str)> {
        self.files
            .iter()
            .map(|f| (f.name.as_str(), f.text.as_str()))
    }

    /// 1-based (line, column) for a byte offset, in its own file.
    pub fn location(&self, offset: u32) -> (u32, u32) {
        let f = self.file(offset);
        let offset = offset - f.base;
        let line_idx = match f.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let col = offset - f.line_starts[line_idx] + 1;
        (line_idx as u32 + 1, col)
    }

    /// The root file's text.
    pub fn text(&self) -> &str {
        &self.files[0].text
    }

    /// The root file's name.
    pub fn name(&self) -> &str {
        &self.files[0].name
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
