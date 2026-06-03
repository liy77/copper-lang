//! tower-lsp `Backend` for the Copper language server.
//!
//! Phase 2 surface (this commit):
//! * `initialize` / `initialized` / `shutdown`
//! * `textDocument/didOpen` / `didChange` / `didClose`
//! * `textDocument/publishDiagnostics` (syntax errors from
//!   `copper-syntax`)
//! * `textDocument/documentSymbol` (functions, structs, classes, impls,
//!   imports — flat list)
//! * `textDocument/hover` (built-in keyword + type cheatsheet)
//! * `textDocument/completion` (keywords + `cstd::*` with full signatures
//!   + symbols from the current file)
//! * `textDocument/signatureHelp` (parameter hints inside `(...)` after a
//!   call to a known function — cstd or user-defined in the file)

use copper_syntax::ast::{AstNode, ClassMember, Param};
use once_cell::sync::Lazy;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::docs::{Document, DocumentMap};
use crate::imports;
use crate::rust_prelude;
use crate::stdlib_methods;

pub struct Backend {
    client: Client,
    docs: DocumentMap,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            docs: DocumentMap::default(),
        }
    }

    async fn refresh_diagnostics(&self, uri: &Url) {
        let Some(doc) = self.docs.get(uri) else {
            return;
        };
        let mut diagnostics: Vec<Diagnostic> = doc
            .parsed
            .errors
            .iter()
            .map(|e| Diagnostic {
                range: doc.span_to_range(e.span),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("copper".to_string()),
                message: e.message.clone(),
                ..Default::default()
            })
            .collect();

        // Unresolved-import warnings: flag any `import … from <module>` whose
        // module isn't std/cstd, a sibling file, or a declared dependency.
        let dir = imports::dir_of_uri(uri);
        for imp in imports::parse_imports(&doc.text.to_string()) {
            if !imports::resolve(&imp.module, dir.as_deref()).is_found() {
                diagnostics.push(Diagnostic {
                    range: doc.span_to_range(imp.module_span),
                    severity: Some(DiagnosticSeverity::WARNING),
                    source: Some("copper".to_string()),
                    message: format!(
                        "import `{}` not found — not std/cstd, no sibling file, and not in \
                         properties.kson. Run `cforge install {}` to add it.",
                        imp.module, imp.module
                    ),
                    ..Default::default()
                });
            }
        }

        drop(doc);
        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "copper-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                document_symbol_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    // `.` for member access, `:` for `Type::static` access.
                    // (`:` triggers on every keystroke of `::`, but the
                    // handler returns nothing when the prefix isn't
                    // actually a `Type::` expression, so it's harmless.)
                    trigger_characters: Some(vec![":".to_string(), ".".to_string()]),
                    ..Default::default()
                }),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
                    retrigger_characters: None,
                    work_done_progress_options: Default::default(),
                }),
                definition_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(
                MessageType::INFO,
                format!(
                    "copper-lsp initialized — cstd loaded {} fns",
                    CSTD_FNS.len()
                ),
            )
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        self.docs.open(uri.clone(), params.text_document.text);
        self.refresh_diagnostics(&uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        if let Some(change) = params.content_changes.into_iter().next() {
            self.docs.change(&uri, change.text);
        }
        self.refresh_diagnostics(&uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.docs.close(&params.text_document.uri);
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let Some(doc) = self.docs.get(&uri) else {
            return Ok(None);
        };

        #[allow(deprecated)]
        let symbols: Vec<DocumentSymbol> = doc
            .parsed
            .nodes
            .iter()
            .map(|n| {
                let (name, kind, detail) = match n {
                    AstNode::Function {
                        name,
                        params,
                        return_type,
                        ..
                    } => (
                        name.clone(),
                        SymbolKind::FUNCTION,
                        Some(format_signature(name, params, return_type.as_deref())),
                    ),
                    AstNode::Struct { name, .. } => (name.clone(), SymbolKind::STRUCT, None),
                    AstNode::Class { name, .. } => (name.clone(), SymbolKind::CLASS, None),
                    AstNode::Impl { target, .. } => (target.clone(), SymbolKind::INTERFACE, None),
                    AstNode::Use { path, .. } => (path.clone(), SymbolKind::MODULE, None),
                    AstNode::Let { name, mutable, .. } => (
                        name.clone(),
                        if *mutable {
                            SymbolKind::VARIABLE
                        } else {
                            SymbolKind::CONSTANT
                        },
                        None,
                    ),
                };
                let range = doc.span_to_range(n.span());
                DocumentSymbol {
                    name,
                    detail,
                    kind,
                    tags: None,
                    deprecated: None,
                    range,
                    selection_range: range,
                    children: None,
                }
            })
            .collect();

        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .clone();
        let pos = params.text_document_position_params.position;

        let Some(doc) = self.docs.get(&uri) else {
            return Ok(None);
        };
        let Some(word) = word_at(&doc.text, pos) else {
            return Ok(None);
        };

        // 0. Import targets: hovering the module of an `import … from <mod>`
        //    describes where it resolves (stdlib / cstd / local file / crate /
        //    not found); hovering an imported name says where it came from.
        {
            let dir = imports::dir_of_uri(&uri);
            for imp in imports::parse_imports(&doc.text.to_string()) {
                if imp.module == word {
                    let res = imports::resolve(&imp.module, dir.as_deref());
                    return Ok(Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: res.describe(&imp.module),
                        }),
                        range: Some(doc.span_to_range(imp.module_span)),
                    }));
                }
                if let Some((name, span)) = imp.names.iter().find(|(n, _)| n == &word) {
                    let res = imports::resolve(&imp.module, dir.as_deref());
                    let status = if res.is_found() {
                        ""
                    } else {
                        " — ⚠️ module not found"
                    };
                    return Ok(Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: format!(
                                "**`{name}`** — imported from `{}`{status}.\n\n{}",
                                imp.module,
                                res.describe(&imp.module)
                            ),
                        }),
                        range: Some(doc.span_to_range(*span)),
                    }));
                }
            }
        }

        // 1. Built-in keyword/type cheatsheet.
        if let Some(info) = builtin_info(&word) {
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: info,
                }),
                range: None,
            }));
        }

        // 2. Rust prelude (Some/None/Ok/Err, Option, Result, String, Vec,
        //    HashMap, println!, format!, vec!, primitive types, …).
        //    Hover lookup tries both `name` and `name!` so the user can
        //    hover the bare identifier `println` or the macro form.
        if let Some(item) =
            rust_prelude::lookup(&word).or_else(|| rust_prelude::lookup(&format!("{}!", word)))
        {
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("```rust\n{}\n```\n\n{}", item.detail, item.doc),
                }),
                range: None,
            }));
        }

        // 3. cstd function?
        if let Some(f) = CSTD_FNS.iter().find(|f| f.name == word) {
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("```copper\n{}\n```\n\nFrom `cstd`.", f.signature),
                }),
                range: None,
            }));
        }

        // 4. User-defined function in this file?
        if let Some(AstNode::Function {
            name,
            params,
            return_type,
            ..
        }) = doc
            .parsed
            .nodes
            .iter()
            .find(|n| matches!(n, AstNode::Function { name, .. } if name == &word))
        {
            let mut value = format!(
                "```copper\n{}\n```",
                format_signature(name, params, return_type.as_deref())
            );
            // Append the `///` doc comment above the definition, like Rust.
            if let Some(d) = doc_comment_for(&doc.text.to_string(), &word) {
                value.push_str("\n\n");
                value.push_str(&d);
            }
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value,
                }),
                range: None,
            }));
        }

        // 5. Member access: `Type::word`, `Type::new(...).word`, or
        //    `instance.word`. The receiver may resolve to a class defined
        //    in the file OR to a stdlib type (`String`, `Vec`, …) — try
        //    both tables.
        if let Some(receiver_type) = member_receiver_type(&doc, pos, &word) {
            // 5a) Class in this file.
            if let Some(members) = doc.parsed.nodes.iter().find_map(|n| match n {
                AstNode::Class { name, members, .. } if name == &receiver_type => Some(members),
                _ => None,
            }) {
                // Constructor lookup: `ClassName::new` → the synthesised
                // `new` from the Copper-syntax constructor.
                if word == "new" {
                    if let Some(ClassMember::Constructor { params, .. }) = members
                        .iter()
                        .find(|m| matches!(m, ClassMember::Constructor { .. }))
                    {
                        let sig = format_method_signature("new", params, Some("Self"));
                        return Ok(Some(Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value: format!(
                                    "```copper\n{}\n```\n\nConstructor for `{}` (Copper class syntax → Rust `pub fn new`).",
                                    sig, receiver_type
                                ),
                            }),
                            range: None,
                        }));
                    }
                }
                if let Some(hover) = hover_for_member(&receiver_type, members, &word) {
                    return Ok(Some(hover));
                }
            }
            // 5b) Stdlib type (String/Vec/Option/Result/HashMap/...).
            let methods = stdlib_methods::methods_for(&receiver_type);
            if let Some(m) = methods.iter().find(|m| m.label == word) {
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: format!(
                            "```rust\n{}\n```\n\n{}\n\nOn `{}`.",
                            m.detail, m.doc, receiver_type
                        ),
                    }),
                    range: None,
                }));
            }
        }

        // 6. Best-effort fallback: a method name unique across all classes
        //    in the file, hovered without a clear receiver. Common case:
        //    the user hovers a method declaration line.
        for n in &doc.parsed.nodes {
            if let AstNode::Class {
                name: cname,
                members,
                ..
            } = n
            {
                if let Some(hover) = hover_for_member(cname, members, &word) {
                    return Ok(Some(hover));
                }
            }
        }

        // 7. Local binding (let/mut) or function parameter — shows the
        //    inferred type so hovering `abs` in `mut abs = x < 0 ? -x : x`
        //    surfaces something rather than nothing.
        if let Some(hover) = local_binding_hover(&doc, pos, &word) {
            return Ok(Some(hover));
        }

        Ok(None)
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let Some(doc) = self.docs.get(&uri) else {
            return Ok(None);
        };

        // ---- Member-access completion ----
        // If the trigger context is `<receiver>.<cursor>`, return only the
        // members of `<receiver>`'s inferred class — bypass the keyword /
        // global symbol noise.
        if let Some(receiver) = receiver_before_dot(&doc.text, pos) {
            if let Some(items) = members_of(&doc, &receiver, pos) {
                return Ok(Some(CompletionResponse::Array(items)));
            }
        }
        // Also: receiver_before_dot may match where members_of returns
        // None (unknown type with no fallback). The outer flow then
        // produces all keywords, which is wrong for `obj.|`. Make sure
        // we always return SOMETHING after a dot, even if it's the
        // stdlib union.
        // (`members_of` already returns the union for unknown types, so
        // we land in the early return above. Defensive note left here.)

        // ---- Static member access: `ClassName::<cursor>` ----
        // Copper's class-syntax constructor `ClassName(args) { ... }`
        // lowers to `pub fn new(args) -> Self` in Rust, so `ClassName::`
        // should offer `new(...)` plus any user-defined static methods
        // (methods whose first param isn't `self`).
        if let Some(class_name) = receiver_before_colons(&doc.text, pos) {
            if let Some(items) = static_members_of(&doc, &class_name) {
                return Ok(Some(CompletionResponse::Array(items)));
            }
        }

        // Keyword completions come from the shared lexicon
        // (editors/copper-lexicon.json), the same source the hover cheatsheet
        // and the MUI/CRM extension use — so they can never drift.
        let mut items: Vec<CompletionItem> = crate::lexicon::keywords()
            .iter()
            .map(|e| CompletionItem {
                label: e.name.clone(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some(e.doc.clone()),
                ..Default::default()
            })
            .collect();

        // User-defined symbols from this file (with signature for funcs).
        for n in &doc.parsed.nodes {
            match n {
                AstNode::Function {
                    name,
                    params,
                    return_type,
                    ..
                } => {
                    let sig = format_signature(name, params, return_type.as_deref());
                    items.push(CompletionItem {
                        label: name.clone(),
                        kind: Some(CompletionItemKind::FUNCTION),
                        detail: Some(sig.clone()),
                        insert_text: Some(snippet_for_call(name, params)),
                        insert_text_format: Some(InsertTextFormat::SNIPPET),
                        documentation: Some(Documentation::MarkupContent(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: format!("```copper\n{}\n```", sig),
                        })),
                        ..Default::default()
                    });
                }
                AstNode::Struct { name, .. } => items.push(CompletionItem {
                    label: name.clone(),
                    kind: Some(CompletionItemKind::STRUCT),
                    ..Default::default()
                }),
                AstNode::Class { name, .. } => items.push(CompletionItem {
                    label: name.clone(),
                    kind: Some(CompletionItemKind::CLASS),
                    ..Default::default()
                }),
                AstNode::Let { name, .. } => items.push(CompletionItem {
                    label: name.clone(),
                    kind: Some(CompletionItemKind::VARIABLE),
                    ..Default::default()
                }),
                _ => {}
            }
        }

        // cstd helpers — full signature in detail, snippet for tab-stops.
        for f in CSTD_FNS.iter() {
            items.push(CompletionItem {
                label: f.name.clone(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(f.signature.clone()),
                insert_text: Some(snippet_for_call(&f.name, &f.params)),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                documentation: Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("From `cstd`.\n\n```copper\n{}\n```", f.signature),
                })),
                ..Default::default()
            });
        }

        // Rust prelude (Some/None/Ok/Err, Option/Result/Vec/HashMap, the
        // print/format/vec macros, primitive types, …).
        for item in rust_prelude::all() {
            items.push(CompletionItem {
                label: item.label.to_string(),
                kind: Some(item.kind),
                detail: Some(item.detail.to_string()),
                insert_text: Some(item.insert.to_string()),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                documentation: Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: item.doc.to_string(),
                })),
                ..Default::default()
            });
        }

        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let Some(doc) = self.docs.get(&uri) else {
            return Ok(None);
        };

        // Walk back from cursor to find the enclosing `(` and the call name.
        let Some((call_name, active_param)) = call_context_at(&doc.text, pos) else {
            return Ok(None);
        };

        // Lookup priority:
        //   1. cstd function (parsed from `std/cstd.crs`)
        //   2. file-local user function
        //   3. Rust prelude entry (Some/Ok/Err, println!, format!, …)
        //
        // For Rust prelude callables we only have static parameter labels,
        // so we synthesise `Param`s with empty spans/types just to drive
        // the signature_help renderer.
        let info: Option<(String, Vec<ParamLabel>)> = CSTD_FNS
            .iter()
            .find(|f| f.name == call_name)
            .map(|f| {
                (
                    f.signature.clone(),
                    f.params.iter().map(ParamLabel::from_param).collect(),
                )
            })
            .or_else(|| {
                doc.parsed.nodes.iter().find_map(|n| match n {
                    AstNode::Function {
                        name,
                        params,
                        return_type,
                        ..
                    } if name == &call_name => Some((
                        format_signature(name, params, return_type.as_deref()),
                        params.iter().map(ParamLabel::from_param).collect(),
                    )),
                    _ => None,
                })
            })
            .or_else(|| {
                // Rust prelude — try both `name` and `name!` so it matches
                // both `Some(` and `println(` (the `!` isn't part of the
                // word `call_context_at` extracts).
                let item = rust_prelude::lookup(&call_name)
                    .or_else(|| rust_prelude::lookup(&format!("{}!", call_name)))?;
                let params = item.params?;
                Some((
                    item.detail.to_string(),
                    params
                        .iter()
                        .map(|s| ParamLabel {
                            text: s.to_string(),
                        })
                        .collect(),
                ))
            });

        let Some((label, params)) = info else {
            return Ok(None);
        };

        let parameters = params
            .into_iter()
            .map(|p| ParameterInformation {
                label: ParameterLabel::Simple(p.text),
                documentation: None,
            })
            .collect();

        Ok(Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label,
                documentation: None,
                parameters: Some(parameters),
                active_parameter: Some(active_param),
            }],
            active_signature: Some(0),
            active_parameter: Some(active_param),
        }))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let Some(doc) = self.docs.get(&uri) else {
            return Ok(None);
        };
        let Some(word) = word_at(&doc.text, pos) else {
            return Ok(None);
        };

        // Walk parsed nodes looking for a definition matching the hovered word.
        // Functions > Classes > Structs > Let bindings, first match wins.
        for node in &doc.parsed.nodes {
            let matches = match node {
                AstNode::Function { name, .. } => name == &word,
                AstNode::Class { name, .. } => name == &word,
                AstNode::Struct { name, .. } => name == &word,
                AstNode::Let { name, .. } => name == &word,
                _ => false,
            };
            if matches {
                let range = doc.span_to_range(node.span());
                return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                    uri: uri.clone(),
                    range,
                })));
            }
        }
        Ok(None)
    }
}

