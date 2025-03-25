use std::vec;

use crate::tokenizer::{kind::TokenKind, tokens::Token};

pub mod result;
pub mod utils;
pub mod scope;
pub mod scope_manager;
pub mod class;

use crate::utils::Consumed;
use crate::{ConsumeVar, ConsumedTrait};
use utils::convert_type;
use result::Result;


const COPPER_OPERATORS : [(&str, &str); 2] = [
    ("++", "+= 1"),
    ("--", "-= 1"),
];

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
            tokens: tokens.iter().filter(|t| t.kind != TokenKind::Whitespace).map(|t| t.clone()).collect(),
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
        if self.peek().is_none() {
            return None;
        }

        Some(self.peek().unwrap().kind.clone())
    }

    pub fn peek_value(&self) -> Option<String> {
        if self.peek().is_none() {
            return None;
        }

        Some(self.peek().unwrap().value.clone())
    }

    pub fn select(&self, index: usize) -> Option<&Token> {
        self.tokens.get(index)
    }

    pub fn select_value(&self, index: usize) -> String {
        self.select(index).unwrap().value.clone()
    }

    pub fn next(&mut self) {
        self.current += 1;

        if self.current >= self.tokens.len() {
            self.eof = true;
        }
    }

    pub fn check_eof(&mut self) {
        if self.current >= self.tokens.len() {
            self.eof = true;
        }
    }

    pub fn append(&mut self, value: &str, mode: AppendMode) {
        match mode {
            AppendMode::Append => {
                self.result.append(value, false);
            },
            AppendMode::AppendToMainFunction => {
                self.result.append_to_main_function(value, false);
            },
            AppendMode::ForceAppend => {
                self.result.force_append(value, false);
            },
            AppendMode::FFAppend => {
                self.result.ff_append(value, false);
            },
            AppendMode::AppendWithSpace => {
                self.result.append(value, true);
            },
            AppendMode::AppendToMainFunctionWithSpace => {
                self.result.append_to_main_function(value, true);
            },
            AppendMode::ForceAppendWithSpace => {
                self.result.force_append(value, true);
            },
            AppendMode::FFAppendWithSpace => {
                self.result.ff_append(value, true);
            },
        }
    }

    pub fn value(&self) -> String {
        self.current().unwrap().value.clone()
    }

    pub fn kind(&self) -> TokenKind {
        self.current().unwrap().kind
    }

    pub fn parse_mut(&mut self) -> Consumed {
        let mut consumed = 0;
        
        if self.value() == "mut" && self.peek_kind() == Some(TokenKind::Identifier) {
            let next = self.select(self.current + 3).unwrap();

            if next.value != "=" {
                consumed += 2;                
                self.append(&format!("let mut {}", self.peek_value().unwrap()), AppendMode::AppendWithSpace);
            }
        }

        Consumed::consume(consumed)
    }

    pub fn parse_var(&mut self) -> Consumed {
        let mut consumed = 0;

        if self.kind() == TokenKind::Identifier && self.peek_value() == Some("=".to_string()) {
            let var_value = self.select(self.current + 2);

            if var_value.is_none() {
                // Empty variable
                return Consumed::consume(0);
            }

            let var_value = var_value.unwrap().value.clone();

            if var_value != "=" {
                consumed += 2;
                self.append(&format!("let {} = ", self.value()), AppendMode::AppendWithSpace);
            }

            return Consumed::Consumed(consumed);
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

    pub fn parse_class_definition(&mut self) -> Consumed {
        let mut consumed = 0;

        if self.value() == "class" && self.kind() == TokenKind::Keyword {
            // Iniciar coleta de tokens da classe
            let mut class_tokens = Vec::new();
            consumed += 1; // Consumir a palavra-chave "class"
            self.next();

            // Coletar tokens até o fechamento da classe
            let mut brace_count = 1; // Já encontramos o primeiro '{'
            while !self.eof && brace_count > 0 {
                if let Some(token) = self.current() {
                    if token.kind == TokenKind::BraceStart {
                        brace_count += 1;
                    } else if token.kind == TokenKind::BraceEnd {
                        brace_count -= 1;
                    }
                    class_tokens.push(token.value.clone());
                    consumed += 1;
                    self.next();
                } else {
                    break;
                }
            }

            // Converter tokens para uma string de definição de classe
            let class_str = class_tokens[0..class_tokens.len() - 1].join(" ");

            // Analisar a classe utilizando o `parse_class`
            let class_str = "class ".to_owned() + &class_str;
            println!("Class definition: {}", class_str);
            let parsed_class = class::parse_class(&class_str);

            // Incorporar o resultado ao `result`
            self.result.append(&parsed_class.to_string(), true);

            self.is_inside_class = false;
            return Consumed::Consumed(consumed);
        }

        Consumed::consume(consumed)
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
                
                if self.current_import_vars.len() > 0 {
                    if self.is_import_list {
                        self.append(&"::".to_string(), AppendMode::Append);
                        self.append(&"{".to_string(), AppendMode::Append);
                        self.append(&self.current_import_vars.clone().join(", "), AppendMode::Append);
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

    pub fn parse_operator(&mut self) -> Consumed {
        let mut consumed = 0;

        for (copper, rust) in COPPER_OPERATORS.iter() {
            if self.peek().is_some() {
                let next_token = self.peek().unwrap();

                if (next_token.value.to_owned() + &self.value()) == *copper {
                    self.append(&rust, AppendMode::Append);
                    consumed += 2;
                    break;
                }
            }
        }

        Consumed::consume(consumed)
    }

    pub fn required_import_var(&mut self, var: &str) -> String {
        return "__".to_owned() + var + "__";
    }

    pub fn parse_regex(&mut self) -> Consumed {
        let mut consumed = 0;

        if self.kind() == TokenKind::Regex {
            if !self.result.has_required_import("regex") {
                self.result.add_required_import("regex");
            }

            let value = self.value();
            let var = self.required_import_var("regex");
            let resolved_regex = value.trim_start_matches('/').trim_end_matches('/');

            self.append(&format!("{}::Regex::new({})", var, &format!("r\"{}\"", resolved_regex)), AppendMode::Append);
            consumed += 1;
        }

        Consumed::consume(consumed)
    }

    pub fn parse_class(&mut self) -> Consumed {
        if self.value() == "class" {
            self.is_inside_class = true;
            return self.parse_class_definition()
        }

        Consumed::consume(0)
    }
    
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
                    TokenKind::Identifier |
                    TokenKind::Keyword => {
                        self.parse_mut()
                            .or(|| self.parse_var())
                            .or(|| self.parse_class())
                            .or(|| self.parse_function())
                            .or(|| self.parse_any())
                            .consume_var(&mut self.current);
                    },
                    TokenKind::ParametersEnd |
                    TokenKind::ParametersStart => {
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
                    TokenKind::BraceStart |
                    TokenKind::BraceEnd => {
                        self.parse_import()
                            .or(|| self.parse_function_body())
                            .or(|| self.parse_any())
                            .consume_var(&mut self.current);
                    },
                    TokenKind::From |
                    TokenKind::ModuleVar |
                    TokenKind::ModulePath |
                    TokenKind::Import => {
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