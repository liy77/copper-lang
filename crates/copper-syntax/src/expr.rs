//! Typed expression / type AST for the Copper **subset** + a recursive Pratt
//! parser that builds it from the tokenizer's token stream.
//!
//! Why this exists
//! ---------------
//! `ast.rs` is a declaration-level, span-based tree: it knows where every
//! `func` / `struct` / `let` is, but bodies are captured as opaque `Span`s and
//! never turned into expression trees. That is enough for the LSP's document
//! symbols, but **not** enough to (a) embed Copper expressions in MUI markup or
//! (b) lower them to a JIT IR (Cranelift). Both need a real `Expr` tree with a
//! slot for a resolved type.
//!
//! Scope (subset, on purpose)
//! --------------------------
//! Literals, identifiers, member access, calls, indexing, unary/binary ops,
//! ternary, assignment, ranges, array literals, closures, struct literals, and
//! `if` / `match` as expressions. This is the surface MUI bindings + event
//! handlers use. It is intentionally NOT all of Copper — it grows as the JIT
//! does. Anything the subset can't form becomes [`ExprKind::Raw`] so a fallback
//! (the full Copper parser) can take over later.
//!
//! Every [`Expr`] carries a [`Span`] and a `ty: Option<Type>` slot. The parser
//! leaves `ty` as `None`; a later inference pass fills it in. That field is what
//! makes this a *typed* AST foundation rather than a bare CST.

use crate::ast::Span;
use crate::tokenizer::kind::TokenKind;
use crate::tokenizer::tokenizer::Tokenizer;
use crate::tokenizer::tokens::{Data, Token};

// ===========================================================================
// Types
// ===========================================================================

/// A (not-yet-resolved) Copper type. `Unknown` is the parser's default; an
/// inference pass replaces it. Cranelift lowering keys off the resolved form.
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int,
    Float,
    Bool,
    Str,
    Unit,
    /// A named/user type, optionally with generic args: `Result<T, E>`.
    Named(String, Vec<Type>),
    Option(Box<Type>),
    Vec(Box<Type>),
    Fn(Vec<Type>, Box<Type>),
    /// Placeholder until inference runs.
    Unknown,
}

// ===========================================================================
// Operators
// ===========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    // Bitwise (the tokenizer's SYMBOL_OPERATORS `& | ^`).
    BitAnd,
    BitOr,
    BitXor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    /// `-x`
    Neg,
    /// `!x`
    Not,
    /// `&x` — shared reference.
    Ref,
    /// `&mut x` — mutable reference.
    RefMut,
    /// `*x` — dereference.
    Deref,
}

/// Compound-assignment flavour. `Plain` is `=`. Mirrors the tokenizer's
/// `COMPOUND_SIGNS` (minus `::`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Plain,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
}

// ===========================================================================
// Literals & string templates
// ===========================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Int(i64),
    Float(f64),
    Bool(bool),
    /// A string with interpolation: `"a ${x} b"` →
    /// `[Lit("a "), Expr(x), Lit(" b")]`.
    Str(StrTemplate),
}

#[derive(Debug, Clone, PartialEq)]
pub struct StrTemplate {
    pub parts: Vec<StrPart>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StrPart {
    Lit(String),
    Expr(Box<Expr>),
}

// ===========================================================================
// Expressions
// ===========================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    /// Resolved type — `None` until an inference pass runs. The presence of
    /// this slot is what makes the tree a *typed* AST.
    pub ty: Option<Type>,
}

impl Expr {
    /// Construct an expression node with no inferred type yet. Public so
    /// front-ends that embed Copper (e.g. `mui-syntax`) can build fallback
    /// nodes without re-running the parser.
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self {
            kind,
            span,
            ty: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Literal(Literal),
    /// A bare name: `count`, `FontStyle`.
    Ident(String),
    /// `obj.field` (member) — `optional` is true for `obj?.field`.
    Member {
        base: Box<Expr>,
        field: String,
        optional: bool,
    },
    /// `callee(args...)`. `turbofish` carries the raw `::<...>` type-arg source
    /// when present (e.g. `parse::<i32>()`), kept as text until type lowering.
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        turbofish: Option<String>,
    },
    /// `a::b::c` path (module path, associated item, enum variant).
    Path {
        segments: Vec<String>,
    },
    /// `base[index]`.
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    /// `expr as Type` cast.
    Cast {
        expr: Box<Expr>,
        ty: Type,
    },
    /// Postfix try: `expr?`.
    Try {
        expr: Box<Expr>,
    },
    Unary {
        op: UnOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// `cond ? then : els`.
    Ternary {
        cond: Box<Expr>,
        then: Box<Expr>,
        els: Box<Expr>,
    },
    /// `target op= value` (and plain `=`).
    Assign {
        target: Box<Expr>,
        op: AssignOp,
        value: Box<Expr>,
    },
    /// `start..end` (`inclusive` → `..=`).
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
        inclusive: bool,
    },
    /// `[a, b, c]`.
    Array(Vec<Expr>),
    /// `|a, b| body` — body is a single expression (block expressions count).
    Closure {
        params: Vec<String>,
        body: Box<Expr>,
    },
    /// `Name { field: value, ..spread }`.
    StructLit {
        name: String,
        fields: Vec<(String, Expr)>,
        spread: Option<Box<Expr>>,
    },
    /// `if c { a } else { b }` used as a value.
    If {
        cond: Box<Expr>,
        then: Box<Expr>,
        els: Option<Box<Expr>>,
    },
    /// `match scrutinee { pat => expr, ... }`.
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    /// `{ stmts; tail }` — a block used as an expression.
    Block(Block),
    /// Recovery node for input the subset parser couldn't handle.
    Raw(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
    pub span: Span,
}

// ===========================================================================
// Patterns (match / let)
// ===========================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// `_`
    Wildcard,
    /// A binding or a nullary variant: `x`, `none`.
    Ident(String),
    Literal(Literal),
    /// `Some(p)`, `Todo { .. }` — a named variant with sub-patterns.
    TupleStruct {
        name: String,
        elems: Vec<Pattern>,
    },
    /// `a | b`
    Or(Vec<Pattern>),
}

// ===========================================================================
// Statements & blocks
// ===========================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// Copper binding: `mut name = value` (mutable) or bare `name = value`
    /// (immutable). `ty` carries an optional `: Type` annotation. There is no
    /// `let` keyword in Copper.
    Let {
        name: String,
        mutable: bool,
        ty: Option<Type>,
        value: Option<Expr>,
        span: Span,
    },
    /// `return expr?`.
    Return { value: Option<Expr>, span: Span },
    /// `yield expr?`.
    Yield { value: Option<Expr>, span: Span },
    /// `break`.
    Break { span: Span },
    /// `continue`.
    Continue { span: Span },
    /// `x++` / `x--` (lowered by the transpiler to `x += 1` / `x -= 1`).
    IncDec {
        target: Expr,
        /// `true` for `++`, `false` for `--`.
        inc: bool,
        span: Span,
    },
    /// `if cond { .. } else if .. { .. } else { .. }` in statement position.
    /// Also models `if let PAT = expr { .. }` via `let_pattern`.
    If {
        cond: Expr,
        /// `Some(pat)` for `if let pat = cond`.
        let_pattern: Option<Pattern>,
        then: Block,
        els: Option<Box<Stmt>>,
        span: Span,
    },
    /// `while cond { .. }` and `while let PAT = expr { .. }`.
    While {
        cond: Expr,
        let_pattern: Option<Pattern>,
        body: Block,
        span: Span,
    },
    /// `loop { .. }`.
    Loop { body: Block, span: Span },
    /// `for pat in iter { .. }`.
    For {
        pattern: Pattern,
        iter: Expr,
        body: Block,
        span: Span,
    },
    /// `unsafe { .. }` block in statement position.
    Unsafe { body: Block, span: Span },
    /// A bare block `{ .. }`.
    BlockStmt(Block),
    /// An expression in statement position.
    Expr(Expr),
}