/// Collect the `///` doc comment immediately above the definition of `name`
/// (a `func`/`fn` whose signature contains `name(`). Returns the joined doc text
/// (markdown), or `None` when there's no doc. Mirrors Rust's hover docs.
fn doc_comment_for(text: &str, name: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let needle = format!("{name}(");
    let def = lines.iter().position(|l| {
        let t = l.trim_start();
        (t.starts_with("func ")
            || t.starts_with("fn ")
            || t.contains(" func ")
            || t.contains(" fn "))
            && l.contains(&needle)
    })?;

    // Walk up over contiguous `///` lines directly above the definition.
    let mut docs: Vec<String> = Vec::new();
    let mut i = def;
    while i > 0 {
        i -= 1;
        let t = lines[i].trim_start();
        if let Some(rest) = t.strip_prefix("///") {
            docs.push(rest.trim().to_string());
        } else {
            break;
        }
    }
    if docs.is_empty() {
        return None;
    }
    docs.reverse();
    Some(docs.join("\n"))
}

fn word_at(rope: &ropey::Rope, pos: Position) -> Option<String> {
    let line = rope.get_line(pos.line as usize)?;
    let line_str = line.to_string();
    let chars: Vec<char> = line_str.chars().collect();
    let col = (pos.character as usize).min(chars.len());

    let is_word = |c: char| c.is_alphanumeric() || c == '_';

    if col >= chars.len() && !chars.last().map(|c| is_word(*c)).unwrap_or(false) {
        return None;
    }
    let mut start = col;
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end < chars.len() && is_word(chars[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    Some(chars[start..end].iter().collect())
}

/// Walk backwards from `pos` looking for the unmatched `(` of the enclosing
/// call, then read the identifier just before it. Returns `(callee, comma_count)`
/// where `comma_count` is the active parameter index (commas at our paren
/// depth between `(` and the cursor).
fn call_context_at(rope: &ropey::Rope, pos: Position) -> Option<(String, u32)> {
    // Linearize the rope up to the cursor.
    let line_idx = pos.line as usize;
    let mut text = String::new();
    for (i, line) in rope.lines().enumerate() {
        if i < line_idx {
            text.push_str(&line.to_string());
        } else if i == line_idx {
            let line_str = line.to_string();
            let chars: Vec<char> = line_str.chars().collect();
            let col = (pos.character as usize).min(chars.len());
            text.extend(chars[..col].iter());
            break;
        }
    }

    let bytes: Vec<char> = text.chars().collect();
    let mut depth: i32 = 0;
    let mut commas: u32 = 0;
    let mut i = bytes.len();
    let mut paren_pos: Option<usize> = None;
    while i > 0 {
        i -= 1;
        let c = bytes[i];
        match c {
            ')' | ']' | '}' => depth += 1,
            '(' => {
                if depth == 0 {
                    paren_pos = Some(i);
                    break;
                }
                depth -= 1;
            }
            '[' | '{' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            ',' if depth == 0 => commas += 1,
            _ => {}
        }
    }

    let paren_pos = paren_pos?;
    // Read the callee identifier immediately before `(`. Macros like
    // `println!(` carry a trailing `!` we need to skip — the lookup in
    // signature_help tries both `name` and `name!` so the bare ident is
    // enough.
    let mut j = paren_pos;
    while j > 0 && (bytes[j - 1].is_whitespace()) {
        j -= 1;
    }
    if j > 0 && bytes[j - 1] == '!' {
        j -= 1;
    }
    let mut k = j;
    while k > 0 {
        let ch = bytes[k - 1];
        if ch.is_alphanumeric() || ch == '_' {
            k -= 1;
        } else {
            break;
        }
    }
    if k == j {
        return None;
    }
    Some((bytes[k..j].iter().collect(), commas))
}

fn builtin_info(word: &str) -> Option<String> {
    // Keyword + primitive-type cheatsheet lives in the shared lexicon
    // (editors/copper-lexicon.json) so the text is identical to what the
    // MUI/CRM extension shows for the same words. Edit the JSON, not this fn.
    crate::lexicon::builtin_markdown(word)
}

// (Keyword completion items are now built from crate::lexicon, which reads the
// shared editors/copper-lexicon.json — see the completion handler above.)

fn format_param(p: &Param) -> String {
    match &p.type_name {
        Some(t) => format!("{}: {}", p.name, t),
        None => p.name.clone(),
    }
}

/// Unified parameter-label representation used by signature_help. Lets us
/// merge AST-derived params (cstd, file-local) with the static slice of
/// strings that Rust-prelude entries carry.
struct ParamLabel {
    text: String,
}

impl ParamLabel {
    fn from_param(p: &Param) -> Self {
        Self {
            text: format_param(p),
        }
    }
}

fn format_signature(name: &str, params: &[Param], return_type: Option<&str>) -> String {
    let parts = params
        .iter()
        .map(format_param)
        .collect::<Vec<_>>()
        .join(", ");
    match return_type {
        Some(rt) if !rt.is_empty() && rt != "void" => {
            format!("func {} {}({})", rt, name, parts)
        }
        _ => format!("func {}({})", name, parts),
    }
}

/// Build a snippet like `name(${1:a}, ${2:b})` — VSCode jumps between
/// placeholders with Tab.
fn snippet_for_call(name: &str, params: &[Param]) -> String {
    if params.is_empty() {
        return format!("{}()", name);
    }
    let inner = params
        .iter()
        .enumerate()
        .map(|(i, p)| format!("${{{}:{}}}", i + 1, p.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}({})", name, inner)
}

/// Receiver of a `.<cursor>` member-access expression. Either a plain
/// identifier (variable / `self`) or a static call (`Class::method(args)`),
/// which we resolve to its return type later.
enum Receiver {
    /// `self` or a variable name.
    Plain(String),
    /// `Type::method` — the call result's type comes from the method's
    /// return type (or `Type` itself when method == "new").
    StaticCall { ty: String, method: String },
}

/// Walk back from `pos` to find the receiver of a `.<cursor>` member
/// access. Handles:
///   * `var.|`               → `Plain("var")`
///   * `self.|`              → `Plain("self")`
///   * `Class::new(args).|`  → `StaticCall { ty: "Class", method: "new" }`
///   * `Class::other(...).|` → `StaticCall { ty: "Class", method: "other" }`
///
/// Chained instance calls (`a.b().c.|`) are not yet supported.
fn receiver_before_dot(rope: &ropey::Rope, pos: Position) -> Option<Receiver> {
    let line = rope.get_line(pos.line as usize)?;
    let line_str = line.to_string();
    let chars: Vec<char> = line_str.chars().collect();
    let col = (pos.character as usize).min(chars.len());

    // Walk back over an in-progress identifier (the user may have typed a
    // few chars after the dot already).
    let mut i = col;
    while i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
        i -= 1;
    }
    if i == 0 || chars[i - 1] != '.' {
        return None;
    }
    let dot_pos = i - 1;

    // Case A: `)` immediately before the dot → call-result receiver. Walk
    // back to the matching `(`, then read the callee.
    if dot_pos > 0 && chars[dot_pos - 1] == ')' {
        let mut depth = 1i32;
        let mut k = dot_pos - 1;
        while k > 0 {
            k -= 1;
            match chars[k] {
                ')' => depth += 1,
                '(' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        if depth != 0 {
            return None;
        }
        // `k` now points at `(`. The callee is the identifier just before.
        let mut m = k;
        while m > 0 && (chars[m - 1].is_alphanumeric() || chars[m - 1] == '_') {
            m -= 1;
        }
        if m == k {
            return None;
        }
        let callee: String = chars[m..k].iter().collect();
        // If preceded by `::`, the part before is the type.
        if m >= 2 && chars[m - 1] == ':' && chars[m - 2] == ':' {
            let mut n = m - 2;
            while n > 0 && (chars[n - 1].is_alphanumeric() || chars[n - 1] == '_') {
                n -= 1;
            }
            if n < m - 2 {
                let ty: String = chars[n..m - 2].iter().collect();
                if ty.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                    return Some(Receiver::StaticCall { ty, method: callee });
                }
            }
        }
        // Plain `func(...)` — fall back to plain identifier so callers can
        // try return-type lookup.
        return Some(Receiver::Plain(callee));
    }

    // Case B: identifier immediately before the dot.
    let mut j = dot_pos;
    while j > 0 && (chars[j - 1].is_alphanumeric() || chars[j - 1] == '_') {
        j -= 1;
    }
    if j == dot_pos {
        return None;
    }
    Some(Receiver::Plain(chars[j..dot_pos].iter().collect()))
}

/// Walk back from `pos` looking for `<Type>::<cursor>`. Returns the type
/// name if the immediately-preceding chars are `::` and the word before
/// starts with an uppercase letter (the convention for class names).
fn receiver_before_colons(rope: &ropey::Rope, pos: Position) -> Option<String> {
    let line = rope.get_line(pos.line as usize)?;
    let line_str = line.to_string();
    let chars: Vec<char> = line_str.chars().collect();
    let col = (pos.character as usize).min(chars.len());

    // Walk back over an in-progress identifier (the user may have started
    // typing `Greeting::ne`).
    let mut i = col;
    while i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
        i -= 1;
    }
    // Expect `::` immediately before.
    if i < 2 || chars[i - 1] != ':' || chars[i - 2] != ':' {
        return None;
    }
    let colons_pos = i - 2;
    let mut j = colons_pos;
    while j > 0 && (chars[j - 1].is_alphanumeric() || chars[j - 1] == '_') {
        j -= 1;
    }
    if j == colons_pos {
        return None;
    }
    let receiver: String = chars[j..colons_pos].iter().collect();
    // Only treat capitalized identifiers as type names — keeps `var::`
    // (which would be a Rust path, not a class) from spamming completions.
    if !receiver
        .chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false)
    {
        return None;
    }
    Some(receiver)
}

/// Static members of `class_name`: `new` (synthesised from a Copper-style
/// constructor) plus methods that don't take `self` as their first param.
/// Returns `None` when no class with that name exists in the file.
fn static_members_of(doc: &Document, class_name: &str) -> Option<Vec<CompletionItem>> {
    let members = doc.parsed.nodes.iter().find_map(|n| match n {
        AstNode::Class { name, members, .. } if name == class_name => Some(members),
        _ => None,
    })?;

    let mut items = Vec::new();

    for m in members {
        match m {
            ClassMember::Constructor { params, .. } => {
                // The Copper class syntax `ClassName(args) { ... }` lowers
                // to `pub fn new(args) -> Self` — surface it as `new`.
                let inner = params
                    .iter()
                    .enumerate()
                    .map(|(i, p)| format!("${{{}:{}}}", i + 1, p.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                items.push(CompletionItem {
                    label: "new".to_string(),
                    kind: Some(CompletionItemKind::CONSTRUCTOR),
                    detail: Some(format_method_signature("new", params, Some("Self"))),
                    insert_text: Some(format!("new({})", inner)),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    documentation: Some(Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: format!(
                            "Constructor for `{}` (Copper class syntax → Rust `pub fn new`).",
                            class_name
                        ),
                    })),
                    ..Default::default()
                });
            }
            ClassMember::Method {
                name,
                params,
                return_type,
                ..
            } => {
                // Static method: doesn't take `self` / `Self` as first param.
                let is_static = params
                    .first()
                    .map(|p| p.name != "self" && p.name != "Self")
                    .unwrap_or(true);
                if !is_static {
                    continue;
                }
                let inner = params
                    .iter()
                    .enumerate()
                    .map(|(i, p)| format!("${{{}:{}}}", i + 1, p.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                items.push(CompletionItem {
                    label: name.clone(),
                    kind: Some(CompletionItemKind::FUNCTION),
                    detail: Some(format_method_signature(
                        name,
                        params,
                        return_type.as_deref(),
                    )),
                    insert_text: Some(format!("{}({})", name, inner)),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..Default::default()
                });
            }
            ClassMember::Field { .. } => {
                // Fields are instance state — only available off `obj.`,
                // never `Type::`.
            }
        }
    }

    Some(items)
}

/// Resolve `receiver` to a class in this file and return its members as
/// completion items. Handles:
///   * `self`     — finds the enclosing class via cursor byte offset
///   * `varname` — finds the matching `let`/`mut` and infers the type from
///     the source text immediately after `=`
fn members_of(doc: &Document, receiver: &Receiver, pos: Position) -> Option<Vec<CompletionItem>> {
    let resolved_type = match receiver {
        Receiver::Plain(name) if name == "self" => enclosing_class(doc, pos),
        Receiver::Plain(name) => infer_variable_type(doc, name, pos),
        Receiver::StaticCall { ty, method } => {
            // `Class::new(...).|` → an instance of `Class`.
            if method == "new" {
                Some(ty.clone())
            } else {
                // Static method call: look up its return type from the
                // class definition. Fall back to the type itself when
                // the return is `Self` / `void` / unknown.
                doc.parsed
                    .nodes
                    .iter()
                    .find_map(|n| match n {
                        AstNode::Class { name, members, .. } if name == ty => {
                            members.iter().find_map(|m| match m {
                                ClassMember::Method {
                                    name: mname,
                                    return_type,
                                    ..
                                } if mname == method => {
                                    let rt = return_type.clone().unwrap_or_default();
                                    let bare = rt.split('<').next().unwrap_or("").trim();
                                    if bare == "Self" || bare.is_empty() || bare == "void" {
                                        Some(ty.clone())
                                    } else {
                                        Some(bare.to_string())
                                    }
                                }
                                _ => None,
                            })
                        }
                        _ => None,
                    })
                    .or_else(|| Some(ty.clone()))
            }
        }
    };

    // 1) Class defined in this file → its fields + methods.
    if let Some(ref tn) = resolved_type {
        if let Some(class_members) = doc.parsed.nodes.iter().find_map(|n| match n {
            AstNode::Class { name, members, .. } if name == tn => Some(members),
            _ => None,
        }) {
            return Some(
                class_members
                    .iter()
                    .filter_map(member_to_completion)
                    .collect(),
            );
        }
    }

    // 2) Known stdlib type → curated method table.
    if let Some(ref tn) = resolved_type {
        let methods = stdlib_methods::methods_for(tn);
        if !methods.is_empty() {
            return Some(methods.iter().map(std_method_to_completion).collect());
        }
    }

    // 3) Type unknown → union of common stdlib methods so the user at
    //    least sees something useful (better than zero suggestions).
    let union = stdlib_methods::union_methods();
    if union.is_empty() {
        None
    } else {
        Some(union.iter().map(|m| std_method_to_completion(m)).collect())
    }
}

fn std_method_to_completion(m: &stdlib_methods::StdMethod) -> CompletionItem {
    CompletionItem {
        label: m.label.to_string(),
        kind: Some(stdlib_methods::to_completion_kind()),
        detail: Some(m.detail.to_string()),
        insert_text: Some(m.insert.to_string()),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("```rust\n{}\n```\n\n{}", m.detail, m.doc),
        })),
        ..Default::default()
    }
}

