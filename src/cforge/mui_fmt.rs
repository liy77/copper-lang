//! `cforge format` — a pretty-printer for MUI (`.mui` / `.crm`) sources.
//!
//! The MUI parser drops comments before building the AST, so an AST-based emitter
//! would silently delete every `// …`. Instead this formatter tokenizes the
//! source itself (a tiny scanner that keeps comments + strings verbatim), builds
//! a shallow node tree, and re-emits it with a "fit-or-break" rule:
//!
//!   * an element whose call `Name(prop: v, …)` fits in `MAX_WIDTH` stays on one
//!     line; if it doesn't, every argument is broken onto its own line, indented;
//!   * `{ … }` child / view bodies always break into indented statements;
//!   * short inline handlers (`onClick: { x += 1 }`) stay inline, long ones break.
//!
//! Comments and string literals are reproduced byte-for-byte, and the output is
//! idempotent (`format(format(x)) == format(x)`).

use std::fs;
use std::path::Path;

use crate::cforge::pretty;

const INDENT: &str = "  "; // two spaces per level
const MAX_WIDTH: usize = 100;

/// True for a path this formatter handles (`.mui` or `.crm`).
pub fn is_formattable(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("mui") | Some("crm")
    )
}

// ── Scanner ─────────────────────────────────────────────────────────────────

enum Tok {
    Word(String),    // identifier / number / operator run (e.g. `gap:`, `+=`, `#fff`)
    Str(String),     // "…" literal, verbatim (incl. interpolation)
    Comment(String), // `// …` or `/* … */`, verbatim
    Open(u8),        // `{` `(` `[`
    Close(u8),       // `}` `)` `]`
    Comma,
    Nl(usize), // a run of newlines (count) — drives blank-line + own-line-comment detection
}

fn scan(src: &str) -> Vec<Tok> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'\n' => {
                let mut n = 0;
                while i < b.len() && (b[i] == b'\n' || b[i] == b'\r') {
                    if b[i] == b'\n' {
                        n += 1;
                    }
                    i += 1;
                }
                out.push(Tok::Nl(n));
            }
            b' ' | b'\t' => i += 1,
            b'"' => {
                let start = i;
                i += 1;
                while i < b.len() {
                    match b[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
                out.push(Tok::Str(src[start..i.min(b.len())].to_string()));
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let start = i;
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                out.push(Tok::Comment(src[start..i].trim_end().to_string()));
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let start = i;
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(b.len());
                out.push(Tok::Comment(src[start..i].to_string()));
            }
            b'{' | b'(' | b'[' => {
                out.push(Tok::Open(c));
                i += 1;
            }
            b'}' | b')' | b']' => {
                out.push(Tok::Close(c));
                i += 1;
            }
            b',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            _ => {
                let start = i;
                while i < b.len() {
                    let d = b[i];
                    if d.is_ascii_whitespace()
                        || matches!(d, b'{' | b'}' | b'(' | b')' | b'[' | b']' | b',' | b'"')
                        || (d == b'/' && matches!(b.get(i + 1), Some(&b'/') | Some(&b'*')))
                    {
                        break;
                    }
                    i += 1;
                }
                out.push(Tok::Word(src[start..i].to_string()));
            }
        }
    }
    out
}

// ── Node tree ───────────────────────────────────────────────────────────────

enum Kind {
    Word(String),
    Str(String),
    Comment(String),
    Group(u8, Vec<Node>), // open byte + children
    Comma,
}

struct Node {
    kind: Kind,
    nl_before: bool,    // a newline preceded this node (→ statement boundary)
    blank_before: bool, // a blank line preceded this node
    own_comment: bool,  // a comment that started its own line
}

fn closer(open: u8) -> u8 {
    match open {
        b'{' => b'}',
        b'(' => b')',
        b'[' => b']',
        _ => 0,
    }
}