impl Stmt {
    /// The source span of this statement.
    pub fn span(&self) -> Span {
        match self {
            Stmt::Let { span, .. }
            | Stmt::Return { span, .. }
            | Stmt::Yield { span, .. }
            | Stmt::Break { span }
            | Stmt::Continue { span }
            | Stmt::IncDec { span, .. }
            | Stmt::If { span, .. }
            | Stmt::While { span, .. }
            | Stmt::Loop { span, .. }
            | Stmt::For { span, .. }
            | Stmt::Unsafe { span, .. } => *span,
            Stmt::BlockStmt(b) => b.span,
            Stmt::Expr(e) => e.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    /// Trailing value expression (block's value), if any.
    pub tail: Option<Box<Expr>>,
    pub span: Span,
}

// ===========================================================================
// Public entry points
// ===========================================================================

#[derive(Debug, Clone)]
pub struct ParseError {
    pub span: Span,
    pub message: String,
}

/// Parse a single expression. Returns the expression (when one could be
/// formed) plus any errors. Never panics.
pub fn parse_expr(source: &str) -> (Option<Expr>, Vec<ParseError>) {
    let mut p = Parser::from_source(source);
    let e = p.parse_expr_bp(0);
    (e, p.errors)
}

/// Parse a statement block body (`stmt; stmt; tail`) without surrounding
/// braces. Useful for handler bodies and view-function bodies.
pub fn parse_stmts(source: &str) -> (Block, Vec<ParseError>) {
    let mut p = Parser::from_source(source);
    let to = p.toks.len();
    let block = p.parse_block_inner(0, to);
    (block, p.errors)
}

/// Parse a statement block from an already-tokenized slice. Used by the
/// program-level parser ([`crate::program`]) so item bodies are parsed from the
/// real tokens — no source-text reconstruction, which would mangle adjacency
/// (e.g. `++` / string interpolation). The slice may include trivia; it is
/// filtered the same way [`parse_stmts`] filters lexer output.
pub fn parse_stmts_tokens(tokens: &[Token]) -> (Block, Vec<ParseError>) {
    let mut p = Parser::from_tokens(tokens);
    let to = p.toks.len();
    let block = p.parse_block_inner(0, to);
    (block, p.errors)
}

/// Resolve a type spelling (e.g. `int`, `Vec<i32>`, `string?`) into a [`Type`],
/// honouring the shared Copper alias table. Public so the program-level parser
/// ([`crate::program`]) resolves return types / field types the same way.
pub fn type_from_source(s: &str) -> Type {
    type_from_str(s)
}

// ===========================================================================
// Parser
// ===========================================================================

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    errors: Vec<ParseError>,
    /// While true, an `Ident {` is NOT a struct literal. Set when parsing a
    /// control-flow head (`if`/`match`/`for` condition/scrutinee), where the
    /// `{` opens the block body, not a struct. Mirrors Rust's restriction.
    no_struct: bool,
}

impl Parser {
    fn from_source(source: &str) -> Self {
        let raw = Tokenizer::new(source.to_string()).tokenize();
        Self::from_tokens(&raw)
    }

    /// Build a parser from an existing token list, filtering trivia. Shared by
    /// [`from_source`](Self::from_source) and [`parse_stmts_tokens`].
    fn from_tokens(raw: &[Token]) -> Self {
        let toks: Vec<Token> = raw
            .iter()
            .filter(|t| {
                let trivia = matches!(
                    t.kind,
                    TokenKind::Whitespace
                        | TokenKind::Newline
                        | TokenKind::Comment
                        | TokenKind::Eof
                );
                // Also drop any stray whitespace-only token regardless of kind.
                !(trivia || t.value.trim().is_empty())
            })
            .cloned()
            .collect();
        Self {
            toks,
            pos: 0,
            errors: Vec::new(),
            no_struct: false,
        }
    }

