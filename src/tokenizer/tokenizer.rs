use std::process::exit;

use crate::{tokenizer::tokens::*, ConsumedTrait};

use super::{kind::TokenKind, tokens::Token};
use once_cell::sync::Lazy;
use regex::Regex;
use crate::utils::Consumed;

pub(super) type EndToken = Token;

impl EndToken {
    pub fn new_end(kind: TokenKind, value: String, length: usize, data: Data) -> Self {
        Self {
            kind,
            value,
            length,
            data,
            generated: false,
            origin: None,
            location_data: None,
        }
    }
}

pub(self) const TRAILING_SPACES: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+$").unwrap());
pub(self) const COMPOUND_SIGNS: [&'static str; 9] = ["::", "-=", "+=", "/=", "*=", "%=", "&=", "^=", "|="];
pub(self) const COMPARE_SIGNS: [&'static str; 6] = ["==", "!=", "<=", ">=", ">", "<"];
pub(self) const ARITHMETIC_SIGNS: [&'static str; 5] = ["+", "-", "*", "/", "%"];
pub(self) const RANGE_SIGNS: [&'static str; 2] = ["..", "..="];
pub(self) const OPERATORS: [&'static str; 9] = ["!", "&", "|", "^", "~", ":", "?", ".", "="];
pub(self) const COMMA_SEPARATORS: [&'static str; 2] = [",", ";"];
pub(self) const BOOL: [&'static str; 2] = ["true", "false"];
pub(self) const BOM: u32 = 65279;
pub(self) const RUST_KEYWORDS: &[&'static str] = &[
        // Control Flow Keywords
        "if", "else", "match", "loop", "while", "for", "break", "continue", "return",

        // Visibility and Access Modifiers
        "pub", "crate", "self", "super", "mod",
    
        // Declaration Keywords
        "let", "const", "static", "mut",
    
        // Types and Traits Keywords
        "struct", "enum", "union", "trait", "impl", "type",
    
        // Memory and Safety Control Keywords
        "unsafe", "async", "await", "move", "dyn",
    
        // Module Handling Keywords
        "extern", "use",
    
        // Function Definition and Implementation Keywords
        "fn", "self", "Self",
    
        // Data Manipulation Keywords
        "ref", "match", "in", "as", "Box",
    
        // Reliability and Testing Control Keywords
        "where", "macro", "macro_rules", "proc",
    
        // Result and Error Handling Keywords
        "Result", "Option", "Some", "None", "Ok", "Err",
    
        // Standard Data and System Types Keywords
        "bool", "char", "i8", "i16", "i32", "i64", "i128", "u8", "u16", "u32", "u64", "u128", "isize", "usize", "f32", "f64", "str",
    
        // Property and Pattern Keywords
        "true", "false",
    
        // Contextual Keywords
        "abstract", "become", "box", "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try",
];

pub(self) const COPPER_KEYWORDS: &[&'static str] = &[
    // Function Definition and Implementation Keywords
    "func", "$init", "$child",

    // Module Handling Keywords
    "import", "from",

    // Class Keywords
    "class", "extends",

    // Struct and Impl Keywords
    "struct", "impl", "trait", "for",

    // Data Format Types
    "json", "xml", "toml",
];

pub(crate) struct Tokenizer {
    source: String,
    index: usize,
    chunk: String,
    chunk_line: isize,
    chunk_column: usize,
    chunk_offset: usize,
    tokens: Vec<Token>,
    ends: Vec<EndToken>,
    seen_for: bool,
    seen_import: bool,
    seen_public: bool,
    import_specifier_list: bool,
    location_data_compensations: Vec<usize>,
}

impl Tokenizer {
    pub fn new(source: String) -> Self {
        let mut s = Self {
            source,
            index: 0,
            chunk: String::new(),
            chunk_line: 0,
            chunk_column: 0,
            chunk_offset: 0,
            tokens: Vec::new(),
            ends: Vec::new(),
            seen_for: false,
            seen_import: false,
            seen_public: false,
            import_specifier_list: false,
            location_data_compensations: vec![],
        };

        s.clean_source();

        s
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        while self.index < self.source.len() {
            // Find the end of current line
            let line_end = self.source[self.index..].find('\n')
                .map(|pos| self.index + pos + 1)
                .unwrap_or(self.source.len());
            
            self.chunk = self.source[self.index..line_end].to_string();
            self.chunk_line += 1;
            self.chunk_column = 0;
            self.chunk_offset = self.index;

            while self.chunk_column < self.chunk.len() {
                let consumed = self.identifier_token()
                    .or(|| self.number_token())
                    .or(|| self.string_token())
                    .or(|| self.comment_token())
                    .or(|| self.regex_token())
                    .or(|| self.operator_token())
                    .or(|| self.symbol_token())
                    .or(|| self.whitespace_token())
                    .or(|| self.line_break_token());

                if !consumed.is_consumed() {
                    self.next_char();
                }
            }
            
            // Move to next line
            self.index = line_end;
        }
        
        if self.ends.len() > 0 {
            let last = self.ends.last().unwrap();
            let location = last.origin.as_ref().unwrap().location_data.clone().unwrap();

            self.error(&format!("Unterminated token {}\nOrigin: {}:{}", last.value, location.last_line, location.last_column));
        }

        if self.kind() != Some(TokenKind::Newline) {
            self.token(TokenKind::Newline, ";\n".to_string());
        }

        self.token(TokenKind::Eof, "END_OF_FILE".to_string());
        self.tokens.clone()
    }

    pub fn drain(&mut self, end: usize) {
        // Advance chunk_column by the number of consumed characters
        let consumed = end - self.index;
        self.chunk_column += consumed;
        self.index += consumed;
    }

    pub fn operator_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        let mut kind = TokenKind::Operator;

        for sign in COMPOUND_SIGNS.iter() {
            if self.chunk.get(self.chunk_column..).unwrap_or_default().starts_with(sign) {
                value.push_str(sign);
                consumed += sign.len();
                self.chunk_column += sign.len();
                break;
            }
        }

        if consumed == 0 {
            for sign in COMPARE_SIGNS.iter() {
                if self.chunk.get(self.chunk_column..).unwrap_or_default().starts_with(sign) {
                    value.push_str(sign);
                    consumed += sign.len();
                    self.chunk_column += sign.len();
                    break;
                }
            }
        }

        if consumed == 0 {
            for sign in ARITHMETIC_SIGNS.iter() {
                if self.chunk.get(self.chunk_column..).unwrap_or_default().starts_with(sign) {
                    value.push_str(sign);
                    consumed += sign.len();
                    self.chunk_column += sign.len();
                    break;
                }
            }
        }

        if consumed == 0 {
            for sign in RANGE_SIGNS.iter().rev() {
                if self.chunk.get(self.chunk_column..).unwrap_or_default().starts_with(sign) {
                    value.push_str(sign);
                    consumed += sign.len();
                    self.chunk_column += sign.len();
                    kind = TokenKind::Range;
                    break;
                }
            }
        }

        if consumed == 0 {
            for sign in OPERATORS.iter() {
        if consumed == 0 {
            for sign in OPERATORS.iter() {
                if self.chunk.get(self.chunk_column..).unwrap_or_default().starts_with(sign) {
                    if *sign == ":" && self.kind() == Some(TokenKind::Param) {
                        value.push_str(":");
                        consumed += sign.len() + 1;
                        self.chunk_column += sign.len() + 1;
                        kind = TokenKind::Colon;
                        break;
                    } else if *sign == "." {
                        kind = TokenKind::Dot;

                        if self.seen_import {
                            value.push_str(sign);
                            consumed += sign.len();
                            self.chunk_column += sign.len();
                            self.add_to_value(sign);
                            
                            return Consumed::Consumed(consumed as isize);
                        }
                    }

                    value.push_str(sign);
                    consumed += sign.len();
                    self.chunk_column += sign.len();
                    break;
                }
            }
        }
            }
        }

        if consumed > 0 {
            self.token(kind, value);
        }

        Consumed::consume(consumed as isize)
    }

    pub fn symbol_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        let mut kind = TokenKind::Symbol;

        // Copper attributes symbol
        if self.current_char() == '$' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            kind = TokenKind::CurrencySign;
        }

        if self.current_char() == '<' || self.current_char() == '>' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            kind = match value.as_str() {
                "<" => TokenKind::AngleStart,
                ">" => TokenKind::AngleEnd,
                _ => TokenKind::Symbol
            };
        }

        if self.current_char() == '(' || self.current_char() == ')' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            kind = match value.as_str() {
                "(" => {
                    if self.kind() == Some(TokenKind::Identifier) {
                        self.end(TokenKind::ParametersEnd, ")".to_string());
                        TokenKind::ParenthesesStart
                    } else {
                        self.end(TokenKind::ParenthesesEnd, ")".to_string());
                        TokenKind::ParenthesesStart
                    }
                },
                ")" => {
                    if self.end_kind() == Some(TokenKind::ParametersEnd) {
                        self.skip_end();
                        TokenKind::ParametersEnd
                    } else {
                        self.skip_end();
                        TokenKind::ParenthesesEnd
                    }
                },
                _ => TokenKind::Symbol
            };
        }

        if self.current_char() == '[' || self.current_char() == ']' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            kind = match value.as_str() {
                "[" => {
                    self.end(TokenKind::BracketEnd, "]".to_string());
                    TokenKind::BracketStart
                },
                "]" => {
                    self.skip_end();
                    TokenKind::BracketEnd
                },
                _ => TokenKind::Symbol
            };
        }

        if self.current_char() == '{' || self.current_char() == '}' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            kind = match value.as_str() {
                "{" => {
                    if self.seen_import && self.value(true) == Some("import".to_string()) {
                        self.import_specifier_list = true;
                    }

                    self.end(TokenKind::BraceEnd, "}".to_string());
                    TokenKind::BraceStart
                },
                "}" => {
                    if self.import_specifier_list {
                        self.import_specifier_list = false;
                    }
                    
                    self.skip_end();
                    TokenKind::BraceEnd
                },
                _ => TokenKind::Symbol
            };
        }

        if COMMA_SEPARATORS.contains(&self.current_char().to_string().as_str()) {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            kind = match value.as_str() {
                "," => TokenKind::Comma,
                ";" => {
                    if self.peek() == '\n' {
                        self.next_char();
                        consumed += 1;
                        
                        if self.seen_import {
                            self.set_kind(TokenKind::ModulePath);
                            self.seen_import = false;
                        }

                        value.push_str("\n");

                        TokenKind::Newline
                    } else {
                        TokenKind::Semicolon

                    }
                },
                _ => TokenKind::Symbol
            };
        }

        if self.current_char() == '.' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            kind = TokenKind::Dot;
        }

        self.token(kind, value);

        Consumed::consume(consumed)
    }

    pub fn string_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        let kind = TokenKind::String;
        let mut escape = false;

        if self.current_char() == '"' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            while self.current_char() != '"' || escape {
                if self.current_char() == '\\' {
                    escape = !escape;
                } else {
                    escape = false;
                }

                value.push(self.current_char());
                self.next_char();
                consumed += 1;
            }

            value.push(self.current_char());
            self.next_char();
            consumed += 1;
        }

        self.token(kind, value);

        Consumed::consume(consumed)
    }

    pub fn number_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        let kind = TokenKind::Number;

        if self.current_char().is_numeric() {
            while self.current_char().is_numeric() {
                value.push(self.current_char());
                self.next_char();
                consumed += 1;
            }

            if self.current_char() == '.' {
                if self.peek().is_numeric() {
                    value.push(self.current_char());
                    self.next_char();
                    consumed += 1;

                    while self.current_char().is_numeric() {
                        value.push(self.current_char());
                        self.next_char();
                        consumed += 1;
                    }
                } else if self.peek() != '.' {
                    self.error(&format!("Invalid float or range: {}", self.peek()));
                }
            }
        }

        self.token(kind, value);
        
        Consumed::consume(consumed)

    }

    pub fn identifier_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        let mut kind = TokenKind::Unknown;

        if self.current_char() == '\'' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            // Continue capturing the lifetime name
            while self.current_char().is_alphanumeric() || self.current_char() == '_' {
                value.push(self.current_char());
                self.next_char();
                consumed += 1;
            }

            kind = TokenKind::Lifetime;
        } else if self.current_char().is_alphabetic() || self.current_char() == '_' {
            while self.current_char().is_alphanumeric() || self.current_char() == '_' {
                value.push(self.current_char());
                self.next_char();
                consumed += 1;
            }

            if BOOL.contains(&value.as_str()) {
                kind = TokenKind::Keyword;
            } else if COPPER_KEYWORDS.contains(&value.as_str()) {
                match value.as_str() {
                    "import" => {
                        self.seen_import = true;
                        kind = TokenKind::Import;
                    },
                    "from" => {
                        kind = TokenKind::From;
                    },
                    "as" => {
                        kind = TokenKind::As;
                    },
                    "public" => {
                        self.seen_public = true;
                        kind = TokenKind::Public;
                    },
                    "for" => {
                        self.seen_for = true;
                        kind = TokenKind::For;
                    },
                    "struct" => {
                        kind = TokenKind::Struct;
                    },
                    "impl" => {
                        kind = TokenKind::Impl;
                    },
                    "trait" => {
                        kind = TokenKind::Trait;
                    },
                    "json" => {
                        kind = TokenKind::Json;
                    },
                    "xml" => {
                        kind = TokenKind::Xml;
                    },
                    "toml" => {
                        kind = TokenKind::Toml;
                    },
                    "func" => {
                        kind = TokenKind::Keyword;
                    },
                    _ => {
                        kind = TokenKind::Keyword;
                    }
                }
            } else if RUST_KEYWORDS.contains(&value.as_str()) {
                kind = TokenKind::Keyword;
            } else if self.import_specifier_list {
                kind = TokenKind::ModuleVar;
            } else if self.seen_import {
                if self.value(true) == Some("from".to_string()) {
                    kind = TokenKind::ModulePath;
                } else {
                    if self.value(true) != Some(" ".to_string()) {
                        self.add_to_value(&value);
                        return Consumed::Consumed(consumed);
                    } else {
                        kind = TokenKind::ModuleVar;
                    }
                }
            } else {
                match self.last_token() {
                    Some(token) if token.value == "func" => {
                        kind = TokenKind::ReturnType
                    },
                    Some(_) if self.end_kind() == Some(TokenKind::ParametersEnd) => {
                        if self.value(true) == Some(":".to_string()) {
                            if self.current_char() == '?' {
                                self.next_char();
                                value.push('?');
                            }

                            kind = TokenKind::ParamType
                        } else {
                            kind = TokenKind::Param
                        }
                    },
                    _ => {
                        kind = TokenKind::Identifier
                    }
                }
            }
        }

        if consumed > 0 {
            self.token(kind, value);
        }
        
        Consumed::consume(consumed)

    }

    pub fn whitespace_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        let kind = TokenKind::Whitespace;

        if self.current_char() == ' ' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            while self.current_char() == ' ' {
                value.push(self.current_char());
                self.next_char();
                consumed += 1;
            }
        }

        self.token(kind, value);

        Consumed::consume(consumed)
    }

    pub fn line_break_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        let kind = TokenKind::Newline;

        if self.current_char() == '\n' && self.kind() != Some(TokenKind::Newline) {
            match self.kind() {
                Some(TokenKind::BraceStart) |
                Some(TokenKind::BracketStart) |
                Some(TokenKind::ParenthesesStart) |
                Some(TokenKind::ParametersStart) |
                Some(TokenKind::BraceEnd) => {
                    value.push(self.current_char()); 
                },
                _ => {
                    value.push_str(&(";".to_owned() + self.current_char().to_string().as_str())); 
                }
            }
            self.next_char();
            consumed += 1;

            if self.seen_import {
                self.seen_import = false;
                self.set_kind(TokenKind::ModulePath);
            }
        } else if self.current_char() == '\n' {
            value.push(self.current_char()); 
            self.next_char();
            consumed += 1;
        }

        self.token(kind, value);

        Consumed::consume(consumed)
    }

    pub fn comment_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        let kind = TokenKind::Comment;

        if self.current_char() == '/' && self.peek() == '/' {
            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            value.push(self.current_char());
            self.next_char();
            consumed += 1;

            while self.current_char() != '\n' {
                value.push(self.current_char());
                self.next_char();
                consumed += 1;
            }

            self.token(kind, value);
        }

        Consumed::consume(consumed)
    }

    pub fn json_object_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        
        // Check if there's a pattern: identifier whitespace = whitespace {
        let remaining_chunk = &self.chunk[self.chunk_column..];
        
        // Only process if at line start or after whitespace
        if self.chunk_column == 0 || 
           (self.chunk_column > 0 && self.chunk.chars().nth(self.chunk_column - 1).unwrap_or(' ').is_whitespace()) {
            
            // Regex to detect: identifier followed by spaces, =, spaces and {
            let json_obj_regex = Regex::new(r"^([a-zA-Z_][a-zA-Z0-9_]*)\s*=\s*\{").unwrap();
            
            if let Some(cap) = json_obj_regex.captures(remaining_chunk) {
                let _identifier = cap.get(1).unwrap().as_str();
                let full_match = cap.get(0).unwrap();
                let match_len = full_match.len();
                
                // Advance to JSON object start
                for _ in 0..match_len {
                    value.push(self.current_char());
                    self.next_char();
                    consumed += 1;
                }
                
                // Now need to collect all content until matching }
                let mut brace_count = 1;
                let mut json_content = String::new();
                
                while brace_count > 0 && self.chunk_column < self.chunk.len() {
                    let ch = self.current_char();
                    json_content.push(ch);
                    
                    match ch {
                        '{' => brace_count += 1,
                        '}' => brace_count -= 1,
                        _ => {}
                    }
                    
                    self.next_char();
                    consumed += 1;
                }
                
                // If we managed to close all braces, create JSON token
                if brace_count == 0 {
                    value.push_str(&json_content);
                    self.token(TokenKind::JsonObject, value);
                    return Consumed::consume(consumed as isize);
                }
            }
        }
        
        Consumed::consume(0)
    }

    pub fn regex_token(&mut self) -> Consumed {
        let mut consumed = 0;
        let mut value = String::new();
        let kind = TokenKind::Regex;
        
        // Only consider regex if it starts with '/'
        if self.current_char() != '/' {
            return Consumed::consume(0);
        }
        
        // Check if it's not a comment (//)
        if self.peek() == '/' {
            return Consumed::consume(0);
        }
        
        // Process only from current position in chunk
        let remaining_chunk = &self.chunk[self.chunk_column..];
        
        // Check again in chunk if it's a comment
        if remaining_chunk.starts_with("//") {
            return Consumed::consume(0);
        }
        
        let r = Regex::new(r"^(/)([^/]+)(/)?").unwrap(); // Added ^ for string start

        if let Some(cap) = r.captures(remaining_chunk) {
            let start = cap.get(1).unwrap().start();
            let end_cap = cap.get(3);

            if end_cap.is_none() {
                self.error("Unterminated regex literal");
            }

            let end = end_cap.unwrap().end();
            value.push_str(&remaining_chunk[start..end]);
            consumed = end - start;
            
            // Advance chunk_column instead of draining
            for _ in 0..consumed {
                self.next_char();
            }
        }

        self.token(kind, value);

        Consumed::consume(consumed as isize)
    }

    pub fn error(&self, message: &str) {
        let (line, column, _) = self.get_line_and_column(self.chunk_column);
        eprintln!("Error: {}\nLine: {}:{}", message, line + 1, column + 1); // +1 for 1-indexed line/column
        exit(1);
    }

    pub fn value(&self, use_origin: bool) -> Option<String> {
        let token = self.tokens.last();

        if token.is_none() {
            return None;
        }

        let token = token.unwrap();
        let value = token.value.clone();
        let origin = token.origin.clone();
    
        if use_origin && origin.is_some() {
            return Some(origin.unwrap().value.clone());
        }
    
        Some(value)
    }

    pub fn add_to_value(&mut self, value: &str) {
        if self.last_token().is_none() {
            return;
        }

        let token = self.tokens.last_mut().unwrap();
        token.value.push_str(value);
    }

    pub fn last_token(&self) -> Option<&Token> {
        self.tokens.iter().filter(|t| t.kind != TokenKind::Whitespace).last()
    } 

    pub fn token(&mut self, kind: TokenKind, value: String) -> &mut Token {
        let length = value.len();

        // If the length is 0, we want to modify the last token
        if length == 0 {
            if self.tokens.len() == 0 {
                return self.token(TokenKind::Unknown, " ".to_string());
            }

            return self.tokens.last_mut().unwrap();
        }

        let mut token = Token::new(kind, value, length, Data::None, false);
        token.set_location_data(self.create_location_data(self.chunk_offset, length));

        match kind {
            TokenKind::BraceStart      | 
            TokenKind::BracketStart    | 
            TokenKind::ParametersStart | 
            TokenKind::ParenthesesStart => {
                self.last_end().unwrap().set_origin(token.clone());
            }
            _ => {}            
        }

        self.tokens.push(token.clone());
        self.tokens.last_mut().unwrap()
    }

    pub fn skip_end(&mut self) {
        if self.ends.len() == 0 {
            return;
        }

        self.ends.remove(self.ends.len() - 1);
    }

    pub fn last_end(&mut self) -> Option<&mut Token> {
        self.ends.last_mut()
    }

    pub fn end(&mut self, kind: TokenKind, value: String) -> &mut EndToken {
        self.ends.push(EndToken::new_end(kind, value.clone(), value.len(), Data::None));

        self.ends.last_mut().unwrap()
    }

    pub fn set_kind(&mut self, kind: TokenKind) {
        let token = self.tokens.last_mut();

        if token.is_some() {
            token.unwrap().kind = kind;
        }
    }

    pub fn kind(&self) -> Option<TokenKind> {
        let token = &self.tokens.last();

        if token.is_some() {
            return Some(token.unwrap().clone().kind);
        }

        None
    }

    pub fn end_kind(&self) -> Option<TokenKind> {
        let token = &self.ends.last();

        if token.is_some() {
            return Some(token.unwrap().clone().kind);
        }

        None
    }

    pub fn create_location_data(&self, offset_in_chunk: usize, length: usize) -> LocationData {
        let last_char = if length > 0 {
            length - 1
        } else {
            0
        };

        let (first_line, first_column, range_start) = self.get_line_and_column(offset_in_chunk);
        let (last_line, last_column, end_offset) = self.get_line_and_column(offset_in_chunk + last_char);

        let range = (range_start, if length > 0 { end_offset + 1 } else { end_offset });

        LocationData {
            first_line,
            first_column,
            last_line,
            last_column,
            range
        }
    }

    pub fn get_location_data_compensation(&self, start: usize, mut end: usize) -> usize {
        let mut total_compensation = 0;
        let initial_end = end;
        let mut current = start;
        let mut iterations = 0;

        while start >= end && iterations < 1000 {
            iterations += 1;
            
            if current == end && start != initial_end {
                break;
            }

            let compensation = self.location_data_compensations.get(current);
            
            if compensation.is_some() {
                let compensation = compensation.unwrap();
                total_compensation += compensation;
                end += compensation;
            }

            current += 1;
            
            // Additional safety condition
            if current >= self.source.len() + 1000 {
                break;
            }
        }

        total_compensation
    }

    #[allow(unused_assignments)]
    pub fn get_line_and_column(&self, offset: usize) -> (isize, usize, usize) {
        let compensation = self.get_location_data_compensation(self.chunk_offset, self.chunk_offset + offset);
        let mut s = String::new();
        let mut line_count: usize = 0;
        let mut column = 0;
        let mut previous_lines_compensation = 0;
        let mut column_compensation = 0;

        if offset == 0 {
            return (self.chunk_line, self.chunk_column + compensation, self.chunk_offset + compensation);
        }

        if offset >= self.chunk.len() {
            s = self.chunk.clone();
        } else {
            let end = match offset {
                o if o > 0 => o,
                _ => self.chunk.len()
            };

            s = self.chunk.get(0..end.min(self.chunk.len())).unwrap().to_owned();
        }

        line_count = self.count_occurrences(&s, "\n");
        column = self.chunk_column;

        if line_count > 0 {
            let r = s.split("\n").collect::<Vec<&str>>();
            column = r.last().unwrap().len();
            previous_lines_compensation = self.get_location_data_compensation(self.chunk_offset, self.chunk_offset + offset - column);
            
            column_compensation = self.get_location_data_compensation(self.chunk_offset + offset - previous_lines_compensation - column, self.chunk_offset + offset + previous_lines_compensation);
        } else {
            column += s.len();
            column_compensation = compensation;
        }

        (self.chunk_line + line_count as isize, column + column_compensation, self.chunk_offset + offset + compensation)
    }

    pub fn count_occurrences(&self, string: &str, substr: &str) -> usize {
        if substr.is_empty() {
            return usize::MAX; // Use usize::MAX to represent infinity
        }
        let mut num = 0;
        let mut pos = 0;
        
        while let Some(p) = string[pos..].find(substr) {
            num += 1;
            pos += p + 1; // Increment position to continue search
        }
        
        num
    }
    
    // pub fn next_chunk(&self) -> String {
    //     self.source.get(self.index..).unwrap_or_default().to_owned()
    // }

    pub fn clean_source(&mut self) {
        let re = Regex::new(r"\r").unwrap();
        let mut thus_far = 0;
        let mut source = &self.source;
    
        if source.len() > 0 && source.chars().nth(0).unwrap() as u32 == BOM {
            self.source = source.chars().skip(1).collect::<String>();
            source = &self.source;
            self.location_data_compensations[0] = 1;
            thus_far += 1;
        }
        
        if TRAILING_SPACES.is_match(&source) {
            self.source = TRAILING_SPACES.replace_all(&source, "").to_string();
            source = &self.source;
            self.chunk_line -= 1;
            self.location_data_compensations.insert(0, self.location_data_compensations.get(0).unwrap_or(&1) - 1);
        }

        for (_, mat) in re.find_iter(&source).enumerate() {
            let offset = mat.start();

            while self.location_data_compensations.len() < (thus_far + offset) + 1 {
                self.location_data_compensations.push(0);
            }

            self.location_data_compensations[thus_far + offset] = 1;
            thus_far += offset;
        }

        self.source = re.replace_all(&source, "").to_string();
    }

    fn current_char(&self) -> char {
        // Use proper UTF-8 indexing
        if let Some((_, ch)) = self.chunk.char_indices().find(|(pos, _)| *pos == self.chunk_column) {
            ch
        } else {
            // Fallback for misaligned positions
            self.chunk.chars().nth(0).unwrap_or('\0')
        }
    }

    fn next_char(&mut self) {
        // Find the next character boundary
        if let Some((next_pos, _)) = self.chunk.char_indices().find(|(pos, _)| *pos > self.chunk_column) {
            self.chunk_column = next_pos;
        } else {
            // End of string
            self.chunk_column = self.chunk.len();
        }
    }

    fn peek(&self) -> char {
        self.chunk.chars().nth(self.chunk_column + 1).unwrap_or_default()
    }
}