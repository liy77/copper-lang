use std::vec;
use crate::tokenizer::{kind::TokenKind, tokens::Token};
pub mod result;
pub mod utils;
pub mod scope;
pub mod scope_manager;

use crate::utils::Consumed;
use crate::{ConsumeVar, ConsumedTrait};
use utils::convert_type;
use result::Result;

const COPPER_OPERATORS: [(&str, &str); 2] = [
    ("++", "+= 1"),
    ("--", "-= 1"),
];

#[derive(Debug)]
pub enum AppendMode {
    Append,
    AppendToMainFunction,
    ForceAppend,
    FFAppend,
    AppendWithSpace,
    AppendToMainFunctionWithSpace,
    ForceAppendWithSpace,
    FFAppendWithSpace,
}

pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
    result: Result,
    eof: bool,
    function_start: bool,
    seen_import: bool,
    current_import_vars: Vec<String>,
    is_import_list: bool,
    is_inside_class: bool,
    current_class: Option<String>,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens: tokens.into_iter().filter(|t| t.kind != TokenKind::Whitespace).collect(),
            current: 0,
            result: Result::new(),
            eof: false,
            function_start: false,
            seen_import: false,
            current_import_vars: vec![],
            is_import_list: false,
            is_inside_class: false,
            current_class: None,
        }
    }

    pub fn current(&self) -> Option<&Token> {
        self.tokens.get(self.current)
    }

    pub fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.current + 1)
    }

    pub fn peek_kind(&self) -> Option<TokenKind> {
        self.peek().map(|t| t.kind.clone())
    }

    pub fn peek_value(&self) -> Option<String> {
        self.peek().map(|t| t.value.clone())
    }

    pub fn select(&self, index: usize) -> Option<&Token> {
        self.tokens.get(index)
    }

    pub fn next(&mut self) {
        self.current += 1;
        self.check_eof();
    }

    pub fn check_eof(&mut self) {
        if self.current >= self.tokens.len() {
            self.eof = true;
        }
    }

    pub fn append(&mut self, value: &str, mode: AppendMode) {
        match mode {
            AppendMode::Append => self.result.append(value, false),
            AppendMode::AppendToMainFunction => self.result.append_to_main_function(value, false),
            AppendMode::ForceAppend => self.result.force_append(value, false),
            AppendMode::FFAppend => self.result.ff_append(value, false),
            AppendMode::AppendWithSpace => self.result.append(value, true),
            AppendMode::AppendToMainFunctionWithSpace => self.result.append_to_main_function(value, true),
            AppendMode::ForceAppendWithSpace => self.result.force_append(value, true),
            AppendMode::FFAppendWithSpace => self.result.ff_append(value, true),
        }
    }

    pub fn value(&self) -> String {
        self.current().map_or(String::new(), |t| t.value.clone())
    }

    pub fn kind(&self) -> TokenKind {
        self.current().map_or(TokenKind::Eof, |t| t.kind.clone())
    }

    pub fn parse_mut(&mut self) -> Consumed {
        let mut consumed = 0;
        if self.value() == "mut" && self.peek_kind() == Some(TokenKind::Identifier) {
            if let Some(next) = self.select(self.current + 3) {
                if next.value != "=" {
                    consumed += 2;
                    self.append(&format!("let mut {}", self.peek_value().unwrap_or_default()), AppendMode::AppendWithSpace);
                }
            }
        }
        Consumed::consume(consumed)
    }

    pub fn parse_var(&mut self) -> Consumed {
        let mut consumed = 0;
        if self.kind() == TokenKind::Identifier && self.peek_value() == Some("=".to_string()) {
            if let Some(var_value) = self.select(self.current + 2) {
                if var_value.value != "=" {
                    consumed += 2;
                    self.append(&format!("let {} = ", self.value()), AppendMode::AppendWithSpace);
                }
            }
        }
        Consumed::consume(consumed)
    }

    pub fn parse_any(&mut self) -> Consumed {
        self.append(&self.value(), AppendMode::Append);
        Consumed::consume(1)
    }

    pub fn parse_function(&mut self) -> Consumed {
        if self.value() == "func" {
            self.function_start = true;
            self.result.enter_function();
            return Consumed::consume(1);
        }
        Consumed::consume(0)
    }

    pub fn parse_function_params(&mut self) -> Consumed {
        let mut consumed = 0;
        if self.value() == "(" && self.kind() == TokenKind::ParametersStart {
            self.append(&self.value(), AppendMode::Append);
            consumed += 1;
        }
        if self.value() == ")" && self.kind() == TokenKind::ParametersEnd {
            if self.result.is_function {
                self.append(&format!(") -> {}", self.result.return_type), AppendMode::Append);
                self.result.exit_function();
                self.result.is_inside_function = true;
            } else {
                self.append(&self.value(), AppendMode::Append);
            }
            consumed += 1;
        }
        Consumed::consume(consumed)
    }

    pub fn parse_function_body(&mut self) -> Consumed {
        let mut consumed = 0;
        if self.value() == "{" && self.kind() == TokenKind::BraceStart {
            self.append(&self.value(), AppendMode::Append);
            consumed += 1;
        }
        if self.value() == "}" && self.kind() == TokenKind::BraceEnd {
            if !self.seen_import && !self.is_inside_class {
                self.append(&self.value(), AppendMode::Append);
                self.append("\n", AppendMode::AppendWithSpace);
                self.result.is_inside_function = false;
                consumed += 1;
            }
        }
        Consumed::consume(consumed)
    }

    pub fn parse_operator(&mut self) -> Consumed {
        let mut consumed = 0;
        for (copper, rust) in COPPER_OPERATORS.iter() {
            if let Some(next_token) = self.peek() {
                if (next_token.value.clone() + &self.value()) == *copper {
                    self.append(rust, AppendMode::Append);
                    consumed += 2;
                    break;
                }
            }
        }
        Consumed::consume(consumed)
    }

    pub fn parse_import(&mut self) -> Consumed {
        let mut consumed = 0;
        if self.value() == "from" && self.kind() == TokenKind::From {
            consumed += 1;
        }
        if self.value() == "import" && self.kind() == TokenKind::Import {
            self.seen_import = true;
            self.append("use", AppendMode::AppendWithSpace);
            consumed += 1;
        }
        if self.value() == "{" && self.kind() == TokenKind::BraceStart && self.seen_import {
            self.is_import_list = true;
            consumed += 1;
        }
        if self.value() == "}" && self.kind() == TokenKind::BraceEnd && self.seen_import {
            consumed += 1;
        }
        if self.kind() == TokenKind::ModuleVar {
            self.current_import_vars.push(self.value());
            consumed += 1;
        }
        if self.kind() == TokenKind::ModulePath {
            if self.seen_import {
                self.append(&self.value(), AppendMode::Append);
                if !self.current_import_vars.is_empty() {
                    if self.is_import_list {
                        self.append(&"::".to_string(), AppendMode::Append);
                        self.append(&"{".to_string(), AppendMode::Append);
                        self.append(&self.current_import_vars.join(", "), AppendMode::Append);
                        self.append(&"}".to_string(), AppendMode::Append);
                        self.is_import_list = false;
                    } else {
                        let var = self.current_import_vars[0].clone();
                        self.append(&" as".to_string(), AppendMode::AppendWithSpace);
                        self.append(&var, AppendMode::Append);
                    }
                    self.current_import_vars.clear();
                }
                self.seen_import = false;
                consumed += 1;
            } else {
                self.append(&self.value(), AppendMode::Append);
                consumed += 1;
            }
        }
        Consumed::consume(consumed)
    }

    pub fn parse_regex(&mut self) -> Consumed {
        let mut consumed = 0;
        if self.kind() == TokenKind::Regex {
            if !self.result.has_required_import("regex") {
                self.result.add_required_import("regex");
            }
            let value = self.value();
            let var = "__regex__";
            let resolved_regex = value.trim_start_matches('/').trim_end_matches('/');
            self.append(&format!("{}::Regex::new({})", var, &format!("r\"{}\"", resolved_regex)), AppendMode::Append);
            consumed += 1;
        }
        Consumed::consume(consumed)
    }

    // Analisa a definição de uma classe
    pub fn parse_class_definition(&mut self) -> Consumed {
        // Detecta "class Nome { ... }"
        if self.value() == "class" && self.kind() == TokenKind::Keyword {
            let mut consumed = 1; // conta o 'class'
            // nome da classe
            let class_name = if let Some(tok) = self.select(self.current + consumed) {
                if tok.kind == TokenKind::Identifier {
                    consumed += 1;
                    tok.value.clone()
                } else {
                    return Consumed::consume(0);
                }
            } else {
                return Consumed::consume(0);
            };
            self.current_class = Some(class_name.clone());
            self.is_inside_class = true;
    
            // coleta todos os tokens entre chaves, incluindo as próprias '{' e '}'
            let mut brace_count = 0;
            let mut class_tokens = Vec::new();
            // primeiro, avança até achar a primeira '{'
            while let Some(tok) = self.select(self.current + consumed) {
                consumed += 1;
                if tok.kind == TokenKind::BraceStart {
                    brace_count = 1;
                    class_tokens.push(tok.clone());
                    break;
                }
            }
            // agora coleta até fechar todas as chaves
            while brace_count > 0 {
                if let Some(tok) = self.select(self.current + consumed) {
                    consumed += 1;
                    match tok.kind {
                        TokenKind::BraceStart => brace_count += 1,
                        TokenKind::BraceEnd   => brace_count -= 1,
                        _ => {}
                    }
                    class_tokens.push(tok.clone());
                } else {
                    break;
                }
            }
    
            // processa membros e anexa o Rust gerado
            let parsed = self.process_class_members(&class_tokens, &class_name);
            self.append(&parsed, AppendMode::AppendWithSpace);
    
            // reset
            self.is_inside_class = false;
            self.current_class = None;
            return Consumed::consume(consumed.try_into().unwrap());
        }
    
        Consumed::consume(0)
    }
    
    fn process_class_members(&mut self, class_tokens: &[Token], class_name: &str) -> String {
        let tokens: Vec<&Token> = class_tokens
            .iter()
            .filter(|t| t.kind != TokenKind::Newline)
            .collect();
        if tokens.len() < 2 {
            return String::new();
        }
        let inner = &tokens[1..tokens.len()-1];
    
        let mut output = String::new();
        let mut fields = Vec::new();
    
        // Collect the fields
        let mut i = 0;
        while i + 2 < inner.len() {
            let a = &inner[i];
            let b = &inner[i+1];
            let c = &inner[i+2];
            let is_colon = (b.kind == TokenKind::Operator || b.kind == TokenKind::Colon) && b.value == ":";
            let is_type = c.kind == TokenKind::Identifier
                || c.kind == TokenKind::ParamType
                || c.kind == TokenKind::Keyword;
    
            if a.kind == TokenKind::Identifier && is_colon && is_type {
                fields.push((a.value.clone(), c.value.clone()));
                i += 3;
            } else {
                break;
            }
        }
    
        // Build the struct
        output.push_str(&format!("struct {} {{\n", class_name));
        for (n, t) in &fields {
            output.push_str(&format!("    {}: {},\n", n, t));
        }
        output.push_str("}\n\n");
    
        // Build the constructor
        output.push_str(&format!("impl {} {{\n", class_name));
        if let Some(pos) = inner
            .iter()
            .position(|t| t.kind == TokenKind::Identifier && t.value == class_name)
        {
            if pos + 1 < inner.len() && inner[pos + 1].kind == TokenKind::ParenthesesStart {
                // coleta params
                let mut params = Vec::new();
                let mut j = pos + 2;
                while j + 2 < inner.len() && inner[j].kind == TokenKind::Param {
                    let pname = inner[j].value.clone();
                    let sep = &inner[j + 1];
                    let ptyp = &inner[j + 2];
                    let ok_sep = (sep.kind == TokenKind::Operator || sep.kind == TokenKind::Colon) && sep.value == ":";
                    if ok_sep && ptyp.kind == TokenKind::ParamType {
                        params.push((pname.clone(), ptyp.value.clone()));
                        j += 3;
                        if j < inner.len() && inner[j].value == "," {
                            j += 1;
                        }
                    } else {
                        break;
                    }
                }
    
                // Jump to the first '{'
                while j < inner.len() && inner[j].kind != TokenKind::BraceStart {
                    j += 1;
                }
                if j >= inner.len() {
                    return output;
                }
    
                // Extract the assignments
                let mut assigns = Vec::new();
                
                // Initialize for the class body analysis
                let mut bi: usize = j + 1;
                while bi < inner.len() {
                    let t0 = &inner[bi];
                    if t0.kind == TokenKind::BraceEnd {
                        break;
                    }
                    // detecta "self . campo ="
                    if t0.kind == TokenKind::Keyword
                        && t0.value == "self"
                        && bi + 3 < inner.len()
                        && inner[bi + 1].kind == TokenKind::Dot
                        && inner[bi + 2].kind == TokenKind::Identifier
                        && inner[bi + 3].value == "="
                    {
                        let field = inner[bi + 2].value.clone();
                        // Collect all tokens until the end of the statement or block
                        let mut expr_tokens = Vec::new();
                        let mut xi = bi + 4;
    
                        while xi < inner.len() {
                            let tt = &inner[xi];
                            // Found statement end or block end
                            if tt.value == ";" || tt.kind == TokenKind::BraceEnd {
                                break;
                            }
                            // Found new assignment
                            if tt.kind == TokenKind::Keyword
                                && tt.value == "self"
                                && xi + 3 < inner.len()
                                && inner[xi + 1].kind == TokenKind::Dot
                                && inner[xi + 2].kind == TokenKind::Identifier
                                && inner[xi + 3].value == "="
                            {
                                break;
                            }
    
                            expr_tokens.push(tt.value.clone());
                            xi += 1;
                        }
    
                        let expr = expr_tokens.join(" ").trim().to_string();
                        assigns.push((field, expr));
                        bi = xi;
                        continue;
                    }
    
                    bi += 1;
                }
    
                // Generate the constructor signature
                let sig = params
                    .iter()
                    .map(|(n, t)| format!("{}: {}", n, t))
                    .collect::<Vec<_>>()
                    .join(", ");
    
                // Initialize the fields
                let init = fields
                    .iter()
                    .map(|(field_name, _)| {
                        // Procura nos assigns se este campo tem uma inicialização explícita
                        if let Some((_, expr)) = assigns.iter().find(|(f, _)| f == field_name) {
                            format!("{}: {}", field_name, expr)
                        } 
                        else if let Some((param_name, _)) = params.iter().find(|(n, _)| n == field_name) {
                            format!("{}: {}", field_name, param_name)
                        } 
                        else {
                            // Default if not found in assigns or params
                            format!("{}: Default::default()", field_name)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
    
                output.push_str(&format!("    pub fn new({}) -> Self {{\n", sig));
                output.push_str(&format!("        Self {{ {} }}\n", init));
                output.push_str("    }\n\n");
            }
        }
    
        // métodos
        let mut k = 0;
        while k < inner.len() {
            if k + 3 < inner.len() {
                let return_type = &inner[k];
                let name_t = &inner[k + 1];
                let pstart = &inner[k + 2];
    
                let valid_return_type = return_type.kind == TokenKind::Identifier
                    || return_type.kind == TokenKind::ParamType
                    || return_type.kind == TokenKind::Keyword;
    
                if valid_return_type
                    && name_t.kind == TokenKind::Identifier
                    && pstart.kind == TokenKind::ParenthesesStart
                {
                    let mut found_self = false;
                    let mut idx = k + 3;
                    while idx < inner.len() {
                        let param = &inner[idx];
                        if param.value == "self" {
                            found_self = true;
                        }
                        if param.kind == TokenKind::ParametersEnd {
                            break;
                        }
                        idx += 1;
                    }
    
                    if found_self && idx < inner.len() && inner[idx].kind == TokenKind::ParametersEnd {
                        let mut body = Vec::new();
                        idx += 1;
                        while idx < inner.len() && inner[idx].kind != TokenKind::BraceStart {
                            idx += 1;
                        }

                        if idx < inner.len() {
                            let mut depth = 1;
                            idx += 1;
                            while idx < inner.len() && depth > 0 {
                                let tt = &inner[idx];
                                if tt.kind == TokenKind::BraceStart {
                                    depth += 1;
                                } else if tt.kind == TokenKind::BraceEnd {
                                    depth -= 1;
                                }
                                if depth > 0 {
                                    body.push(tt.value.clone());
                                }
                                idx += 1;
                            }
    
                            let body_str = body.join(" ");
                            let rust_type = convert_type(&return_type.value);
                            if rust_type == "()" {
                                output.push_str(&format!("    pub fn {}(&self) {{\n", name_t.value));
                            } else {
                                output.push_str(&format!("    pub fn {}(&self) -> {} {{\n", name_t.value, rust_type));
                            }
                            output.push_str(&format!("        {}\n", body_str));
                            output.push_str("    }\n\n");
                        }
                        k = idx;
                        continue;
                    }
                }
            }
            k += 1;
        }
    
        output.push_str("}\n");
        output
    }

    // Função principal de análise
    pub fn parse(&mut self) -> String {
        loop {
            if self.eof {
                self.result.write_main_function();
                break self.result.get().expect("Format Error");
            }
    
            if let Some(token) = self.current() {
                match token.kind {
                    TokenKind::Eof => {
                        self.eof = true;
                    },
                    TokenKind::Identifier | TokenKind::Keyword => {
                        self.parse_mut()
                            .or(|| self.parse_var())
                            .or(|| self.parse_class_definition())
                            .or(|| self.parse_function())
                            .or(|| self.parse_any())
                            .consume_var(&mut self.current);
                    },
                    TokenKind::ParametersEnd | TokenKind::ParametersStart => {
                        self.parse_function_params()
                            .consume_var(&mut self.current);
                    },
                    TokenKind::ParamType => {
                        self.append(&convert_type(&self.value()), AppendMode::Append);
                        self.next();
                    },
                    TokenKind::ReturnType => {
                        self.result.return_type(convert_type(&self.value()));
                        self.next();
                    },
                    TokenKind::BraceStart | TokenKind::BraceEnd => {
                        self.parse_import()
                            .or(|| self.parse_function_body())
                            .or(|| self.parse_any())
                            .consume_var(&mut self.current);
                    },
                    TokenKind::From | TokenKind::ModuleVar | TokenKind::ModulePath | TokenKind::Import => {
                        self.parse_import()
                            .or(|| self.parse_any())
                            .consume_var(&mut self.current);
                    },
                    TokenKind::Operator => {
                        self.parse_operator()
                            .or(|| self.parse_any())
                            .consume_var(&mut self.current);
                    },
                    TokenKind::Regex => {
                        self.parse_regex()
                            .or(|| self.parse_any())
                            .consume_var(&mut self.current);
                    },
                    _ => {
                        self.append(&self.value(), AppendMode::Append);
                        self.next();
                    }
                }
            } else {
                self.result.write_main_function();
                break self.result.get().expect("Format Error");
            }
    
            self.check_eof();
        }
    }
}

pub fn parse(tokens: Vec<Token>) -> String {
    let mut parser = Parser::new(tokens);
    parser.parse()
}