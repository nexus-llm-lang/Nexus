use crate::types::Span;

/// Byte-offset ↔ LSP line/character converter.
pub struct LineIndex {
    line_starts: Vec<usize>,
    text_len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(i + 1);
            }
        }
        LineIndex {
            line_starts,
            text_len: text.len(),
        }
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    pub fn offset_to_position(&self, offset: usize) -> lsp_types::Position {
        let offset = offset.min(self.text_len);
        let line = self.line_starts.partition_point(|&s| s <= offset).saturating_sub(1);
        let col = offset - self.line_starts[line];
        lsp_types::Position {
            line: line as u32,
            character: col as u32,
        }
    }

    pub fn position_to_offset(&self, pos: lsp_types::Position) -> usize {
        let line = pos.line as usize;
        if line >= self.line_starts.len() {
            return self.text_len;
        }
        (self.line_starts[line] + pos.character as usize).min(self.text_len)
    }

    pub fn span_to_range(&self, span: &Span) -> lsp_types::Range {
        lsp_types::Range {
            start: self.offset_to_position(span.start),
            end: self.offset_to_position(span.end),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line() {
        let idx = LineIndex::new("hello");
        let pos = idx.offset_to_position(3);
        assert_eq!(pos.line, 0);
        assert_eq!(pos.character, 3);
    }

    #[test]
    fn multi_line() {
        let idx = LineIndex::new("ab\ncd\nef");
        assert_eq!(idx.offset_to_position(0), lsp_types::Position { line: 0, character: 0 });
        assert_eq!(idx.offset_to_position(3), lsp_types::Position { line: 1, character: 0 });
        assert_eq!(idx.offset_to_position(4), lsp_types::Position { line: 1, character: 1 });
        assert_eq!(idx.offset_to_position(6), lsp_types::Position { line: 2, character: 0 });
    }

    #[test]
    fn roundtrip() {
        let idx = LineIndex::new("ab\ncd\nef");
        let pos = lsp_types::Position { line: 1, character: 1 };
        let offset = idx.position_to_offset(pos);
        assert_eq!(offset, 4);
        assert_eq!(idx.offset_to_position(offset), pos);
    }
}
