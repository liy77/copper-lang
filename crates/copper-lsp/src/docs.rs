//! Document store: per-URI text buffer + cached parse.
//!
//! `Backend` owns one of these and updates it on every `didOpen` /
//! `didChange`. Re-tokenizes the whole document on each change — fine for
//! files under a few thousand lines; incremental tokenization is a future
//! optimization.

use copper_syntax::ast::{ParsedFile, Span};
use dashmap::DashMap;
use ropey::Rope;
use tower_lsp::lsp_types::{Position, Range, Url};

#[derive(Default)]
pub struct DocumentMap {
    docs: DashMap<Url, Document>,
}

pub struct Document {
    pub text: Rope,
    pub parsed: ParsedFile,
}

impl Document {
    fn new(text: String) -> Self {
        let parsed = copper_syntax::ast::parse(&text);
        Self {
            text: Rope::from_str(&text),
            parsed,
        }
    }

    fn rebuild(&mut self, new_text: String) {
        self.parsed = copper_syntax::ast::parse(&new_text);
        self.text = Rope::from_str(&new_text);
    }

    /// Convert a byte-offset span into an LSP `Range` (line/character pair,
    /// UTF-16 code units per spec).
    pub fn span_to_range(&self, span: Span) -> Range {
        Range {
            start: self.byte_to_position(span.start as usize),
            end: self.byte_to_position(span.end as usize),
        }
    }

    fn byte_to_position(&self, byte: usize) -> Position {
        let byte = byte.min(self.text.len_bytes());
        let char_idx = self.text.byte_to_char(byte);
        let line = self.text.char_to_line(char_idx);
        let line_start_char = self.text.line_to_char(line);
        let character = char_idx - line_start_char;
        Position {
            line: line as u32,
            character: character as u32,
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