/// Hover for a bare identifier that resolves to a local binding —
/// function param, class field reachable through `self`, or `let`/`mut`
/// declaration. Returns `None` when nothing matches.
///
/// Reuses `infer_variable_type` so the type-inference rules stay in one
/// place (param → field → let RHS literal/call).
fn local_binding_hover(doc: &Document, pos: Position, word: &str) -> Option<Hover> {
    // Distinguish `mut x` from `let x` (purely cosmetic in the hover
    // header) by walking the parsed Let nodes for the latest matching
    // binding.
    let let_meta = doc.parsed.nodes.iter().rev().find_map(|n| match n {
        AstNode::Let { name, mutable, .. } if name == word => Some(*mutable),
        _ => None,
    });

    // Also detect "this is a function param" so we can label it as such
    // instead of pretending it's a let binding.
    let byte = position_to_byte(&doc.text, pos)? as u32;
    let param_label = doc.parsed.nodes.iter().find_map(|n| match n {
        AstNode::Function {
            params,
            body: Some(b),
            ..
        } if b.contains(byte) => params
            .iter()
            .find(|p| p.name == word)
            .map(|p| (p.name.clone(), p.type_name.clone())),
        _ => None,
    });

    let inferred = infer_variable_type(doc, word, pos);

    // Decide which label to show.
    let (header, ty_text) = if let Some((name, ty)) = param_label {
        (
            format!("(parameter) {}", name),
            ty.unwrap_or_else(|| "?".to_string()),
        )
    } else if let Some(mutable) = let_meta {
        let kw = if mutable { "mut" } else { "let" };
        (
            format!("{} {}", kw, word),
            inferred.clone().unwrap_or_else(|| "?".to_string()),
        )
    } else {
        // Nothing matched — neither param nor let.
        return None;
    };

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("```copper\n{}: {}\n```", header, ty_text),
        }),
        range: None,
    })
}