fn build(toks: &[Tok], i: &mut usize, close: u8) -> Vec<Node> {
    let mut out = Vec::new();
    let mut nl = 0usize;
    while *i < toks.len() {
        match &toks[*i] {
            Tok::Nl(n) => {
                nl += n;
                *i += 1;
            }
            Tok::Close(c) => {
                if *c == close {
                    *i += 1;
                    return out;
                }
                *i += 1; // stray closer — drop
            }
            Tok::Open(o) => {
                let open = *o;
                *i += 1;
                let children = build(toks, i, closer(open));
                out.push(mk(Kind::Group(open, children), nl));
                nl = 0;
            }
            Tok::Comma => {
                out.push(mk(Kind::Comma, nl));
                nl = 0;
                *i += 1;
            }
            Tok::Word(w) => {
                out.push(mk(Kind::Word(w.clone()), nl));
                nl = 0;
                *i += 1;
            }
            Tok::Str(s) => {
                out.push(mk(Kind::Str(s.clone()), nl));
                nl = 0;
                *i += 1;
            }
            Tok::Comment(c) => {
                let mut n = mk(Kind::Comment(c.clone()), nl);
                n.own_comment = nl > 0;
                out.push(n);
                nl = 0;
                *i += 1;
            }
        }
    }
    out
}

fn mk(kind: Kind, nl: usize) -> Node {
    Node {
        kind,
        nl_before: nl > 0,
        blank_before: nl >= 2,
        own_comment: false,
    }
}

// ── Emit ────────────────────────────────────────────────────────────────────

/// Pretty-print a MUI source string. Pure + idempotent.
pub fn format_mui(src: &str) -> String {
    let toks = scan(src);
    let mut i = 0;
    let nodes = build(&toks, &mut i, 0);
    let mut out = String::with_capacity(src.len() + 64);
    emit_block_body(&mut out, &nodes, 0);
    // exactly one trailing newline
    while out.ends_with('\n') {
        out.pop();
    }
    out.push('\n');
    out
}

fn ind(n: usize) -> String {
    INDENT.repeat(n)
}

fn line(out: &mut String, s: &str) {
    out.push_str(s.trim_end());
    out.push('\n');
}

/// Emit a sequence of statements (the document, or a `{ … }` body).
fn emit_block_body(out: &mut String, nodes: &[Node], indent: usize) {
    let mut k = 0;
    let mut first = true;
    while k < nodes.len() {
        if !first && nodes[k].blank_before {
            out.push('\n');
        }
        // Own-line comment → its own line.
        if let Kind::Comment(c) = &nodes[k].kind {
            if nodes[k].own_comment || first {
                line(out, &format!("{}{}", ind(indent), c));
                k += 1;
                first = false;
                continue;
            }
        }
        // Gather one statement: this node plus following nodes on the same source
        // line (nl_before == false). Wrapped `(…)`/`{…}` are single Group nodes,
        // so a statement is its header line + any attached groups.
        let start = k;
        k += 1;
        while k < nodes.len() && !nodes[k].nl_before {
            k += 1;
        }
        let stmt: Vec<&Node> = nodes[start..k].iter().collect();
        emit_statement(out, &stmt, indent);
        first = false;
    }
}

fn has_content(s: &str, indent: usize) -> bool {
    s.len() > ind(indent).len()
}

/// Should there be a space before a `(`/`[`/`{` group, given the previous node?
/// No space after a call target / index target (`signal`, `arr`); a space after
/// an operator-ish word (`=`, `onClick:`) or a string.
fn space_before_group(prev: Option<&Kind>, open: u8) -> bool {
    // `{` (handler / block / import list) always takes a leading space; `(`/`[`
    // attach to a call/index target (`signal(`, `arr[`) with no space.
    if open == b'{' {
        return true;
    }
    match prev {
        Some(Kind::Word(w)) => !w
            .chars()
            .last()
            .map(|c| c.is_alphanumeric() || c == '_')
            .unwrap_or(false),
        Some(Kind::Str(_)) | Some(Kind::Group(..)) => true,
        _ => false,
    }
}

