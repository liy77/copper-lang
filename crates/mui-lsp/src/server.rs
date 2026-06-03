//! tower-lsp `Backend` for the MUI language server.
//!
//! Features:
//! * `initialize` / `initialized` / `shutdown`
//! * `didOpen` / `didChange` / `didClose` + `publishDiagnostics`
//!   (parse errors from `mui-syntax`, plus unresolved-import warnings)
//! * `completion` — widgets, props, enum members, prop values, imported
//!   components, `App {}` fields, keywords
//! * `hover` — widget / prop / enum / keyword docs
//! * `documentSymbol` — the `App` block, imports, and `view`s
//! * `documentColor` / `colorPresentation` — `#rrggbb[aa]` / `rgba()` swatches
//! * `definition` — jump from an import (or a use of an imported component) to
//!   the file that defines it

use std::collections::HashMap;
use std::path::PathBuf;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use copper_syntax::expr::ExprKind;
use mui_syntax::ast::{ImportKind, Node, PropValue};

use crate::catalog;
use crate::docs::{Document, DocumentMap};

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
                source: Some("mui".to_string()),
                message: e.message.clone(),
                ..Default::default()
            })
            .collect();

        // Unresolved-import diagnostics: resolve each import path relative to
        // this file's directory and flag the ones that don't exist on disk.
        if let Ok(file_path) = uri.to_file_path() {
            let dir = file_path.parent().map(PathBuf::from).unwrap_or_default();
            for imp in &doc.parsed.imports {
                let resolved = if std::path::Path::new(&imp.path).is_absolute() {
                    PathBuf::from(&imp.path)
                } else {
                    dir.join(&imp.path)
                };
                if !resolved.exists() {
                    diagnostics.push(Diagnostic {
                        range: doc.span_to_range(imp.span),
                        severity: Some(DiagnosticSeverity::WARNING),
                        source: Some("mui".to_string()),
                        message: format!("import path not found: {}", imp.path),
                        ..Default::default()
                    });
                }
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
                name: "mui-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        ".".to_string(),
                        ":".to_string(),
                        "(".to_string(),
                    ]),
                    ..Default::default()
                }),
                document_symbol_provider: Some(OneOf::Left(true)),
                color_provider: Some(ColorProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "mui-lsp ready")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        self.docs.open(uri.clone(), params.text_document.text);
        self.refresh_diagnostics(&uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        if let Some(change) = params.content_changes.into_iter().last() {
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
        let Some(doc) = self.docs.get(&params.text_document.uri) else {
            return Ok(None);
        };
        let mut symbols: Vec<DocumentSymbol> = Vec::new();

        if let Some(app) = &doc.parsed.app {
            let range = doc.span_to_range(app.span);
            symbols.push(symbol(
                app.name.clone().unwrap_or_else(|| "App".to_string()),
                Some("app".to_string()),
                SymbolKind::NAMESPACE,
                range,
            ));
        }
        for imp in &doc.parsed.imports {
            let range = doc.span_to_range(imp.span);
            symbols.push(symbol(
                format!("import {}", imp.names.join(", ")),
                Some(imp.path.clone()),
                SymbolKind::MODULE,
                range,
            ));
        }
        for view in &doc.parsed.views {
            let range = doc.span_to_range(view.span);
            let detail = if view.params.is_empty() {
                None
            } else {
                Some(format!(
                    "({})",
                    view.params
                        .iter()
                        .map(|p| p.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            };
            symbols.push(symbol(
                view.name.clone(),
                detail,
                SymbolKind::FUNCTION,
                range,
            ));
        }
        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let Some(doc) = self.docs.get(uri) else {
            return Ok(None);
        };
        let Some(word) = word_at(&doc, pos) else {
            return Ok(None);
        };

        // 1) Widget.
        if let Some(w) = catalog::widget(&word) {
            return Ok(Some(md_hover(format!(
                "**{}**{}\n\n{}",
                w.name,
                if w.container { " · container" } else { "" },
                w.doc
            ))));
        }
        // 2) Enum type.
        if let Some(members) = catalog::enum_members(&word) {
            return Ok(Some(md_hover(format!(
                "**{} enum**\n\nMembers: {}",
                word,
                members
                    .iter()
                    .map(|m| format!("`{m}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))));
        }
        // 3) Keyword.
        if let Some((_, detail, doc_)) = catalog::MUI_KEYWORDS.iter().find(|(k, ..)| *k == word) {
            return Ok(Some(md_hover(format!("```mui\n{detail}\n```\n\n{doc_}"))));
        }
        // 4) Prop.
        if let Some(d) = catalog::prop_doc(&word) {
            return Ok(Some(md_hover(format!("**{word}** (prop)\n\n{d}"))));
        }
        // 5) Imported component.
        for imp in &doc.parsed.imports {
            if imp.kind == ImportKind::Mui && imp.names.iter().any(|n| n == &word) {
                return Ok(Some(md_hover(format!(
                    "**{word}** — component imported from `{}`",
                    imp.path
                ))));
            }
        }
        // 6) Component id (declared via `id: varname` on an element).
        let ids = collect_ids(&doc.parsed);
        if let Some(widget_name) = ids.get(&word) {
            let props_summary = catalog::widget(widget_name)
                .map(|w| {
                    let mut ps: Vec<&str> = w.props.to_vec();
                    ps.truncate(6);
                    ps.iter()
                        .map(|p| format!("`{p}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            return Ok(Some(md_hover(format!(
                "**{word}** — `{widget_name}` widget reference (via `id: {word}`)\n\nProps: {props_summary}…"
            ))));
        }
        Ok(None)
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let Some(doc) = self.docs.get(uri) else {
            return Ok(None);
        };
        let byte = doc.position_to_byte(pos);
        let full = doc.full_text();
        let before = &full[..byte.min(full.len())];

        // a) `Ident.` completion — enum members or component-id props.
        if let Some(ident) = enum_dot_before(before) {
            if let Some(members) = catalog::enum_members(&ident) {
                let items = members
                    .iter()
                    .map(|m| simple_item(m, CompletionItemKind::ENUM_MEMBER, ""))
                    .collect();
                return Ok(Some(CompletionResponse::Array(items)));
            }
            // Component id declared via `id: varname` → offer that widget's props.
            let ids = collect_ids(&doc.parsed);
            if let Some(widget_name) = ids.get(&ident) {
                let mut items: Vec<CompletionItem> = Vec::new();
                if let Some(w) = catalog::widget(widget_name) {
                    for p in w.props {
                        items.push(prop_item(p));
                    }
                }
                for p in catalog::COMMON_PROPS {
                    items.push(prop_item(p));
                }
                return Ok(Some(CompletionResponse::Array(items)));
            }
        }

        // b) Inside an element's `( ... )` arg list.
        if let Some(ctx) = arg_context(before) {
            match ctx.prop {
                // value position: `prop: |`
                Some(prop) => {
                    return Ok(Some(CompletionResponse::Array(value_items(&prop))));
                }
                // prop-name position.
                None => {
                    let w = catalog::widget(&ctx.element);
                    let mut items: Vec<CompletionItem> = Vec::new();
                    if let Some(w) = w {
                        for p in w.props {
                            items.push(prop_item(p));
                        }
                    }
                    for p in catalog::COMMON_PROPS {
                        items.push(prop_item(p));
                    }
                    return Ok(Some(CompletionResponse::Array(items)));
                }
            }
        }

        // c) Inside `App() { ... }` → app fields.
        if enclosing_block_owner(before)
            .as_deref()
            .is_some_and(|o| o.eq_ignore_ascii_case("app"))
        {
            let items = catalog::APP_FIELDS
                .iter()
                .map(|(f, d)| {
                    let mut it = simple_item(f, CompletionItemKind::FIELD, d);
                    it.insert_text = Some(format!("{f}: "));
                    it
                })
                .collect();
            return Ok(Some(CompletionResponse::Array(items)));
        }

        // d) Element / statement position: widgets + imported components + kw.
        let mut items: Vec<CompletionItem> = Vec::new();
        for w in catalog::WIDGETS {
            items.push(widget_item(w));
        }
        for imp in &doc.parsed.imports {
            if imp.kind == ImportKind::Mui {
                for n in &imp.names {
                    let mut it = simple_item(
                        n,
                        CompletionItemKind::CLASS,
                        &format!("component from {}", imp.path),
                    );
                    it.insert_text = Some(format!("{n}($0)"));
                    it.insert_text_format = Some(InsertTextFormat::SNIPPET);
                    items.push(it);
                }
            }
        }
        for (kw, detail, d) in catalog::MUI_KEYWORDS {
            let mut it = simple_item(kw, CompletionItemKind::KEYWORD, d);
            it.detail = Some((*detail).to_string());
            items.push(it);
        }
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let Some(doc) = self.docs.get(uri) else {
            return Ok(None);
        };
        let Some(word) = word_at(&doc, pos) else {
            return Ok(None);
        };
        // Resolve relative to the current file's directory.
        let Ok(file_path) = uri.to_file_path() else {
            return Ok(None);
        };
        let dir = file_path.parent().map(PathBuf::from).unwrap_or_default();

        // The word is either an imported name (jump to the file) or it sits on
        // an import line (jump to the path).
        let target_path = doc.parsed.imports.iter().find_map(|imp| {
            let on_this_import =
                imp.names.iter().any(|n| n == &word) || imp.path.contains(&word) || word == "from";
            if on_this_import {
                let p = if std::path::Path::new(&imp.path).is_absolute() {
                    PathBuf::from(&imp.path)
                } else {
                    dir.join(&imp.path)
                };
                Some(p)
            } else {
                None
            }
        });

        let Some(path) = target_path else {
            return Ok(None);
        };
        let Ok(target_uri) = Url::from_file_path(&path) else {
            return Ok(None);
        };
        let loc = Location {
            uri: target_uri,
            range: Range::new(Position::new(0, 0), Position::new(0, 0)),
        };
        Ok(Some(GotoDefinitionResponse::Scalar(loc)))
    }

    async fn document_color(&self, params: DocumentColorParams) -> Result<Vec<ColorInformation>> {
        let Some(doc) = self.docs.get(&params.text_document.uri) else {
            return Ok(Vec::new());
        };
        Ok(scan_colors(&doc))
    }

    async fn color_presentation(
        &self,
        params: ColorPresentationParams,
    ) -> Result<Vec<ColorPresentation>> {
        let c = params.color;
        let (r, g, b) = (
            (c.red * 255.0).round() as u8,
            (c.green * 255.0).round() as u8,
            (c.blue * 255.0).round() as u8,
        );
        let label = if c.alpha >= 1.0 {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            format!(
                "#{r:02x}{g:02x}{b:02x}{:02x}",
                (c.alpha * 255.0).round() as u8
            )
        };
        Ok(vec![ColorPresentation {
            label,
            ..Default::default()
        }])
    }
}

// ---- completion-item builders ----

fn simple_item(label: &str, kind: CompletionItemKind, detail: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail: (!detail.is_empty()).then(|| detail.to_string()),
        ..Default::default()
    }
}

fn widget_item(w: &catalog::Widget) -> CompletionItem {
    let snippet = if w.positional {
        format!("{}($1)$0", w.name)
    } else if w.container {
        format!("{}($1) {{\n\t$0\n}}", w.name)
    } else {
        format!("{}($0)", w.name)
    };
    CompletionItem {
        label: w.name.to_string(),
        kind: Some(CompletionItemKind::CLASS),
        detail: Some(w.doc.to_string()),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: w.doc.to_string(),
        })),
        insert_text: Some(snippet),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..Default::default()
    }
}

fn prop_item(name: &str) -> CompletionItem {
    let mut it = simple_item(
        name,
        CompletionItemKind::PROPERTY,
        catalog::prop_doc(name).unwrap_or(""),
    );
    it.insert_text = Some(format!("{name}: "));
    it
}

/// Completions for a value position `prop: |`.
fn value_items(prop: &str) -> Vec<CompletionItem> {
    // Explicit value lists first.
    if let Some(vals) = catalog::prop_values(prop) {
        return vals
            .iter()
            .map(|v| simple_item(v, CompletionItemKind::VALUE, ""))
            .collect();
    }
    // Enum-typed props.
    let enum_for = match prop {
        "fillMode" => Some("FillMode"),
        "fontStyle" => Some("FontStyle"),
        "wrap" => Some("WrapMode"),
        _ => None,
    };
    if let Some(en) = enum_for {
        if let Some(members) = catalog::enum_members(en) {
            return members
                .iter()
                .map(|m| simple_item(&format!("{en}.{m}"), CompletionItemKind::ENUM_MEMBER, ""))
                .collect();
        }
    }
    // Color-typed props → handy literals.
    if matches!(
        prop,
        "color"
            | "fill"
            | "background"
            | "bg"
            | "borderColor"
            | "tint"
            | "textColor"
            | "selectionColor"
    ) {
        return vec![
            snippet_item("#${1:rrggbb}", "hex color"),
            snippet_item("rgba(${1:0}, ${2:0}, ${3:0}, ${4:1})", "rgba color"),
        ];
    }
    // Boolean props.
    if matches!(
        prop,
        "value" | "checked" | "visible" | "indeterminate" | "open"
    ) {
        return vec![
            simple_item("true", CompletionItemKind::VALUE, ""),
            simple_item("false", CompletionItemKind::VALUE, ""),
        ];
    }
    Vec::new()
}

fn snippet_item(snippet: &str, detail: &str) -> CompletionItem {
    CompletionItem {
        label: snippet.split("${").next().unwrap_or(snippet).to_string(),
        kind: Some(CompletionItemKind::VALUE),
        detail: Some(detail.to_string()),
        insert_text: Some(snippet.to_string()),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..Default::default()
    }
}

fn symbol(name: String, detail: Option<String>, kind: SymbolKind, range: Range) -> DocumentSymbol {
    #[allow(deprecated)]
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
}

fn md_hover(value: String) -> Hover {
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: None,
    }
}

// ---- context detection (operates on the text before the cursor) ----

/// The identifier under `pos`, if any.
fn word_at(doc: &Document, pos: Position) -> Option<String> {
    let byte = doc.position_to_byte(pos);
    let text = doc.full_text();
    let bytes = text.as_bytes();
    let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut start = byte;
    while start > 0 && is_word(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = byte;
    while end < bytes.len() && is_word(bytes[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    Some(text[start..end].to_string())
}

/// When the text immediately before the cursor is `Ident.partial`, returns the
/// `Ident` (so `FillMode.Co|` → `FillMode`).
fn enum_dot_before(before: &str) -> Option<String> {
    let trimmed = before.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_');
    let rest = trimmed.strip_suffix('.')?;
    let ident: String = rest
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    (!ident.is_empty()).then_some(ident)
}

struct ArgCtx {
    element: String,
    /// `Some(prop)` when the cursor is in the value position `prop: |`.
    prop: Option<String>,
}

/// If the cursor sits inside an element's `( ... )` arg list, returns the
/// element name and (when in a value position) the prop being assigned.
fn arg_context(before: &str) -> Option<ArgCtx> {
    let bytes = before.as_bytes();
    let mut i = before.len();
    let mut depth = 0i32;
    let mut open = None;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b')' => depth += 1,
            b'(' => {
                if depth == 0 {
                    open = Some(i);
                    break;
                }
                depth -= 1;
            }
            b'{' | b'}' if depth == 0 => return None, // block boundary, not an arg list
            _ => {}
        }
    }
    let open = open?;
    // Element name: the identifier ending just before `(`.
    let pre = &before[..open];
    let name: String = pre
        .trim_end()
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if name.is_empty() {
        return None;
    }
    // Current argument = text after the last top-level `,` within the arg list.
    let seg = &before[open + 1..];
    let mut d = 0i32;
    let mut arg_start = 0usize;
    for (idx, c) in seg.char_indices() {
        match c {
            '(' | '[' | '{' => d += 1,
            ')' | ']' | '}' => d -= 1,
            ',' if d == 0 => arg_start = idx + 1,
            _ => {}
        }
    }
    let cur = &seg[arg_start..];
    // Value position when the current arg contains a top-level `:` that isn't
    // part of `::`.
    let prop = top_level_colon_prop(cur);
    Some(ArgCtx {
        element: name,
        prop,
    })
}

/// In `cur` (one argument's text), if there's a `name :` separator, return the
/// `name`; this means the cursor is typing the value.
fn top_level_colon_prop(cur: &str) -> Option<String> {
    let chars: Vec<char> = cur.chars().collect();
    let mut d = 0i32;
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' | '{' => d += 1,
            ')' | ']' | '}' => d -= 1,
            ':' if d == 0 => {
                // Skip `::`.
                if chars.get(i + 1) == Some(&':') || (i > 0 && chars[i - 1] == ':') {
                    i += 1;
                    continue;
                }
                let name: String = cur[..byte_index(cur, i)].trim().to_string();
                return (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
                    .then_some(name);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn byte_index(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

/// The identifier that owns the nearest enclosing unclosed `{` (e.g. `App` for
/// the body of `App() { | }`, or a `view` name). Returns the token right before
/// the brace, skipping a trailing `()` and whitespace.
fn enclosing_block_owner(before: &str) -> Option<String> {
    let bytes = before.as_bytes();
    let mut i = before.len();
    let mut depth = 0i32;
    let mut brace = None;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b'}' => depth += 1,
            b'{' => {
                if depth == 0 {
                    brace = Some(i);
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    let brace = brace?;
    let mut pre = before[..brace].trim_end();
    // Skip an empty `()` (the `App()` form).
    if pre.ends_with(')') {
        if let Some(p) = pre.rfind('(') {
            pre = pre[..p].trim_end();
        }
    }
    let ident: String = pre
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    (!ident.is_empty()).then_some(ident)
}

// ---- color scanning (document_color) ----

// ---- component id registry ----

/// Walk the parsed document and collect every `id: <ident>` prop found on any
/// element, mapping the identifier to the widget type name.
fn collect_ids(doc: &mui_syntax::ast::Document) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for view in &doc.views {
        for node in &view.body {
            collect_ids_from_node(node, &mut map);
        }
    }
    map
}

fn collect_ids_from_node(node: &Node, map: &mut HashMap<String, String>) {
    match node {
        Node::Element(elem) => {
            for prop in &elem.props {
                if prop.name == "id" {
                    if let PropValue::Expr(e) = &prop.value {
                        if let ExprKind::Ident(id_name) = &e.kind {
                            map.insert(id_name.clone(), elem.name.clone());
                        }
                    }
                }
            }
            for child in &elem.children {
                collect_ids_from_node(child, map);
            }
        }
        Node::If { then, els, .. } => {
            for n in then {
                collect_ids_from_node(n, map);
            }
            if let Some(els_nodes) = els {
                for n in els_nodes {
                    collect_ids_from_node(n, map);
                }
            }
        }
        Node::For { body, .. } => {
            for n in body {
                collect_ids_from_node(n, map);
            }
        }
        Node::Match { arms, .. } => {
            for arm in arms {
                for n in &arm.body {
                    collect_ids_from_node(n, map);
                }
            }
        }
        Node::Let { .. } | Node::Effect { .. } | Node::Expr(_) => {}
    }
}

fn scan_colors(doc: &Document) -> Vec<ColorInformation> {
    let text = doc.full_text();
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        // Skip line comments so `// #fff` isn't a swatch.
        if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'#' {
            let start = i;
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j] as char).is_ascii_hexdigit() {
                j += 1;
            }
            let len = j - (i + 1);
            if (len == 6 || len == 8)
                && !bytes
                    .get(j)
                    .is_some_and(|b| (*b as char).is_ascii_hexdigit())
            {
                if let Some(color) = parse_hex(&text[start + 1..j]) {
                    out.push(color_info(doc, start, j, color));
                }
                i = j;
                continue;
            }
        }
        // rgba( ... ) / rgb( ... )
        if (text[i..].starts_with("rgba(") || text[i..].starts_with("rgb("))
            && (i == 0 || !(bytes[i - 1] as char).is_alphanumeric())
        {
            if let Some(close) = text[i..].find(')') {
                let end = i + close + 1;
                if let Some(color) = parse_rgb_call(&text[i..end]) {
                    out.push(color_info(doc, i, end, color));
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn color_info(doc: &Document, start: usize, end: usize, color: Color) -> ColorInformation {
    use copper_syntax::ast::Span;
    ColorInformation {
        range: doc.span_to_range(Span::new(start as u32, end as u32)),
        color,
    }
}

fn parse_hex(hex: &str) -> Option<Color> {
    let h = hex.trim();
    let byte = |s: &str| u8::from_str_radix(s, 16).ok();
    let (r, g, b, a) = match h.len() {
        6 => (byte(&h[0..2])?, byte(&h[2..4])?, byte(&h[4..6])?, 255),
        8 => (
            byte(&h[0..2])?,
            byte(&h[2..4])?,
            byte(&h[4..6])?,
            byte(&h[6..8])?,
        ),
        _ => return None,
    };
    Some(Color {
        red: r as f32 / 255.0,
        green: g as f32 / 255.0,
        blue: b as f32 / 255.0,
        alpha: a as f32 / 255.0,
    })
}

fn parse_rgb_call(s: &str) -> Option<Color> {
    let inner = s
        .trim()
        .trim_start_matches("rgba")
        .trim_start_matches("rgb")
        .trim()
        .strip_prefix('(')?
        .strip_suffix(')')?;
    let parts: Vec<f32> = inner
        .split(',')
        .filter_map(|p| p.trim().parse::<f32>().ok())
        .collect();
    if parts.len() < 3 {
        return None;
    }
    let norm = |v: f32| (v / 255.0).clamp(0.0, 1.0);
    Some(Color {
        red: norm(parts[0]),
        green: norm(parts[1]),
        blue: norm(parts[2]),
        alpha: parts.get(3).copied().unwrap_or(1.0).clamp(0.0, 1.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arg_context_prop_name_position() {
        let ctx = arg_context("  Button(").expect("in args");
        assert_eq!(ctx.element, "Button");
        assert!(ctx.prop.is_none());
    }

    #[test]
    fn arg_context_value_position() {
        let ctx = arg_context("  Button(\"Hi\", radius: ").expect("in args");
        assert_eq!(ctx.element, "Button");
        assert_eq!(ctx.prop.as_deref(), Some("radius"));
    }

    #[test]
    fn arg_context_value_after_first_prop() {
        let ctx = arg_context("Stack(orientation: vertical, gap: ").expect("in args");
        assert_eq!(ctx.element, "Stack");
        assert_eq!(ctx.prop.as_deref(), Some("gap"));
    }

    #[test]
    fn arg_context_closed_args_is_none() {
        // Cursor sits after a fully-closed arg list → not in args.
        assert!(arg_context("Text(\"x\") ").is_none());
    }

    #[test]
    fn enum_dot_detected() {
        assert_eq!(
            enum_dot_before("Image(\"a\", fillMode: FillMode."),
            Some("FillMode".to_string())
        );
        assert_eq!(
            enum_dot_before("fillMode: FillMode.Co"),
            Some("FillMode".to_string())
        );
    }

    #[test]
    fn block_owner_app_and_nested() {
        assert_eq!(enclosing_block_owner("App() {\n  ").as_deref(), Some("App"));
        assert_eq!(
            enclosing_block_owner("view Main() {\n  Stack() {\n    ").as_deref(),
            Some("Stack")
        );
    }

    #[test]
    fn hex_and_rgb_parse() {
        assert!(parse_hex("ff8000").is_some());
        assert!(parse_hex("ff8000ff").is_some());
        assert!(parse_rgb_call("rgba(0, 0, 0, 0.5)").is_some());
        assert!(parse_rgb_call("rgb(10, 20, 30)").is_some());
    }
}