/// Render a hover bubble for `member_name` looked up in `members`. Returns
/// `None` if the member doesn't exist. Used by both the class-context
/// hover branch and the file-wide fallback.
fn hover_for_member(class_name: &str, members: &[ClassMember], member_name: &str) -> Option<Hover> {
    let m = members.iter().find(|m| m.name() == member_name)?;
    let value = match m {
        ClassMember::Field {
            name, type_name, ..
        } => format!(
            "```copper\n{}: {}\n```\n\nField on `{}`.",
            name,
            type_name.as_deref().unwrap_or("?"),
            class_name
        ),
        ClassMember::Method {
            name,
            params,
            return_type,
            ..
        } => format!(
            "```copper\n{}\n```\n\nMethod on `{}`.",
            format_method_signature(name, params, return_type.as_deref()),
            class_name
        ),
        ClassMember::Constructor { params, .. } => format!(
            "```copper\n{}\n```\n\nConstructor for `{}`.",
            format_method_signature(class_name, params, None),
            class_name
        ),
    };
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: None,
    })
}

/// Detect whether the word at `pos` is a member access (`.word` or
/// `::word`) and return the receiver's resolved type — could be a
/// class declared in the file OR a stdlib type (`String`, `Vec`, ...).
/// The caller decides which member table to consult.
fn member_receiver_type(doc: &Document, pos: Position, _word: &str) -> Option<String> {
    let line = doc.text.get_line(pos.line as usize)?;
    let line_str = line.to_string();
    let chars: Vec<char> = line_str.chars().collect();
    let col = (pos.character as usize).min(chars.len());

    // Walk back over the word itself so the receiver detectors can run
    // from the boundary just before it.
    let mut start = col;
    while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
        start -= 1;
    }
    let probe = Position {
        line: pos.line,
        character: start as u32,
    };

    if let Some(ty) = receiver_before_colons(&doc.text, probe) {
        return Some(ty);
    }
    if let Some(rx) = receiver_before_dot(&doc.text, probe) {
        return match rx {
            Receiver::Plain(name) if name == "self" => enclosing_class(doc, pos),
            Receiver::Plain(name) => infer_variable_type(doc, &name, pos),
            Receiver::StaticCall { ty, .. } => Some(ty),
        };
    }
    None
}