fn emit_statement(out: &mut String, nodes: &[&Node], indent: usize) {
    // `import { … } from "…"` stays on one line.
    if let Some(Kind::Word(w)) = nodes.first().map(|n| &n.kind) {
        if w == "import" {
            line(out, &format!("{}{}", ind(indent), inline_nodes(nodes)));
            return;
        }
    }
    let mut buf = ind(indent);
    let mut prev: Option<&Kind> = None;
    for n in nodes {
        match &n.kind {
            Kind::Group(b'{', ch) => {
                // Statement-level brace = a block body (always breaks).
                if ch.is_empty() {
                    if has_content(&buf, indent) {
                        buf.push(' ');
                    }
                    buf.push_str("{}");
                } else {
                    if has_content(&buf, indent) {
                        buf.push(' ');
                    }
                    buf.push('{');
                    line(out, &buf);
                    emit_block_body(out, ch, indent + 1);
                    buf = format!("{}}}", ind(indent));
                }
            }
            Kind::Word(w) if w == "else" => buf.push_str(" else"),
            Kind::Word(w) => {
                if has_content(&buf, indent) {
                    buf.push(' ');
                }
                buf.push_str(w);
            }
            Kind::Str(s) => {
                if has_content(&buf, indent) {
                    buf.push(' ');
                }
                buf.push_str(s);
            }
            Kind::Group(open, ch) => {
                let inline = inline_group(*open, ch);
                let space = space_before_group(prev, *open) && has_content(&buf, indent);
                let projected = buf.len() + usize::from(space) + inline.len();
                if (projected <= MAX_WIDTH && !group_has_comment(ch)) || !can_break(*open, ch) {
                    if space {
                        buf.push(' ');
                    }
                    buf.push_str(&inline);
                } else {
                    if space {
                        buf.push(' ');
                    }
                    buf.push(*open as char);
                    line(out, &buf);
                    emit_arg_list(out, ch, indent + 1);
                    buf = format!("{}{}", ind(indent), closer(*open) as char);
                }
            }
            Kind::Comment(c) => {
                if has_content(&buf, indent) {
                    buf.push_str("  ");
                }
                buf.push_str(c);
                line(out, &buf);
                buf = ind(indent);
            }
            Kind::Comma => buf.push(','),
        }
        prev = Some(&n.kind);
    }
    if has_content(&buf, indent) {
        line(out, &buf);
    }
}

/// A `(`/`[` group is worth breaking only if it actually holds a comma-separated
/// list (or a comment); a single long argument breaking onto its own line is
/// pointless, so we keep it inline.
fn can_break(open: u8, ch: &[Node]) -> bool {
    if open == b'{' {
        return false;
    }
    ch.iter()
        .any(|n| matches!(n.kind, Kind::Comma) || matches!(&n.kind, Kind::Comment(_)))
}

fn group_has_comment(ch: &[Node]) -> bool {
    ch.iter().any(|n| matches!(n.kind, Kind::Comment(_)))
}

/// Emit a `(`/`[` body broken one argument per line.
fn emit_arg_list(out: &mut String, ch: &[Node], indent: usize) {
    let mut seg: Vec<&Node> = Vec::new();
    let mut idx = 0;
    while idx < ch.len() {
        let node = &ch[idx];
        match &node.kind {
            Kind::Comma => {
                if !seg.is_empty() {
                    emit_arg(out, &seg, indent, true);
                    seg.clear();
                }
            }
            Kind::Comment(c) if node.own_comment => {
                if !seg.is_empty() {
                    let comma = trailing_comma(ch, idx);
                    emit_arg(out, &seg, indent, comma);
                    seg.clear();
                }
                line(out, &format!("{}{}", ind(indent), c));
            }
            _ => seg.push(node),
        }
        idx += 1;
    }
    if !seg.is_empty() {
        emit_arg(out, &seg, indent, false);
    }
}

/// True if a comma appears after position `idx` at this group level.
fn trailing_comma(ch: &[Node], idx: usize) -> bool {
    ch[idx + 1..].iter().any(|n| matches!(n.kind, Kind::Comma))
}