    // --- cursor helpers ---------------------------------------------------
    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }
    fn peek_val(&self) -> Option<&str> {
        self.toks.get(self.pos).map(|t| t.value.as_str())
    }
    fn bump(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }
    fn eat(&mut self, value: &str) -> bool {
        if self.peek_val() == Some(value) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    /// Eat a `=>`, which the tokenizer may surface either as one token or as
    /// `=` followed by `>`.
    fn eat_fat_arrow(&mut self) -> bool {
        if self.eat("=>") {
            return true;
        }
        if self.peek_val() == Some("=")
            && self.toks.get(self.pos + 1).map(|t| t.value.as_str()) == Some(">")
        {
            self.pos += 2;
            return true;
        }
        false
    }
    fn span_at(&self, idx: usize) -> Span {
        self.toks.get(idx).map(span_of).unwrap_or_default()
    }
    fn cur_span(&self) -> Span {
        self.toks
            .get(self.pos)
            .map(span_of)
            .or_else(|| self.toks.last().map(span_of))
            .unwrap_or_default()
    }
    fn err(&mut self, span: Span, message: impl Into<String>) {
        self.errors.push(ParseError {
            span,
            message: message.into(),
        });
    }

    // --- Pratt expression parser -----------------------------------------
    fn parse_expr_bp(&mut self, min_bp: u8) -> Option<Expr> {
        let mut lhs = self.parse_prefix()?;

        loop {
            let Some(op) = self.peek_val().map(|s| s.to_string()) else {
                break;
            };
            let op = op.as_str();

            // Postfix: call, index, member, path/turbofish.
            match op {
                "(" => {
                    lhs = self.parse_call(lhs, None);
                    continue;
                }
                "[" => {
                    lhs = self.parse_index(lhs);
                    continue;
                }
                "." | "?." => {
                    let optional = op == "?.";
                    let start = lhs.span;
                    self.bump();
                    let field = self.expect_ident("field name after `.`");
                    let end = self.span_at(self.pos.saturating_sub(1));
                    lhs = Expr::new(
                        ExprKind::Member {
                            base: Box::new(lhs),
                            field,
                            optional,
                        },
                        Span::merge(start, end),
                    );
                    continue;
                }
                "::" => {
                    lhs = self.parse_path_or_turbofish(lhs);
                    continue;
                }
                "!" => {
                    // Macro invocation: `name!(...)` / `name![...]` / `name!{...}`
                    // only when the `!` is immediately followed by a bracket.
                    let after = self.toks.get(self.pos + 1).map(|t| t.value.as_str());
                    if matches!(after, Some("(") | Some("[") | Some("{")) {
                        self.bump(); // !
                        let opener = self.peek_val().unwrap_or("(").to_string();
                        if opener == "(" {
                            lhs = self.parse_call(lhs, None);
                        } else {
                            // `vec![...]` / `name!{...}`: keep args as one Array
                            // / Block-ish call; reuse the bracket/brace parsers.
                            let arg = if opener == "[" {
                                self.parse_array(self.cur_span())
                            } else {
                                self.parse_block_expr(self.cur_span())
                            };
                            let span = Span::merge(lhs.span, arg.span);
                            lhs = Expr::new(
                                ExprKind::Call {
                                    callee: Box::new(lhs),
                                    args: vec![arg],
                                    turbofish: None,
                                },
                                span,
                            );
                        }
                        continue;
                    }
                    break;
                }
                _ => {}
            }

            // `as` cast: `expr as Type`.
            if op == "as" {
                let start = lhs.span;
                self.bump();
                let ty = self.parse_type();
                let end = self.span_at(self.pos.saturating_sub(1));
                lhs = Expr::new(
                    ExprKind::Cast {
                        expr: Box::new(lhs),
                        ty,
                    },
                    Span::merge(start, end),
                );
                continue;
            }

            // `?` is overloaded: postfix try (`expr?`) vs. ternary
            // (`cond ? a : b`). The real Copper distinguishes them by whether a
            // matching `:` follows; we use the cheaper, equivalent rule: if the
            // token after `?` can start an expression it's a ternary, otherwise
            // it's the try operator. (`?.` was already handled above.)
            if op == "?" {
                let next = self.toks.get(self.pos + 1).map(|t| t.value.as_str());
                // `expr?` (try) vs `cond ? a : b` (ternário): é ternário só se
                // houver um `:` no mesmo nível antes do fim do statement.
                if !starts_expr(next) || !self.ternary_colon_ahead() {
                    // Postfix try.
                    let start = lhs.span;
                    self.bump();
                    let span = Span::merge(start, self.span_at(self.pos.saturating_sub(1)));
                    lhs = Expr::new(
                        ExprKind::Try {
                            expr: Box::new(lhs),
                        },
                        span,
                    );
                    continue;
                }
                if bp_ternary() < min_bp {
                    break;
                }
                let start = lhs.span;
                self.bump();
                let then = self.parse_expr_bp(0)?;
                if !self.eat(":") {
                    let sp = self.cur_span();
                    self.err(sp, "expected `:` in ternary");
                }
                let els = self.parse_expr_bp(bp_ternary())?;
                let span = Span::merge(start, els.span);
                lhs = Expr::new(
                    ExprKind::Ternary {
                        cond: Box::new(lhs),
                        then: Box::new(then),
                        els: Box::new(els),
                    },
                    span,
                );
                continue;
            }

            // `++` / `--`: the tokenizer emits these as two `+`/`-` tokens.
            // Copper always treats them as inc/dec (never `x + (+y)`), so stop
            // here and let the statement layer build the `IncDec` node.
            if (op == "+" || op == "-")
                && self.toks.get(self.pos + 1).map(|t| t.value.as_str()) == Some(op)
            {
                break;
            }

            // Range.
            if op == ".." || op == "..=" {
                let inclusive = op == "..=";
                let start = lhs.span;
                self.bump();
                let end = self.parse_expr_bp(bp_range() + 1)?;
                let span = Span::merge(start, end.span);
                lhs = Expr::new(
                    ExprKind::Range {
                        start: Box::new(lhs),
                        end: Box::new(end),
                        inclusive,
                    },
                    span,
                );
                continue;
            }

            // Assignment (right assoc). Guard against `=>`: the tokenizer emits
            // a fat arrow as `=` `>`, so a bare `=` followed by `>` is a match
            // arm separator, not an assignment — stop before it.
            if op == "=" && self.toks.get(self.pos + 1).map(|t| t.value.as_str()) == Some(">") {
                break;
            }
            if let Some(aop) = assign_op(op) {
                if bp_assign() < min_bp {
                    break;
                }
                let start = lhs.span;
                self.bump();
                let value = self.parse_expr_bp(bp_assign())?;
                let span = Span::merge(start, value.span);
                lhs = Expr::new(
                    ExprKind::Assign {
                        target: Box::new(lhs),
                        op: aop,
                        value: Box::new(value),
                    },
                    span,
                );
                continue;
            }

            // `&&` / `||` can arrive as two single-char tokens; coalesce them.
            if op == "&" || op == "|" {
                let twin = self.toks.get(self.pos + 1).map(|t| t.value.as_str());
                if (op == "&" && twin == Some("&")) || (op == "|" && twin == Some("|")) {
                    let (bop, lbp, rbp) = if op == "&" {
                        (BinOp::And, 10u8, 11u8)
                    } else {
                        (BinOp::Or, 8u8, 9u8)
                    };
                    if lbp < min_bp {
                        break;
                    }
                    let start = lhs.span;
                    self.bump();
                    self.bump();
                    let rhs = self.parse_expr_bp(rbp)?;
                    let span = Span::merge(start, rhs.span);
                    lhs = Expr::new(
                        ExprKind::Binary {
                            op: bop,
                            lhs: Box::new(lhs),
                            rhs: Box::new(rhs),
                        },
                        span,
                    );
                    continue;
                }
            }

            // Binary operators.
            let Some((bop, lbp, rbp)) = bin_op(op) else {
                break;
            };
            if lbp < min_bp {
                break;
            }
            let start = lhs.span;
            self.bump();
            let rhs = self.parse_expr_bp(rbp)?;
            let span = Span::merge(start, rhs.span);
            lhs = Expr::new(
                ExprKind::Binary {
                    op: bop,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            );
        }

        Some(lhs)
    }

    fn parse_prefix(&mut self) -> Option<Expr> {
        let t = self.peek()?.clone();
        let span = span_of(&t);
        match t.kind {
            TokenKind::Number => {
                self.bump();
                Some(Expr::new(ExprKind::Literal(number_literal(&t.value)), span))
            }
            TokenKind::String | TokenKind::InterpolatedString => {
                self.bump();
                Some(Expr::new(
                    ExprKind::Literal(Literal::Str(str_template(&t))),
                    span,
                ))
            }
            // `Param` / `ParamType`: the tokenizer tags identifiers that appear
            // inside a `|...|` param list or after a `:` this way. In
            // expression position they are ordinary names (e.g. the closure
            // body `x` in `|x| x * 2`).
            TokenKind::Identifier
            | TokenKind::Keyword
            | TokenKind::Type
            | TokenKind::Param
            | TokenKind::ParamType => {
                match t.value.as_str() {
                    "true" => {
                        self.bump();
                        Some(Expr::new(ExprKind::Literal(Literal::Bool(true)), span))
                    }
                    "false" => {
                        self.bump();
                        Some(Expr::new(ExprKind::Literal(Literal::Bool(false)), span))
                    }
                    "if" => self.parse_if_expr(),
                    "match" => self.parse_match_expr(),
                    _ => {
                        self.bump();
                        let name = t.value.clone();
                        // Struct literal: `Name { field: val, ... }` — only when
                        // an identifier is immediately followed by `{` AND we are
                        // not in a control-flow head (where `{` opens a block).
                        if self.peek_val() == Some("{") && !self.no_struct {
                            return Some(self.parse_struct_lit(name, span));
                        }
                        Some(Expr::new(ExprKind::Ident(name), span))
                    }
                }
            }
            _ => match t.value.as_str() {
                "(" => {
                    self.bump();
                    let inner = self.parse_expr_bp(0)?;
                    if !self.eat(")") {
                        let sp = self.cur_span();
                        self.err(sp, "expected `)`");
                    }
                    Some(inner)
                }
                "[" => Some(self.parse_array(span)),
                "{" => Some(self.parse_block_expr(span)),
                "|" => Some(self.parse_closure(span)),
                "-" => {
                    self.bump();
                    let e = self.parse_expr_bp(bp_unary())?;
                    let s = Span::merge(span, e.span);
                    Some(Expr::new(
                        ExprKind::Unary {
                            op: UnOp::Neg,
                            expr: Box::new(e),
                        },
                        s,
                    ))
                }
                "!" => {
                    self.bump();
                    let e = self.parse_expr_bp(bp_unary())?;
                    let s = Span::merge(span, e.span);
                    Some(Expr::new(
                        ExprKind::Unary {
                            op: UnOp::Not,
                            expr: Box::new(e),
                        },
                        s,
                    ))
                }
                "*" => {
                    self.bump();
                    let e = self.parse_expr_bp(bp_unary())?;
                    let s = Span::merge(span, e.span);
                    Some(Expr::new(
                        ExprKind::Unary {
                            op: UnOp::Deref,
                            expr: Box::new(e),
                        },
                        s,
                    ))
                }
                "&" => {
                    self.bump();
                    // `&mut x` vs `&x`.
                    let op = if self.peek_val() == Some("mut") {
                        self.bump();
                        UnOp::RefMut
                    } else {
                        UnOp::Ref
                    };
                    let e = self.parse_expr_bp(bp_unary())?;
                    let s = Span::merge(span, e.span);
                    Some(Expr::new(
                        ExprKind::Unary {
                            op,
                            expr: Box::new(e),
                        },
                        s,
                    ))
                }
                other => {
                    self.err(span, format!("unexpected token `{other}` in expression"));
                    self.bump();
                    None
                }
            },
        }
    }

    fn parse_call(&mut self, callee: Expr, turbofish: Option<String>) -> Expr {
        let start = callee.span;
        self.bump(); // (
        let mut args = Vec::new();
        while let Some(v) = self.peek_val() {
            if v == ")" {
                break;
            }
            if let Some(arg) = self.parse_expr_bp(0) {
                args.push(arg);
            } else {
                break;
            }
            if !self.eat(",") {
                break;
            }
        }
        let end = self.cur_span();
        self.eat(")");
        Expr::new(
            ExprKind::Call {
                callee: Box::new(callee),
                args,
                turbofish,
            },
            Span::merge(start, end),
        )
    }

    /// Handle `::` after a primary. Two cases:
    ///   * turbofish — `name::<T, U>(...)` → consume `<...>` and the call,
    ///     attaching the type-arg source to the [`ExprKind::Call`].
    ///   * path — `a::b::c` → collect segments into [`ExprKind::Path`].
    fn parse_path_or_turbofish(&mut self, lhs: Expr) -> Expr {
        let start = lhs.span;
        self.bump(); // ::
        if self.peek_val() == Some("<") {
            // Turbofish: collect `<...>` as raw text (balanced), then a call.
            let mut depth = 0i32;
            let mut buf = String::new();
            while let Some(t) = self.peek() {
                let v = t.value.clone();
                if v == "<" {
                    depth += 1;
                } else if v == ">" {
                    depth -= 1;
                }
                buf.push_str(&v);
                self.bump();
                if depth <= 0 {
                    break;
                }
            }
            if self.peek_val() == Some("(") {
                return self.parse_call(lhs, Some(buf));
            }
            // `name::<T>` with no call — keep as a path-ish node carrying it.
            let end = self.span_at(self.pos.saturating_sub(1));
            return Expr::new(
                ExprKind::Call {
                    callee: Box::new(lhs),
                    args: Vec::new(),
                    turbofish: Some(buf),
                },
                Span::merge(start, end),
            );
        }
        // Plain path: seed segments from the lhs if it is a bare ident/path.
        let mut segments = match lhs.kind {
            ExprKind::Ident(name) => vec![name],
            ExprKind::Path { segments } => segments,
            _ => Vec::new(),
        };
        segments.push(self.expect_ident("path segment after `::`"));
        // Continue chaining further `::seg` segments.
        while self.peek_val() == Some("::")
            && self
                .toks
                .get(self.pos + 1)
                .map(|t| t.value.as_str() != "<")
                .unwrap_or(false)
        {
            self.bump();
            segments.push(self.expect_ident("path segment after `::`"));
        }
        let end = self.span_at(self.pos.saturating_sub(1));
        Expr::new(ExprKind::Path { segments }, Span::merge(start, end))
    }

    /// Parse a type reference (used by `as` casts). Collects type tokens up to a
    /// natural boundary and resolves Copper aliases via the shared lexicon, so
    /// `as int` yields [`Type::Int`] exactly as the transpiler would.
    fn parse_type(&mut self) -> Type {
        let mut buf = String::new();
        let mut depth = 0i32;
        while let Some(t) = self.peek() {
            let v = t.value.as_str();
            if depth == 0 && matches!(v, ")" | "]" | "}" | "," | ";" | "=" | "{") {
                break;
            }
            match v {
                "<" | "(" | "[" => depth += 1,
                ">" | ")" | "]" => depth -= 1,
                _ => {}
            }
            buf.push_str(v);
            self.bump();
            // A bare identifier/primitive with nothing generic following ends
            // the type (so `x as i32 + 1` stops after `i32`).
            if depth == 0 {
                let nxt = self.peek_val();
                if !matches!(nxt, Some("<") | Some("::") | Some("?")) {
                    break;
                }
            }
        }
        type_from_str(buf.trim())
    }

    fn parse_index(&mut self, base: Expr) -> Expr {
        let start = base.span;
        self.bump(); // [
        let index = self
            .parse_expr_bp(0)
            .unwrap_or_else(|| Expr::new(ExprKind::Raw(String::new()), self.cur_span()));
        let end = self.cur_span();
        self.eat("]");
        Expr::new(
            ExprKind::Index {
                base: Box::new(base),
                index: Box::new(index),
            },
            Span::merge(start, end),
        )
    }

    fn parse_array(&mut self, open: Span) -> Expr {
        self.bump(); // [
        let mut elems = Vec::new();
        while let Some(v) = self.peek_val() {
            if v == "]" {
                break;
            }
            if let Some(e) = self.parse_expr_bp(0) {
                elems.push(e);
            } else {
                break;
            }
            if !self.eat(",") {
                break;
            }
        }
        let end = self.cur_span();
        self.eat("]");
        Expr::new(ExprKind::Array(elems), Span::merge(open, end))
    }

    /// A partir do `?` na posição atual, há um `:` de ternário no mesmo nível
    /// de parênteses antes do fim do statement? (Distingue `expr?` de
    /// `cond ? a : b`.) Pula `::` (paths/turbofish).
    fn ternary_colon_ahead(&self) -> bool {
        let mut depth = 0i32;
        let mut i = self.pos + 1;
        while let Some(t) = self.toks.get(i) {
            if matches!(t.kind, TokenKind::Newline | TokenKind::Eof) {
                return false;
            }
            match t.value.as_str() {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                ";" | "," if depth == 0 => return false,
                "::" => {}
                ":" if depth == 0 => {
                    // Evita confundir com `::` quebrado em dois tokens.
                    if self.toks.get(i + 1).map(|t| t.value.as_str()) == Some(":")
                        || self.toks.get(i - 1).map(|t| t.value.as_str()) == Some(":")
                    {
                        // parte de `::`
                    } else {
                        return true;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn parse_closure(&mut self, open: Span) -> Expr {
        self.bump(); // |
        let mut params = Vec::new();
        while let Some(v) = self.peek_val() {
            if v == "|" {
                break;
            }
            // Pula marcadores de referência/mut antes do nome do binding:
            // `|&&x|`, `|&x|`, `|mut x|`, `|*p|`. Eles não vêm seguidos de
            // vírgula, então precisam ser consumidos dentro do mesmo parâmetro.
            while matches!(
                self.peek_val(),
                Some("&") | Some("&&") | Some("mut") | Some("*")
            ) {
                self.bump();
            }
            if matches!(
                self.peek().map(|t| &t.kind),
                Some(TokenKind::Identifier) | Some(TokenKind::Param)
            ) {
                if let Some(tok) = self.bump() {
                    params.push(tok.value);
                }
            } else if self.peek_val() == Some("|") {
                break;
            } else {
                self.bump();
            }
            if !self.eat(",") {
                break;
            }
        }
        self.eat("|");
        let body = self
            .parse_expr_bp(0)
            .unwrap_or_else(|| Expr::new(ExprKind::Raw(String::new()), self.cur_span()));
        let span = Span::merge(open, body.span);
        Expr::new(
            ExprKind::Closure {
                params,
                body: Box::new(body),
            },
            span,
        )
    }

    fn parse_struct_lit(&mut self, name: String, start: Span) -> Expr {
        self.bump(); // {
        let mut fields = Vec::new();
        let mut spread = None;
        while let Some(v) = self.peek_val() {
            if v == "}" {
                break;
            }
            if v == ".." {
                self.bump();
                spread = self.parse_expr_bp(0).map(Box::new);
                self.eat(",");
                continue;
            }
            let field = self.expect_ident("struct field name");
            let value = if self.eat(":") {
                self.parse_expr_bp(0)
                    .unwrap_or_else(|| Expr::new(ExprKind::Ident(field.clone()), start))
            } else {
                // Shorthand `Name { field }`.
                Expr::new(ExprKind::Ident(field.clone()), start)
            };
            fields.push((field, value));
            if !self.eat(",") {
                break;
            }
        }
        let end = self.cur_span();
        self.eat("}");
        Expr::new(
            ExprKind::StructLit {
                name,
                fields,
                spread,
            },
            Span::merge(start, end),
        )
    }

    fn parse_if_expr(&mut self) -> Option<Expr> {
        let start = self.cur_span();
        self.bump(); // if
        let saved = self.no_struct;
        self.no_struct = true;
        let cond = self.parse_expr_bp(0);
        self.no_struct = saved;
        let cond = cond?;
        let then = self.parse_braced_block()?;
        let els = if self.eat("else") {
            if self.peek_val() == Some("if") {
                self.parse_if_expr().map(Box::new)
            } else {
                self.parse_braced_block().map(Box::new)
            }
        } else {
            None
        };
        let end = els.as_ref().map(|e| e.span).unwrap_or(then.span);
        Some(Expr::new(
            ExprKind::If {
                cond: Box::new(cond),
                then: Box::new(then),
                els,
            },
            Span::merge(start, end),
        ))
    }

    fn parse_match_expr(&mut self) -> Option<Expr> {
        let start = self.cur_span();
        self.bump(); // match
        let saved = self.no_struct;
        self.no_struct = true;
        let scrutinee = self.parse_expr_bp(0);
        self.no_struct = saved;
        let scrutinee = scrutinee?;
        if !self.eat("{") {
            let sp = self.cur_span();
            self.err(sp, "expected `{` after match scrutinee");
        }
        let mut arms = Vec::new();
        while let Some(v) = self.peek_val() {
            if v == "}" {
                break;
            }
            let arm_start = self.cur_span();
            let pattern = self.parse_pattern();
            let guard = if self.eat("if") {
                self.parse_expr_bp(0)
            } else {
                None
            };
            if !self.eat_fat_arrow() {
                let sp = self.cur_span();
                self.err(sp, "expected `=>` in match arm");
            }
            let body = self
                .parse_expr_bp(0)
                .unwrap_or_else(|| Expr::new(ExprKind::Raw(String::new()), self.cur_span()));
            let span = Span::merge(arm_start, body.span);
            arms.push(MatchArm {
                pattern,
                guard,
                body,
                span,
            });
            // Trailing comma optional.
            self.eat(",");
        }
        let end = self.cur_span();
        self.eat("}");
        Some(Expr::new(
            ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
            Span::merge(start, end),
        ))
    }

    fn parse_pattern(&mut self) -> Pattern {
        let mut first = self.parse_pattern_atom();
        if self.peek_val() == Some("|") {
            let mut alts = vec![first];
            while self.eat("|") {
                alts.push(self.parse_pattern_atom());
            }
            first = Pattern::Or(alts);
        }
        first
    }

    fn parse_pattern_atom(&mut self) -> Pattern {
        let Some(t) = self.peek().cloned() else {
            return Pattern::Wildcard;
        };
        match t.kind {
            TokenKind::Number | TokenKind::String | TokenKind::InterpolatedString => {
                if let Some(e) = self.parse_prefix() {
                    if let ExprKind::Literal(l) = e.kind {
                        return Pattern::Literal(l);
                    }
                }
                Pattern::Wildcard
            }
            _ => {
                let name = t.value.clone();
                if name == "_" {
                    self.bump();
                    return Pattern::Wildcard;
                }
                self.bump();
                if self.peek_val() == Some("(") {
                    self.bump();
                    let mut elems = Vec::new();
                    while let Some(v) = self.peek_val() {
                        if v == ")" {
                            break;
                        }
                        elems.push(self.parse_pattern());
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.eat(")");
                    Pattern::TupleStruct { name, elems }
                } else if self.peek_val() == Some("{") {
                    // `Name { .. }` — skip the fields for now, keep the name.
                    let mut depth = 0i32;
                    while let Some(tk) = self.peek() {
                        match tk.value.as_str() {
                            "{" => depth += 1,
                            "}" => {
                                depth -= 1;
                                if depth == 0 {
                                    self.bump();
                                    break;
                                }
                            }
                            _ => {}
                        }
                        self.bump();
                    }
                    Pattern::TupleStruct {
                        name,
                        elems: Vec::new(),
                    }
                } else {
                    Pattern::Ident(name)
                }
            }
        }
    }

    fn parse_braced_block(&mut self) -> Option<Expr> {
        let open = self.cur_span();
        if !self.eat("{") {
            self.err(open, "expected `{`");
            return None;
        }
        Some(self.parse_block_expr_from(open))
    }

    fn parse_block_expr(&mut self, open: Span) -> Expr {
        self.bump(); // {
        self.parse_block_expr_from(open)
    }

    /// Parse statements until the matching `}` (the `{` was already consumed).
    fn parse_block_expr_from(&mut self, open: Span) -> Expr {
        let end_idx = self.matching_brace(self.pos);
        let block = self.parse_block_inner(self.pos, end_idx);
        self.pos = end_idx;
        self.eat("}");
        let span = Span::merge(open, block.span);
        Expr::new(ExprKind::Block(block), span)
    }

    /// Parse the `[from, to)` token range as statements + an optional trailing
    /// value expression.
    fn parse_block_inner(&mut self, from: usize, to: usize) -> Block {
        self.pos = from;
        let start_span = self.span_at(from);
        let mut stmts = Vec::new();
        let mut tail = None;

        while self.pos < to {
            if matches!(self.peek_val(), Some(";") | Some(",")) {
                self.bump();
                continue;
            }
            let before = self.pos;
            // A Copper binding is `mut name = expr` / `name = expr` /
            // `name: Type = expr` (NO `let`). A leading `mut`, or an identifier
            // whose next token is `=` or `:`, is a binding; anything else
            // (including `obj.field = x`) is an expression statement.
            let next_is = self.toks.get(self.pos + 1).map(|t| t.value.as_str());
            let is_binding = self.peek_val() == Some("mut")
                || (self.at_ident() && matches!(next_is, Some("=") | Some(":")));
            match self.peek_val() {
                _ if is_binding => {
                    if let Some(s) = self.parse_bind_stmt() {
                        stmts.push(s);
                    }
                }
                Some("return") => {
                    let sp = self.cur_span();
                    self.bump();
                    let value = if self.at_stmt_end(to) {
                        None
                    } else {
                        self.parse_expr_bp(0)
                    };
                    stmts.push(Stmt::Return { value, span: sp });
                }
                Some("yield") => {
                    let sp = self.cur_span();
                    self.bump();
                    let value = if self.at_stmt_end(to) {
                        None
                    } else {
                        self.parse_expr_bp(0)
                    };
                    stmts.push(Stmt::Yield { value, span: sp });
                }
                Some("break") => {
                    let sp = self.cur_span();
                    self.bump();
                    stmts.push(Stmt::Break { span: sp });
                }
                Some("continue") => {
                    let sp = self.cur_span();
                    self.bump();
                    stmts.push(Stmt::Continue { span: sp });
                }
                Some("if") => stmts.push(self.parse_if_stmt()),
                Some("while") => stmts.push(self.parse_while_stmt()),
                Some("loop") => stmts.push(self.parse_loop_stmt()),
                Some("for") => stmts.push(self.parse_for_stmt()),
                Some("unsafe") if next_is == Some("{") => {
                    let sp = self.cur_span();
                    self.bump(); // unsafe
                    let body = self.parse_block_braced();
                    let span = Span::merge(sp, body.span);
                    stmts.push(Stmt::Unsafe { body, span });
                }
                Some("{") => {
                    let body = self.parse_block_braced();
                    stmts.push(Stmt::BlockStmt(body));
                }
                _ => {
                    let Some(e) = self.parse_expr_bp(0) else {
                        break;
                    };
                    // Postfix `++` / `--` (tokenized as two `+`/`-`).
                    if let Some(stmt) = self.try_incdec(&e) {
                        stmts.push(stmt);
                    } else {
                        let is_sep = matches!(self.peek_val(), Some(";") | Some(","));
                        if !is_sep && self.pos >= to {
                            tail = Some(Box::new(e));
                        } else {
                            stmts.push(Stmt::Expr(e));
                        }
                    }
                }
            }
            if self.pos == before {
                // No progress — avoid an infinite loop on malformed input.
                self.bump();
            }
        }

        let end_span = self.span_at(to.saturating_sub(1));
        Block {
            stmts,
            tail,
            span: Span::merge(start_span, end_span),
        }
    }

    /// Parse a Copper binding statement: `mut name = expr` (mutable) or a bare
    /// `name = expr` (immutable). Copper has no `let` keyword, so we only
    /// consume a leading `mut`, never the name blindly.
    fn parse_bind_stmt(&mut self) -> Option<Stmt> {
        let start = self.cur_span();
        let mutable = self.peek_val() == Some("mut");
        if mutable {
            self.bump(); // `mut`
        }
        let name = self.expect_ident("binding name");
        // Optional `: Type` annotation (`mut doubled: Vec<i32> = ...`).
        let ty = if self.eat(":") {
            Some(self.parse_type())
        } else {
            None
        };
        let value = if self.eat("=") {
            self.parse_expr_bp(0)
        } else {
            None
        };
        let end = value.as_ref().map(|v| v.span).unwrap_or(start);
        Some(Stmt::Let {
            name,
            mutable,
            ty,
            value,
            span: Span::merge(start, end),
        })
    }

    fn at_ident(&self) -> bool {
        matches!(
            self.peek().map(|t| t.kind),
            Some(TokenKind::Identifier) | Some(TokenKind::Param)
        )
    }

    /// True at a statement boundary: a `;`, end of the current block range, or
    /// out of tokens. Used by `return` / `yield` to detect a bare keyword.
    fn at_stmt_end(&self, to: usize) -> bool {
        self.pos >= to || matches!(self.peek_val(), Some(";") | None)
    }

    /// Parse a `{ ... }` block (the next token must be `{`) into a [`Block`].
    fn parse_block_braced(&mut self) -> Block {
        let open = self.cur_span();
        if self.peek_val() != Some("{") {
            return Block {
                stmts: Vec::new(),
                tail: None,
                span: open,
            };
        }
        // Reuse the block-expression machinery, then unwrap to the Block.
        self.bump(); // {
        let expr = self.parse_block_expr_from(open);
        if let ExprKind::Block(b) = expr.kind {
            b
        } else {
            Block {
                stmts: Vec::new(),
                tail: None,
                span: open,
            }
        }
    }

    /// If `e` is immediately followed by `++` / `--` (each two tokens), consume
    /// them and return the [`Stmt::IncDec`]; otherwise leave the cursor put.
    fn try_incdec(&mut self, e: &Expr) -> Option<Stmt> {
        let (a, b) = (
            self.peek_val(),
            self.toks.get(self.pos + 1).map(|t| t.value.as_str()),
        );
        let inc = match (a, b) {
            (Some("+"), Some("+")) => true,
            (Some("-"), Some("-")) => false,
            _ => return None,
        };
        let start = e.span;
        self.bump();
        self.bump();
        let span = Span::merge(start, self.span_at(self.pos.saturating_sub(1)));
        Some(Stmt::IncDec {
            target: e.clone(),
            inc,
            span,
        })
    }

    /// `if cond { .. }` / `if let PAT = cond { .. }` with `else` / `else if`.
    fn parse_if_stmt(&mut self) -> Stmt {
        let start = self.cur_span();
        self.bump(); // if
        let let_pattern = self.parse_opt_let_pattern();
        let saved = self.no_struct;
        self.no_struct = true;
        let cond = self
            .parse_expr_bp(0)
            .unwrap_or_else(|| Expr::new(ExprKind::Raw(String::new()), self.cur_span()));
        self.no_struct = saved;
        let then = self.parse_block_braced();
        let els = if self.eat("else") {
            if self.peek_val() == Some("if") {
                Some(Box::new(self.parse_if_stmt()))
            } else {
                let b = self.parse_block_braced();
                Some(Box::new(Stmt::BlockStmt(b)))
            }
        } else {
            None
        };
        let end = self.span_at(self.pos.saturating_sub(1));
        Stmt::If {
            cond,
            let_pattern,
            then,
            els,
            span: Span::merge(start, end),
        }
    }

    fn parse_while_stmt(&mut self) -> Stmt {
        let start = self.cur_span();
        self.bump(); // while
        let let_pattern = self.parse_opt_let_pattern();
        let saved = self.no_struct;
        self.no_struct = true;
        let cond = self
            .parse_expr_bp(0)
            .unwrap_or_else(|| Expr::new(ExprKind::Raw(String::new()), self.cur_span()));
        self.no_struct = saved;
        let body = self.parse_block_braced();
        let end = self.span_at(self.pos.saturating_sub(1));
        Stmt::While {
            cond,
            let_pattern,
            body,
            span: Span::merge(start, end),
        }
    }

    fn parse_loop_stmt(&mut self) -> Stmt {
        let start = self.cur_span();
        self.bump(); // loop
        let body = self.parse_block_braced();
        let end = self.span_at(self.pos.saturating_sub(1));
        Stmt::Loop {
            body,
            span: Span::merge(start, end),
        }
    }

    fn parse_for_stmt(&mut self) -> Stmt {
        let start = self.cur_span();
        self.bump(); // for
        let pattern = self.parse_pattern();
        self.eat("in");
        let saved = self.no_struct;
        self.no_struct = true;
        let iter = self
            .parse_expr_bp(0)
            .unwrap_or_else(|| Expr::new(ExprKind::Raw(String::new()), self.cur_span()));
        self.no_struct = saved;
        let body = self.parse_block_braced();
        let end = self.span_at(self.pos.saturating_sub(1));
        Stmt::For {
            pattern,
            iter,
            body,
            span: Span::merge(start, end),
        }
    }

    /// Consume a leading `let PAT =` if present (the `if let` / `while let`
    /// form), returning the pattern. Copper writes `let` only here, never for
    /// plain bindings.
    fn parse_opt_let_pattern(&mut self) -> Option<Pattern> {
        if self.peek_val() != Some("let") {
            return None;
        }
        self.bump(); // let
        let pat = self.parse_pattern();
        self.eat("=");
        Some(pat)
    }

    fn expect_ident(&mut self, what: &str) -> String {
        match self.peek().cloned() {
            Some(t)
                if matches!(
                    t.kind,
                    TokenKind::Identifier
                        | TokenKind::Keyword
                        | TokenKind::Type
                        // Dentro de `(...)` o tokenizer marca identificadores
                        // como `Param`/`ParamType`; um campo após `.` num
                        // argumento (`f(a.w)`) chega assim.
                        | TokenKind::Param
                        | TokenKind::ParamType
                ) =>
            {
                self.bump();
                t.value
            }
            _ => {
                let sp = self.cur_span();
                self.err(sp, format!("expected {what}"));
                String::new()
            }
        }
    }

    /// Index of the `}` matching the `{` whose body starts at `from`.
    fn matching_brace(&self, from: usize) -> usize {
        let mut depth = 1i32;
        let mut i = from;
        while i < self.toks.len() {
            match self.toks[i].value.as_str() {
                "{" => depth += 1,
                "}" => {
                    depth -= 1;
                    if depth == 0 {
                        return i;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        self.toks.len()
    }
}

// ===========================================================================
// Operator tables (binding powers)
// ===========================================================================

fn bp_assign() -> u8 {
    2
}
fn bp_ternary() -> u8 {
    4
}
fn bp_range() -> u8 {
    6
}
fn bp_unary() -> u8 {
    20
}

fn assign_op(op: &str) -> Option<AssignOp> {
    Some(match op {
        "=" => AssignOp::Plain,
        "+=" => AssignOp::Add,
        "-=" => AssignOp::Sub,
        "*=" => AssignOp::Mul,
        "/=" => AssignOp::Div,
        "%=" => AssignOp::Rem,
        "&=" => AssignOp::BitAnd,
        "|=" => AssignOp::BitOr,
        "^=" => AssignOp::BitXor,
        _ => return None,
    })
}

/// Returns `(op, left_bp, right_bp)`. Higher binds tighter; `left < right`
/// means left-associative. Precedence follows Rust's (which is what the
/// transpiled output is ultimately compiled against).
fn bin_op(op: &str) -> Option<(BinOp, u8, u8)> {
    Some(match op {
        "||" => (BinOp::Or, 8, 9),
        "&&" => (BinOp::And, 10, 11),
        "==" => (BinOp::Eq, 12, 13),
        "!=" => (BinOp::Ne, 12, 13),
        "<" => (BinOp::Lt, 14, 15),
        "<=" => (BinOp::Le, 14, 15),
        ">" => (BinOp::Gt, 14, 15),
        ">=" => (BinOp::Ge, 14, 15),
        // Bitwise: between comparison and arithmetic, matching Rust.
        "|" => (BinOp::BitOr, 15, 16),
        "^" => (BinOp::BitXor, 16, 17),
        "&" => (BinOp::BitAnd, 17, 18),
        "+" => (BinOp::Add, 18, 19),
        "-" => (BinOp::Sub, 18, 19),
        "*" => (BinOp::Mul, 20, 21),
        "/" => (BinOp::Div, 20, 21),
        "%" => (BinOp::Rem, 20, 21),
        _ => return None,
    })
}

/// True when `next` (the token immediately after a `?`) can begin an
/// expression — used to tell a ternary (`cond ? a : b`) from the postfix try
/// operator (`expr?`). Closers, separators, and infix operators cannot start
/// an expression, so `?` before them is a try.
fn starts_expr(next: Option<&str>) -> bool {
    match next {
        None => false,
        Some(v) => !matches!(
            v,
            ")" | "]"
                | "}"
                | ","
                | ";"
                | ":"
                | "."
                | "?"
                | "="
                | "=="
                | "!="
                | "<="
                | ">="
                | "+"
                | "-"
                | "*"
                | "/"
                | "%"
                | "&&"
                | "||"
                | "&"
                | "|"
                | "^"
                | ".."
                | "..="
                | "=>"
        ),
    }
}

/// Resolve a collected type string into a [`Type`], honouring the shared
/// Copper alias table (so `int`/`uint`/`string`/`void`/... map exactly as the
/// transpiler does) and a few structural forms.
fn type_from_str(s: &str) -> Type {
    let s = s.trim();
    if s.is_empty() {
        return Type::Unknown;
    }
    if let Some(inner) = s.strip_suffix('?') {
        return Type::Option(Box::new(type_from_str(inner)));
    }
    // Resolve Copper aliases via the single source of truth, then classify the
    // resulting Rust spelling.
    let rust = crate::lexicon::convert_type(s);
    match rust.as_str() {
        "i64" | "i8" | "i16" | "i32" | "i128" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
        | "isize" => Type::Int,
        "f32" | "f64" => Type::Float,
        "bool" => Type::Bool,
        "String" | "str" => Type::Str,
        "()" => Type::Unit,
        other => {
            // Vec<T> / Option<T> structural recognition; otherwise Named.
            if let Some(rest) = other.strip_prefix("Vec<").and_then(|r| r.strip_suffix('>')) {
                Type::Vec(Box::new(type_from_str(rest)))
            } else if let Some(rest) = other
                .strip_prefix("Option<")
                .and_then(|r| r.strip_suffix('>'))
            {
                Type::Option(Box::new(type_from_str(rest)))
            } else {
                Type::Named(other.to_string(), Vec::new())
            }
        }
    }
}

fn span_of(t: &Token) -> Span {
    let (s, e) = t
        .location_data
        .as_ref()
        .map(|l| (l.range.0 as u32, l.range.1 as u32))
        .unwrap_or((0, t.length as u32));
    Span::new(s, e)
}

/// Decide int vs float from the literal text (Copper's tokenizer keeps the
/// numeric value as the token's string).
fn number_literal(s: &str) -> Literal {
    let is_float = s.contains('.') || s.contains('e') || s.contains('E');
    if is_float {
        s.parse::<f64>()
            .map(Literal::Float)
            .unwrap_or(Literal::Float(0.0))
    } else {
        s.parse::<i64>()
            .map(Literal::Int)
            .unwrap_or(Literal::Int(0))
    }
}

/// Build a [`StrTemplate`] from a string token. Interpolated strings carry a
/// `Data::Interpolation { placeholder, args }` (the tokenizer already split
/// out `$ident` / `${expr}`); we parse each arg with this same subset parser.
/// Plain strings become a single literal part.
fn str_template(t: &Token) -> StrTemplate {
    if let Data::Interpolation { placeholder, args } = &t.data {
        let inner = unquote(placeholder);
        let mut parts = Vec::new();
        let segments: Vec<&str> = inner.split("{}").collect();
        let mut ai = 0usize;
        for (i, seg) in segments.iter().enumerate() {
            if !seg.is_empty() {
                parts.push(StrPart::Lit(unescape(seg)));
            }
            if i + 1 < segments.len() {
                if let Some(arg) = args.get(ai) {
                    if let (Some(e), _) = parse_expr(arg) {
                        parts.push(StrPart::Expr(Box::new(e)));
                    }
                }
                ai += 1;
            }
        }
        return StrTemplate { parts };
    }
    StrTemplate {
        parts: vec![StrPart::Lit(unescape(unquote(&t.value)))],
    }
}

fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(s)
}

/// Process C-style backslash escapes in a string-literal body (after the
/// surrounding quotes were stripped by [`unquote`]). Produces the real
/// control characters so `Text("a\nb")` renders two lines on every OS
/// (`\n` → U+000A, identical bytes on Windows/macOS/Linux — the line
/// break is honoured by the text renderer, not by a platform newline).
///
/// Recognised: `\n \r \t \0 \\ \" \'`, plus `\xHH` (1–2 hex
/// digits) and `\u{...}` (Unicode scalar). An unknown escape keeps the
/// backslash verbatim so it round-trips instead of being silently eaten.
fn unescape(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('0') => out.push('\0'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('x') => {
                // \xHH — up to 2 hex digits.
                let mut hex = String::new();
                while hex.len() < 2 {
                    match chars.peek() {
                        Some(h) if h.is_ascii_hexdigit() => {
                            hex.push(*h);
                            chars.next();
                        }
                        _ => break,
                    }
                }
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) => out.push(ch),
                    None => {
                        out.push('\\');
                        out.push('x');
                        out.push_str(&hex);
                    }
                }
            }
            Some('u') => {
                // \u{XXXX} — Unicode scalar in braces.
                if chars.peek() == Some(&'{') {
                    chars.next();
                    let mut hex = String::new();
                    while let Some(h) = chars.peek() {
                        if *h == '}' {
                            chars.next();
                            break;
                        }
                        if h.is_ascii_hexdigit() && hex.len() < 6 {
                            hex.push(*h);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        Some(ch) => out.push(ch),
                        None => {
                            out.push_str("\\u{");
                            out.push_str(&hex);
                            out.push('}');
                        }
                    }
                } else {
                    out.push('\\');
                    out.push('u');
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expr(s: &str) -> Expr {
        let (e, errs) = parse_expr(s);
        assert!(errs.is_empty(), "errors for {s:?}: {errs:?}");
        e.unwrap_or_else(|| panic!("no expr for {s:?}"))
    }

    #[test]
    fn literals_and_idents() {
        assert!(matches!(
            expr("42").kind,
            ExprKind::Literal(Literal::Int(42))
        ));
        assert!(matches!(
            expr("3.5").kind,
            ExprKind::Literal(Literal::Float(_))
        ));
        assert!(matches!(
            expr("true").kind,
            ExprKind::Literal(Literal::Bool(true))
        ));
        assert!(matches!(expr("count").kind, ExprKind::Ident(ref n) if n == "count"));
    }

    #[test]
    fn precedence() {
        // 1 + 2 * 3  ==  1 + (2 * 3)
        let e = expr("1 + 2 * 3");
        let ExprKind::Binary {
            op: BinOp::Add,
            rhs,
            ..
        } = e.kind
        else {
            panic!("expected top-level +, got {:?}", e.kind);
        };
        assert!(matches!(rhs.kind, ExprKind::Binary { op: BinOp::Mul, .. }));
    }

    #[test]
    fn comparison_and_logic() {
        let e = expr("count > 10 && ok");
        assert!(matches!(e.kind, ExprKind::Binary { op: BinOp::And, .. }));
    }

    #[test]
    fn call_member_index() {
        let e = expr("user.posts[0].title");
        assert!(matches!(e.kind, ExprKind::Member { ref field, .. } if field == "title"));
    }

    #[test]
    fn ternary() {
        let e = expr("done ? 1 : 0");
        assert!(matches!(e.kind, ExprKind::Ternary { .. }));
    }

    #[test]
    fn assignment_compound() {
        let e = expr("count += step");
        assert!(matches!(
            e.kind,
            ExprKind::Assign {
                op: AssignOp::Add,
                ..
            }
        ));
    }

    #[test]
    fn closure() {
        // Closures appear nested (call args, handler blocks), never at the
        // absolute start of input — Copper's tokenizer mangles a leading `|`.
        let e = expr("(|x, y| x + y)");
        let ExprKind::Closure { params, .. } = e.kind else {
            panic!("expected closure, got {:?}", e.kind)
        };
        assert_eq!(params, vec!["x".to_string(), "y".to_string()]);
    }

    #[test]
    fn struct_literal() {
        let e = expr("Todo { id: 1, title: t }");
        let ExprKind::StructLit { name, fields, .. } = e.kind else {
            panic!("expected struct lit")
        };
        assert_eq!(name, "Todo");
        assert_eq!(fields.len(), 2);
    }

    #[test]
    fn match_expr() {
        let e = expr("match x { some(v) => v, none => 0 }");
        let ExprKind::Match { arms, .. } = e.kind else {
            panic!("expected match")
        };
        assert_eq!(arms.len(), 2);
    }

    #[test]
    fn block_with_statements() {
        // Copper bindings: `mut a = 1` / bare `b = 2` (NO `let`), then a tail.
        let (b, errs) = parse_stmts("mut a = 1\nb = 2\na + b");
        assert!(errs.is_empty(), "errs: {errs:?}");
        assert_eq!(b.stmts.len(), 2);
        let Stmt::Let { name, mutable, .. } = &b.stmts[0] else {
            panic!("expected binding, got {:?}", b.stmts[0]);
        };
        assert_eq!(name, "a");
        assert!(mutable);
        let Stmt::Let { name, mutable, .. } = &b.stmts[1] else {
            panic!("expected binding, got {:?}", b.stmts[1]);
        };
        assert_eq!(name, "b");
        assert!(!mutable, "bare binding is immutable");
        assert!(b.tail.is_some());
    }

    // ---- forms added to match the real grammar (drift guard) ----

    #[test]
    fn bitwise_operators() {
        assert!(matches!(
            expr("a & b").kind,
            ExprKind::Binary {
                op: BinOp::BitAnd,
                ..
            }
        ));
        assert!(matches!(
            expr("a | b").kind,
            ExprKind::Binary {
                op: BinOp::BitOr,
                ..
            }
        ));
        assert!(matches!(
            expr("a ^ b").kind,
            ExprKind::Binary {
                op: BinOp::BitXor,
                ..
            }
        ));
    }

    #[test]
    fn all_compound_assigns() {
        for (src, want) in [
            ("x %= 2", AssignOp::Rem),
            ("x &= 2", AssignOp::BitAnd),
            ("x |= 2", AssignOp::BitOr),
            ("x ^= 2", AssignOp::BitXor),
        ] {
            let e = expr(src);
            let ExprKind::Assign { op, .. } = e.kind else {
                panic!("expected assign for {src:?}, got {:?}", e.kind);
            };
            assert_eq!(op, want, "for {src:?}");
        }
    }

    #[test]
    fn cast_uses_copper_aliases() {
        // `as int` must resolve through the shared lexicon to Type::Int, just
        // like the transpiler maps `int` -> `i64`.
        let e = expr("x as int");
        let ExprKind::Cast { ty, .. } = e.kind else {
            panic!("expected cast, got {:?}", e.kind);
        };
        assert_eq!(ty, Type::Int);
    }

    #[test]
    fn path_and_turbofish() {
        assert!(matches!(expr("std::io").kind, ExprKind::Path { .. }));
        // `s.parse::<i32>()` — turbofish call.
        let e = expr("s.parse::<i32>()");
        assert!(
            matches!(
                e.kind,
                ExprKind::Call {
                    turbofish: Some(_),
                    ..
                }
            ),
            "expected turbofish call, got {:?}",
            e.kind
        );
    }

    #[test]
    fn postfix_try_vs_ternary() {
        // `expr?` with nothing after the `?` is the try operator.
        let (e, _) = parse_expr("foo()?");
        assert!(matches!(e.unwrap().kind, ExprKind::Try { .. }));
        // `cond ? a : b` is still a ternary.
        assert!(matches!(expr("c ? 1 : 0").kind, ExprKind::Ternary { .. }));
    }

    #[test]
    fn prefix_ref_and_deref() {
        assert!(matches!(
            expr("&x").kind,
            ExprKind::Unary { op: UnOp::Ref, .. }
        ));
        assert!(matches!(
            expr("&mut x").kind,
            ExprKind::Unary {
                op: UnOp::RefMut,
                ..
            }
        ));
        assert!(matches!(
            expr("*p").kind,
            ExprKind::Unary {
                op: UnOp::Deref,
                ..
            }
        ));
    }
}