fn member_to_completion(m: &ClassMember) -> Option<CompletionItem> {
    match m {
        ClassMember::Field {
            name, type_name, ..
        } => Some(CompletionItem {
            label: name.clone(),
            kind: Some(CompletionItemKind::FIELD),
            detail: type_name.clone(),
            ..Default::default()
        }),
        ClassMember::Method {
            name,
            params,
            return_type,
            ..
        } => {
            // Skip the leading `self` from the snippet — `self` is implicit
            // when the user already typed `obj.`.
            let visible_params: Vec<&Param> = params
                .iter()
                .filter(|p| p.name != "self" && p.name != "Self")
                .collect();
            let inner = visible_params
                .iter()
                .enumerate()
                .map(|(i, p)| format!("${{{}:{}}}", i + 1, p.name))
                .collect::<Vec<_>>()
                .join(", ");
            Some(CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::METHOD),
                detail: Some(format_method_signature(
                    name,
                    params,
                    return_type.as_deref(),
                )),
                insert_text: Some(format!("{}({})", name, inner)),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            })
        }
        ClassMember::Constructor { .. } => None,
    }
}

fn format_method_signature(name: &str, params: &[Param], return_type: Option<&str>) -> String {
    let parts = params
        .iter()
        .map(format_param)
        .collect::<Vec<_>>()
        .join(", ");
    match return_type {
        Some(rt) if !rt.is_empty() && rt != "void" => {
            format!("func {} {}({})", rt, name, parts)
        }
        _ => format!("func {}({})", name, parts),
    }
}