/// Emit a single argument (a value, a `prop: value`, or a `prop: { handler }`).
/// Inline when it fits; otherwise break its handler block.
fn emit_arg(out: &mut String, nodes: &[&Node], indent: usize, comma: bool) {
    let inline = inline_nodes(nodes);
    let tail = if comma { "," } else { "" };
    let oneline = format!("{}{}{}", ind(indent), inline, tail);
    let breaks = nodes.iter().any(|n| matches!(n.kind, Kind::Comment(_)));
    if oneline.len() <= MAX_WIDTH && !breaks {
        line(out, &oneline);
        return;
    }

    // Break: emit the header up to the handler `{`, the block body, then `} ,`.
    let mut buf = ind(indent);
    let mut prev: Option<&Kind> = None;
    for n in nodes {
        match &n.kind {
            Kind::Group(b'{', ch) => {
                if space_before_group(prev, b'{') && has_content(&buf, indent) {
                    buf.push(' ');
                }
                buf.push('{');
                // keep a closure param (`|v|`) on the opening line
                let mut body_from = 0;
                if let Some(Kind::Word(w)) = ch.first().map(|c| &c.kind) {
                    if w.starts_with('|') {
                        buf.push(' ');
                        buf.push_str(w);
                        body_from = 1;
                    }
                }
                if ch.len() == body_from {
                    buf.push_str(" }");
                    buf.push_str(tail);
                } else {
                    line(out, &buf);
                    emit_block_body(out, &ch[body_from..], indent + 1);
                    buf = format!("{}}}{}", ind(indent), tail);
                }
            }
            Kind::Word(w) => {
                if has_content(&buf, indent) {
                    buf.push(' ');
                }
                buf.push_str(w);
            }
            Kind::Str(s) => {
                if has_content(&buf, indent) {
                    buf.push(' ');
                }
                buf.push_str(s);
            }
            Kind::Group(open, c2) => {
                let inl = inline_group(*open, c2);
                if space_before_group(prev, *open) && has_content(&buf, indent) {
                    buf.push(' ');
                }
                buf.push_str(&inl);
            }
            Kind::Comment(c) => {
                if has_content(&buf, indent) {
                    buf.push_str("  ");
                }
                buf.push_str(c);
                line(out, &buf);
                buf = ind(indent);
            }
            Kind::Comma => buf.push(','),
        }
        prev = Some(&n.kind);
    }
    if has_content(&buf, indent) {
        line(out, &buf);
    }
}

/// Render a node sequence on a single line (no breaks). Used for fit-measuring
/// and for everything that stays inline.
fn inline_nodes(nodes: &[&Node]) -> String {
    let mut s = String::new();
    let mut prev: Option<&Kind> = None;
    for n in nodes {
        match &n.kind {
            Kind::Word(w) if w == "else" => s.push_str(" else"),
            Kind::Word(w) => {
                if !s.is_empty() {
                    s.push(' ');
                }
                s.push_str(w);
            }
            Kind::Str(t) => {
                if !s.is_empty() {
                    s.push(' ');
                }
                s.push_str(t);
            }
            Kind::Group(o, ch) => {
                if space_before_group(prev, *o) && !s.is_empty() {
                    s.push(' ');
                }
                s.push_str(&inline_group(*o, ch));
            }
            Kind::Comment(c) => {
                if !s.is_empty() {
                    s.push_str("  ");
                }
                s.push_str(c);
            }
            Kind::Comma => {
                while s.ends_with(' ') {
                    s.pop();
                }
                s.push_str(", ");
            }
        }
        prev = Some(&n.kind);
    }
    s
}

fn inline_group(open: u8, ch: &[Node]) -> String {
    if open == b'{' {
        if ch.is_empty() {
            return "{}".to_string();
        }
        let refs: Vec<&Node> = ch.iter().collect();
        return format!("{{ {} }}", inline_nodes(&refs));
    }
    // ( or [ — comma-separated
    let mut parts: Vec<String> = Vec::new();
    let mut seg: Vec<&Node> = Vec::new();
    for node in ch {
        if matches!(node.kind, Kind::Comma) {
            parts.push(inline_nodes(&seg));
            seg.clear();
        } else {
            seg.push(node);
        }
    }
    if !seg.is_empty() {
        parts.push(inline_nodes(&seg));
    }
    let inner = parts.join(", ");
    format!("{}{}{}", open as char, inner, closer(open) as char)
}

// ── CLI ─────────────────────────────────────────────────────────────────────

