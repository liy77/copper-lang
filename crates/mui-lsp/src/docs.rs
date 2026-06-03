//! Per-URI document store: text buffer + cached MUI parse.
//!
//! The `Backend` owns one of these and refreshes it on every `didOpen` /
//! `didChange`. The whole file is re-parsed on each change — `mui-syntax` is
//! cheap, and these files are small.

use dashmap::DashMap;
use mui_syntax::ast::Document as MuiDocument;
use ropey::Rope;
use tower_lsp::lsp_types::{Position, Range, Url};

use copper_syntax::ast::Span;

#[derive(Default)]
pub struct DocumentMap {
    docs: DashMap<Url, Document>,
}

pub struct Document {
    pub text: Rope,
    pub parsed: MuiDocument,
}

impl Document {
    fn new(text: String) -> Self {
        let parsed = mui_syntax::parse(&text);
        Self {
            text: Rope::from_str(&text),
            parsed,
        }
    }

    fn rebuild(&mut self, new_text: String) {
        self.parsed = mui_syntax::parse(&new_text);
        self.text = Rope::from_str(&new_text);
    }

    /// The whole document text as a `String` (for regex-style scans).
    pub fn full_text(&self) -> String {
        self.text.to_string()
    }

    /// Byte-offset span → LSP `Range` (UTF-16 line/character per the spec).
    pub fn span_to_range(&self, span: Span) -> Range {
        Range {
            start: self.byte_to_position(span.start as usize),
            end: self.byte_to_position(span.end as usize),
        }
    }

    /// Byte offset of an LSP position (UTF-8 byte index into the buffer).
    pub fn position_to_byte(&self, pos: Position) -> usize {
        let line = (pos.line as usize).min(self.text.len_lines().saturating_sub(1));
        let line_start_char = self.text.line_to_char(line);
        let line_slice = self.text.line(line);
        // Clamp character to the line length (chars), then convert to byte.
        let ch = (pos.character as usize).min(line_slice.len_chars());
        self.text.char_to_byte(line_start_char + ch)
    }

    fn byte_to_position(&self, byte: usize) -> Position {
        let byte = byte.min(self.text.len_bytes());
        let char_idx = self.text.byte_to_char(byte);
        let line = self.text.char_to_line(char_idx);
        let line_start_char = self.text.line_to_char(line);
        Position {
            line: line as u32,
            character: (char_idx - line_start_char) as u32,
        }
    }
}

impl DocumentMap {
    pub fn open(&self, uri: Url, text: String) {
        self.docs.insert(uri, Document::new(text));
    }

    pub fn change(&self, uri: &Url, text: String) {
        if let Some(mut entry) = self.docs.get_mut(uri) {
            entry.rebuild(text);
        } else {
            self.docs.insert(uri.clone(), Document::new(text));
        }
    }

    pub fn close(&self, uri: &Url) {
        self.docs.remove(uri);
    }

    pub fn get(&self, uri: &Url) -> Option<dashmap::mapref::one::Ref<'_, Url, Document>> {
        self.docs.get(uri)
    }
}