/// Find the class whose body contains `pos` (in byte offsets). Used to
/// resolve `self` member access inside a method body.
fn enclosing_class(doc: &Document, pos: Position) -> Option<String> {
    let byte = position_to_byte(&doc.text, pos)?;
    doc.parsed.nodes.iter().find_map(|n| match n {
        AstNode::Class {
            name,
            body: Some(b),
            ..
        } if b.contains(byte as u32) => Some(name.clone()),
        _ => None,
    })
}

/// Best-effort variable-type inference. Looks at, in order:
///   1. Function parameters of the function whose body span contains
///      the cursor (handles `func void shout(msg: &str) { msg.| }`).
///   2. Class fields, when the cursor is inside a method body and the
///      name matches a field (handles `self.name.|` indirectly via the
///      `self.` path; this covers shorthand cases too).
///   3. The latest `let`/`mut` binding's annotation or RHS literal.
fn infer_variable_type(doc: &Document, name: &str, pos: Position) -> Option<String> {
    // Step 1: enclosing function param.
    if let Some(byte) = position_to_byte(&doc.text, pos) {
        let byte = byte as u32;
        if let Some(ty) = doc.parsed.nodes.iter().find_map(|n| match n {
            AstNode::Function {
                params,
                body: Some(b),
                ..
            } if b.contains(byte) => params.iter().find_map(|p| {
                if p.name == name {
                    p.type_name.clone()
                } else {
                    None
                }
            }),
            _ => None,
        }) {
            return Some(ty);
        }
        // Step 2: enclosing class field.
        if let Some(ty) = doc.parsed.nodes.iter().find_map(|n| match n {
            AstNode::Class {
                members,
                body: Some(b),
                ..
            } if b.contains(byte) => members.iter().find_map(|m| match m {
                ClassMember::Field {
                    name: fname,
                    type_name,
                    ..
                } if fname == name => type_name.clone(),
                ClassMember::Method {
                    name: mname,
                    params,
                    body: Some(mb),
                    ..
                } if mb.contains(byte) || mname == name => params
                    .iter()
                    .find_map(|p| (p.name == name).then(|| p.type_name.clone()).flatten()),
                _ => None,
            }),
            _ => None,
        }) {
            return Some(ty);
        }
    }

    // Step 3: latest `let`/`mut` binding for this name.
    let span = doc.parsed.nodes.iter().rev().find_map(|n| match n {
        AstNode::Let { name: n2, span, .. } if n2 == name => Some(*span),
        _ => None,
    })?;
    // Read source from just past the binding span and look for an `=`.
    let from = span.end as usize;
    let src = doc.text.slice(..).to_string();
    let tail = src.get(from..)?;

    // Skip whitespace, optional `: Type` annotation, then `=`.
    let bytes = tail.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    // Optional type annotation: `: Foo`
    if i < bytes.len() && bytes[i] == b':' {
        i += 1;
        // Capture the annotation as the type — wins over RHS inference.
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut end = i;
        while end < bytes.len() {
            let c = bytes[end];
            if c == b'=' || c == b'\n' || c == b';' {
                break;
            }
            end += 1;
        }
        let ann = tail[i..end].trim().trim_end_matches('=').trim().to_string();
        if !ann.is_empty() {
            // Strip generics: `Foo<...>` → `Foo`.
            let bare = ann.split('<').next().unwrap_or(&ann).trim().to_string();
            return Some(bare);
        }
    }
    // Otherwise look at the RHS of `=`.
    while i < bytes.len() && bytes[i] != b'=' {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    i += 1; // skip `=`
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }

    // Literal patterns first.
    let rhs = &tail[i..];
    if rhs.starts_with('"') {
        return Some("String".to_string());
    }
    if rhs.starts_with('\'') {
        return Some("char".to_string());
    }
    if rhs.starts_with("true") || rhs.starts_with("false") {
        return Some("bool".to_string());
    }
    if rhs.starts_with("vec!") {
        return Some("Vec".to_string());
    }
    if rhs.starts_with("Some(") || rhs.starts_with("None") {
        return Some("Option".to_string());
    }
    if rhs.starts_with("Ok(") || rhs.starts_with("Err(") {
        return Some("Result".to_string());
    }
    let first_byte = rhs.as_bytes()[0];
    if first_byte.is_ascii_digit() {
        return Some(if rhs.contains('.') { "f64" } else { "i64" }.to_string());
    }

    // Identifier start: could be `ClassName(...)`, `ClassName::new(...)`,
    // or a call to a known function (cstd / file-local) whose return type
    // we can look up.
    let mut end = i;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    if end == i {
        return None;
    }
    let ident = &tail[i..end];

    // Class constructor (Copper class syntax) or `ClassName::new(...)`:
    // if `ident` matches a class declared in this file, the receiver type
    // is that class.
    if doc
        .parsed
        .nodes
        .iter()
        .any(|n| matches!(n, AstNode::Class { name, .. } if name == ident))
    {
        return Some(ident.to_string());
    }

    // cstd / file-local function call: lift its return type.
    let return_ty = doc.parsed.nodes.iter().find_map(|n| match n {
        AstNode::Function {
            name, return_type, ..
        } if name == ident => return_type.clone(),
        _ => None,
    });
    if let Some(rt) = return_ty {
        let bare = rt.split('<').next().unwrap_or(&rt).trim().to_string();
        if !bare.is_empty() && bare != "void" {
            return Some(bare);
        }
    }

    // RHS starts with a lowercase identifier that we can resolve as a
    // local — propagate ITS type. Covers `mut abs = x < 0 ? -x : x`,
    // where the type comes from `x`. Guarded against self-reference so
    // a typo like `mut x = x` doesn't recurse forever.
    if !ident
        .chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false)
        && ident != name
    {
        if let Some(propagated) = infer_variable_type(doc, ident, pos) {
            return Some(propagated);
        }
    }

    // Last fallback: capitalized identifier → assume it's a type ctor we
    // don't have full info on (e.g. external Rust type the user `use`d).
    if ident
        .chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false)
    {
        Some(ident.to_string())
    } else {
        None
    }
}