/// `cforge format <paths…>`: format each `.mui`/`.crm` file (recursing into
/// directories). `check` only reports which files would change (exit 1 if any);
/// `to_stdout` prints the formatted result instead of writing it.
pub fn format_command(paths: &[String], check: bool, to_stdout: bool) -> i32 {
    let mut files: Vec<String> = Vec::new();
    for p in paths {
        collect(Path::new(p), &mut files);
    }
    if files.is_empty() {
        pretty::warn("no .mui or .crm files found to format");
        return 0;
    }

    let mut changed = 0usize;
    let mut errors = 0usize;
    for f in &files {
        let src = match fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => {
                pretty::fail(&format!("{f}: {e}"));
                errors += 1;
                continue;
            }
        };
        let formatted = format_mui(&src);

        if to_stdout {
            print!("{formatted}");
            continue;
        }
        if formatted == src {
            pretty::step(&format!("{f} — already formatted"));
            continue;
        }
        changed += 1;
        if check {
            pretty::warn(&format!("{f} — would reformat"));
        } else if let Err(e) = fs::write(f, &formatted) {
            pretty::fail(&format!("{f}: {e}"));
            errors += 1;
        } else {
            pretty::ok(&format!("formatted {f}"));
        }
    }

    if to_stdout {
        return 0;
    }
    if errors > 0 {
        return 1;
    }
    if check && changed > 0 {
        pretty::warn(&format!("{changed} file(s) need formatting"));
        return 1;
    }
    if !check {
        pretty::ok(&format!(
            "{} file(s) formatted, {} already clean",
            changed,
            files.len() - changed
        ));
    }
    0
}

fn collect(path: &Path, out: &mut Vec<String>) {
    if path.is_file() {
        if let Some(s) = path.to_str() {
            if is_formattable(s) {
                out.push(s.to_string());
            }
        }
        return;
    }
    if path.is_dir() {
        for entry in walkdir::WalkDir::new(path)
            .follow_links(true)
            .into_iter()
            .filter_entry(|e| {
                !matches!(
                    e.file_name().to_str(),
                    Some("dist") | Some("target") | Some(".git") | Some("node_modules")
                )
            })
            .filter_map(Result::ok)
            .filter(|e| e.path().is_file())
        {
            if let Some(s) = entry.path().to_str() {
                if is_formattable(s) {
                    out.push(s.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::format_mui;

    #[test]
    fn breaks_long_prop_lists() {
        let src = "Stack(orientation: horizontal, gap: 0, align: center, background: #2b2d30, borderColor: #393b40, borderWidth: 1, height: 40) {\n  Text(\"x\")\n}\n";
        let out = format_mui(src);
        assert!(
            out.contains("Stack(\n  orientation: horizontal,\n"),
            "long arg list breaks one per line:\n{out}"
        );
        assert!(out.contains("\n) {\n"), "closer + block brace:\n{out}");
    }

    #[test]
    fn keeps_short_inline() {
        let out = format_mui("Text(\"hi\", size: 13, color: #548af7)\n");
        assert_eq!(out, "Text(\"hi\", size: 13, color: #548af7)\n");
    }

    #[test]
    fn idempotent() {
        let src = "view Demo() {\n  Stack(orientation: horizontal, gap: 0, align: center, background: #2b2d30, borderColor: #393b40, borderWidth: 1, height: 40) {\n    Button(\"Play\", width: 58, onClick: { play_clicked += 1 })\n  }\n}\n";
        let once = format_mui(src);
        assert_eq!(once, format_mui(&once), "idempotent:\n{once}");
    }

    #[test]
    fn preserves_comments_and_strings() {
        let out = format_mui("// header\nText(\"a:b  c\")   // trailing\n");
        assert!(out.contains("// header"));
        assert!(out.contains("\"a:b  c\""));
        assert!(out.contains("// trailing"));
    }

    #[test]
    fn import_stays_inline() {
        let out = format_mui("import { Foo } from \"./foo.mui\"\n");
        assert_eq!(out, "import { Foo } from \"./foo.mui\"\n");
    }

    #[test]
    fn keeps_double_colon() {
        let out = format_mui("Button(onClick: { a::b::c() })\n");
        assert!(out.contains("a::b::c"));
    }
}