fn position_to_byte(rope: &ropey::Rope, pos: Position) -> Option<usize> {
    let line = pos.line as usize;
    if line >= rope.len_lines() {
        return None;
    }
    let line_start = rope.line_to_byte(line);
    let line_str = rope.get_line(line)?.to_string();
    let chars: Vec<char> = line_str.chars().collect();
    let col = (pos.character as usize).min(chars.len());
    let prefix_bytes = chars[..col].iter().collect::<String>().len();
    Some(line_start + prefix_bytes)
}

/// Cached cstd function table — name + params + pretty signature. Built
/// once at startup by parsing the embedded `cstd.crs` source.
struct CstdFn {
    name: String,
    params: Vec<Param>,
    signature: String,
}

const CSTD_SOURCE: &str = include_str!("../../../std/cstd.crs");

static CSTD_FNS: Lazy<Vec<CstdFn>> = Lazy::new(|| {
    let parsed = copper_syntax::ast::parse(CSTD_SOURCE);
    parsed
        .nodes
        .into_iter()
        .filter_map(|n| match n {
            AstNode::Function {
                name,
                params,
                return_type,
                ..
            } => Some(CstdFn {
                signature: format_signature(&name, &params, return_type.as_deref()),
                name,
                params,
            }),
            _ => None,
        })
        .collect()
});
