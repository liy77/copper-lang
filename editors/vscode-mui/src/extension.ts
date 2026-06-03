// MUI VSCode extension — colors, completion, hover tooltips, and diagnostics
// for .mui / .crm. Self-contained (no language server): highlighting comes
// from the TextMate grammar; this file adds the interactive language features.
//
//   1. DocumentColorProvider — swatches + picker for `#rrggbb` / `rgba()`.
//   2. CompletionItemProvider — elements, props, enum members, prop values,
//      Copper keywords / reactivity primitives.
//   3. HoverProvider — description bubbles for elements (Text, Button, …),
//      props (size, color, …), enums (FontStyle) and members, and keywords.
//   4. Diagnostics — bracket balance, unknown enum members, and unknown props
//      on known widgets.

import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import type { LanguageClient } from 'vscode-languageclient/node';
import * as lspClient from './client';
import {
  WIDGETS,
  COMMON_PROPS,
  ENUMS,
  ENUM_DOCS,
  PROP_DOCS,
  PROP_VALUES,
  WidgetDef,
  widgetByName,
  isKnownProp,
  keywordByLabel,
  allKeywords,
  loadCopperLexicon,
} from './catalog';

// Both languages are handled identically: `mui` (.mui markup) and `crm`
// (.crm = Copper + Mocida). They share the grammar and all providers — the
// only reason they're distinct languages is per-extension file icons.
const SELECTOR: vscode.DocumentSelector = [
  { scheme: 'file', language: 'mui' },
  { scheme: 'file', language: 'crm' },
];

const LANG_IDS = new Set(['mui', 'crm']);

let client: LanguageClient | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  // Load Copper keyword/type descriptions from the shared lexicon so they
  // match the Copper extension exactly. Canonical file lives one level up
  // (editors/copper-lexicon.json); fall back to the embedded copy if absent
  // (e.g. a standalone-packaged build).
  loadSharedCopperLexicon(context);

  // Prefer the real language server (mui-lsp). When it's available it owns
  // completion, hover, color, symbols, go-to-definition and diagnostics — so we
  // skip the in-process providers to avoid duplicates. If no server binary is
  // found, fall back to the built-in TypeScript providers below.
  client = await lspClient.start(context);
  if (client) {
    context.subscriptions.push({ dispose: () => void client?.stop() });
  } else {
    registerFallbackProviders(context);
  }

  context.subscriptions.push(
    vscode.commands.registerCommand('mui.restartLanguageServer', async () => {
      if (client) {
        await vscode.window.withProgress(
          {
            location: vscode.ProgressLocation.Notification,
            title: 'Restarting MUI Language Server…',
            cancellable: false,
          },
          async () => {
            await client!.stop();
            await client!.start();
          }
        );
      } else {
        vscode.window.showInformationMessage(
          'MUI: running in built-in mode (no mui-lsp binary found) — nothing to restart.'
        );
      }
    })
  );
}

/** In-process language features used when the `mui-lsp` server isn't available. */
function registerFallbackProviders(context: vscode.ExtensionContext): void {
  context.subscriptions.push(
    vscode.languages.registerColorProvider(SELECTOR, new MuiColorProvider()),
    vscode.languages.registerCompletionItemProvider(
      SELECTOR,
      new MuiCompletionProvider(),
      '.', // enum member access (FontStyle.)
      ':', // prop value position
      ' ' // new arg / new node
    ),
    vscode.languages.registerHoverProvider(SELECTOR, new MuiHoverProvider()),
    vscode.languages.registerDocumentSymbolProvider(SELECTOR, new MuiDocumentSymbolProvider()),
    vscode.languages.registerDefinitionProvider(SELECTOR, new MuiDefinitionProvider()),
    vscode.languages.registerSignatureHelpProvider(
      SELECTOR,
      new MuiSignatureHelpProvider(),
      '(',
      ','
    )
  );

  // ---- Diagnostics ----
  const diags = vscode.languages.createDiagnosticCollection('mui');
  context.subscriptions.push(diags);

  const refresh = (doc?: vscode.TextDocument) => {
    if (!doc || !LANG_IDS.has(doc.languageId)) return;
    diags.set(doc.uri, computeDiagnostics(doc));
  };

  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument(refresh),
    vscode.workspace.onDidChangeTextDocument((e) => refresh(e.document)),
    vscode.workspace.onDidCloseTextDocument((d) => diags.delete(d.uri)),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration('mui')) {
        vscode.workspace.textDocuments.forEach(refresh);
      }
    })
  );
  vscode.workspace.textDocuments.forEach(refresh);
}

export async function deactivate(): Promise<void> {
  await client?.stop();
}

/**
 * Read the canonical `editors/copper-lexicon.json` (the single source of truth
 * shared with copper-lsp) and feed it to the catalog. Tries a few locations so
 * it works both in the dev workspace (F5) and a packaged extension; on any
 * failure the catalog uses its embedded mirror, so this never throws.
 */
function loadSharedCopperLexicon(context: vscode.ExtensionContext): void {
  const candidates = [
    path.join(context.extensionPath, '..', 'copper-lexicon.json'), // editors/ (dev)
    path.join(context.extensionPath, 'out', 'copper-lexicon.json'), // bundled (.vsix)
    path.join(context.extensionPath, 'copper-lexicon.json'), // bundled (root)
  ];
  for (const file of candidates) {
    try {
      const raw = JSON.parse(fs.readFileSync(file, 'utf8'));
      loadCopperLexicon(raw);
      return;
    } catch {
      /* try next candidate */
    }
  }
  loadCopperLexicon(); // embedded fallback
}

// ===========================================================================
// Colors
// ===========================================================================

class MuiColorProvider implements vscode.DocumentColorProvider {
  private static readonly HEX = /#([0-9a-fA-F]{8}|[0-9a-fA-F]{6})\b/g;
  private static readonly RGB = /\brgba?\s*\(\s*([0-9.]+)\s*,\s*([0-9.]+)\s*,\s*([0-9.]+)\s*(?:,\s*([0-9.]+)\s*)?\)/g;

  provideDocumentColors(
    document: vscode.TextDocument
  ): vscode.ColorInformation[] {
    if (!vscode.workspace.getConfiguration('mui').get('colorDecorators', true)) {
      return [];
    }
    const out: vscode.ColorInformation[] = [];
    const text = document.getText();

    for (const m of text.matchAll(MuiColorProvider.HEX)) {
      const color = hexToColor(m[1]);
      if (!color) continue;
      const start = document.positionAt(m.index!);
      const end = document.positionAt(m.index! + m[0].length);
      out.push(new vscode.ColorInformation(new vscode.Range(start, end), color));
    }

    for (const m of text.matchAll(MuiColorProvider.RGB)) {
      const r = clamp01(parseFloat(m[1]) / 255);
      const g = clamp01(parseFloat(m[2]) / 255);
      const b = clamp01(parseFloat(m[3]) / 255);
      const a = m[4] !== undefined ? clamp01(parseFloat(m[4])) : 1;
      const start = document.positionAt(m.index!);
      const end = document.positionAt(m.index! + m[0].length);
      out.push(
        new vscode.ColorInformation(
          new vscode.Range(start, end),
          new vscode.Color(r, g, b, a)
        )
      );
    }
    return out;
  }

  provideColorPresentations(
    color: vscode.Color,
    context: { range: vscode.Range; document: vscode.TextDocument }
  ): vscode.ColorPresentation[] {
    const existing = context.document.getText(context.range);
    const hasAlpha = color.alpha < 1;
    if (/^rgba?\b/.test(existing)) {
      const r = Math.round(color.red * 255);
      const g = Math.round(color.green * 255);
      const b = Math.round(color.blue * 255);
      const label = hasAlpha
        ? `rgba(${r}, ${g}, ${b}, ${round(color.alpha)})`
        : `rgb(${r}, ${g}, ${b})`;
      return [new vscode.ColorPresentation(label)];
    }
    return [new vscode.ColorPresentation(colorToHex(color, hasAlpha))];
  }
}

function hexToColor(hex: string): vscode.Color | undefined {
  const h = hex.toLowerCase();
  if (h.length === 6 || h.length === 8) {
    const r = parseInt(h.slice(0, 2), 16) / 255;
    const g = parseInt(h.slice(2, 4), 16) / 255;
    const b = parseInt(h.slice(4, 6), 16) / 255;
    const a = h.length === 8 ? parseInt(h.slice(6, 8), 16) / 255 : 1;
    if ([r, g, b, a].some(Number.isNaN)) return undefined;
    return new vscode.Color(r, g, b, a);
  }
  return undefined;
}

function colorToHex(c: vscode.Color, withAlpha: boolean): string {
  const h = (n: number) =>
    Math.round(clamp01(n) * 255)
      .toString(16)
      .padStart(2, '0');
  const base = `#${h(c.red)}${h(c.green)}${h(c.blue)}`;
  return withAlpha ? `${base}${h(c.alpha)}` : base;
}

const clamp01 = (n: number) => Math.max(0, Math.min(1, n));
const round = (n: number) => Math.round(n * 100) / 100;

// ===========================================================================
// Completion
// ===========================================================================

class MuiCompletionProvider implements vscode.CompletionItemProvider {
  provideCompletionItems(
    document: vscode.TextDocument,
    position: vscode.Position
  ): vscode.CompletionItem[] {
    const line = document.lineAt(position.line).text;
    const upto = line.slice(0, position.character);

    // 1) Enum member access: `FontStyle.` → its members.
    const enumMatch = upto.match(/\b([A-Z][A-Za-z0-9_]*)\.\s*([A-Za-z0-9_]*)$/);
    if (enumMatch && ENUMS[enumMatch[1]]) {
      return ENUMS[enumMatch[1]].map((m) => {
        const it = new vscode.CompletionItem(m, vscode.CompletionItemKind.EnumMember);
        it.detail = `${enumMatch[1]}.${m}`;
        return it;
      });
    }

    // 1b) Component id.` access: `stack1.` → props for that widget type.
    const idDotMatch = upto.match(/\b([a-z_][A-Za-z0-9_]*)\.\s*([A-Za-z0-9_]*)$/);
    if (idDotMatch) {
      const compIds = collectComponentIds(document);
      const widgetName = compIds.get(idDotMatch[1]);
      if (widgetName) {
        const w = widgetByName(widgetName);
        const props = new Set<string>([...(w?.props ?? []), ...COMMON_PROPS]);
        return [...props].map((p) => {
          const it = new vscode.CompletionItem(p, vscode.CompletionItemKind.Property);
          it.insertText = new vscode.SnippetString(`${p}: $0`);
          it.detail = `${widgetName} prop`;
          if (PROP_DOCS[p]) it.documentation = new vscode.MarkdownString(PROP_DOCS[p]);
          return it;
        });
      }
    }

    // 2) Prop value position: `propName: ` → known values for that prop.
    const propValMatch = upto.match(/\b([a-z_][A-Za-z0-9_]*)\s*:\s*([A-Za-z0-9_.]*)$/);
    if (propValMatch && PROP_VALUES[propValMatch[1]]) {
      return PROP_VALUES[propValMatch[1]].map((v) => {
        const it = new vscode.CompletionItem(v, vscode.CompletionItemKind.Value);
        it.detail = `${propValMatch[1]} value`;
        return it;
      });
    }

    // 3) Inside an element's argument list → prop names for that element.
    const enclosing = findEnclosingElement(document, position);
    if (enclosing) {
      const widget = widgetByName(enclosing);
      const props = new Set<string>([...(widget?.props ?? []), ...COMMON_PROPS]);
      return [...props].map((p) => {
        const it = new vscode.CompletionItem(p, vscode.CompletionItemKind.Property);
        it.insertText = new vscode.SnippetString(`${p}: $0`);
        it.detail = widget ? `${widget.name} prop` : 'prop';
        if (PROP_DOCS[p]) it.documentation = new vscode.MarkdownString(PROP_DOCS[p]);
        return it;
      });
    }

    // 4) Node position → element names (with snippets) + keywords.
    const items: vscode.CompletionItem[] = [];

    for (const w of WIDGETS) {
      const it = new vscode.CompletionItem(w.name, vscode.CompletionItemKind.Class);
      it.detail = w.doc;
      it.insertText = elementSnippet(w.name);
      it.documentation = new vscode.MarkdownString(widgetMarkdown(w).value);
      items.push(it);
    }

    for (const k of allKeywords()) {
      const it = new vscode.CompletionItem(k.label, vscode.CompletionItemKind.Keyword);
      it.detail = k.detail;
      it.documentation = new vscode.MarkdownString(k.doc);
      items.push(it);
    }

    items.push(snippet('view', 'view ${1:Name}(${2}) {\n\t$0\n}', 'component scaffold'));
    items.push(snippet('signalbind', 'mut ${1:state} = signal(${2:0})', 'reactive state binding'));

    return items;
  }
}

function elementSnippet(name: string): vscode.SnippetString {
  const w = widgetByName(name);
  if (w?.container) {
    return new vscode.SnippetString(`${name}(${'$1'}) {\n\t$0\n}`);
  }
  if (w?.positional) {
    return new vscode.SnippetString(`${name}("$1"${'$2'})`);
  }
  return new vscode.SnippetString(`${name}($0)`);
}

function snippet(label: string, body: string, detail: string): vscode.CompletionItem {
  const it = new vscode.CompletionItem(label, vscode.CompletionItemKind.Snippet);
  it.insertText = new vscode.SnippetString(body);
  it.detail = detail;
  return it;
}

// ===========================================================================
// Hover — description bubbles
// ===========================================================================

class MuiHoverProvider implements vscode.HoverProvider {
  provideHover(
    document: vscode.TextDocument,
    position: vscode.Position
  ): vscode.Hover | undefined {
    const range = document.getWordRangeAtPosition(position, /[A-Za-z_][A-Za-z0-9_]*/);
    if (!range) return;
    const word = document.getText(range);
    const line = document.lineAt(position.line).text;
    const before = line.slice(0, range.start.character);
    const after = line.slice(range.end.character);

    // 1) Enum member: `EnumType.word`.
    const memberCtx = before.match(/([A-Z][A-Za-z0-9_]*)\.\s*$/);
    if (memberCtx && ENUMS[memberCtx[1]]) {
      const type = memberCtx[1];
      const valid = ENUMS[type].includes(word);
      const md = new vscode.MarkdownString();
      md.appendMarkdown(`\`${type}.${word}\` — enum member\n\n${ENUM_DOCS[type] ?? ''}`);
      if (!valid) md.appendMarkdown(`\n\n⚠️ \`${word}\` is not a member of \`${type}\`.`);
      return new vscode.Hover(md, range);
    }

    // 2) Enum type itself.
    if (ENUMS[word]) {
      const md = new vscode.MarkdownString();
      md.appendMarkdown(`**${word}** — enum\n\n${ENUM_DOCS[word] ?? ''}\n\n`);
      md.appendMarkdown('Members: ' + ENUMS[word].map((m) => `\`${m}\``).join(', '));
      return new vscode.Hover(md, range);
    }

    // 3) Widget / element.
    const w = widgetByName(word);
    if (w && /^[A-Z]/.test(word)) {
      return new vscode.Hover(widgetMarkdown(w), range);
    }

    // 4) Keyword or primitive type — rendered exactly like the Copper
    //    extension's hover: a ```copper signature block + the description.
    const kw = keywordByLabel(word);
    if (kw) {
      const md = new vscode.MarkdownString();
      md.appendMarkdown('```copper\n' + kw.detail + '\n```\n\n' + kw.doc);
      return new vscode.Hover(md, range);
    }

    // 5) Prop name — when followed by `:` or it's a known prop inside an element.
    const looksLikeProp = /^\s*:/.test(after);
    if ((looksLikeProp || PROP_DOCS[word]) && /^[a-z_]/.test(word)) {
      const enclosing = findEnclosingElement(document, position);
      const widget = enclosing ? widgetByName(enclosing) : undefined;
      const doc = PROP_DOCS[word];
      if (doc || looksLikeProp) {
        const md = new vscode.MarkdownString();
        const owner = widget ? `\`${widget.name}\` prop` : 'prop';
        md.appendMarkdown(`**${word}** — ${owner}\n\n${doc ?? '_(no description)_'}`);
        if (PROP_VALUES[word]) {
          md.appendMarkdown(
            '\n\nValues: ' + PROP_VALUES[word].map((v) => `\`${v}\``).join(', ')
          );
        }
        return new vscode.Hover(md, range);
      }
    }

    return undefined;
  }
}

function widgetMarkdown(w: WidgetDef): vscode.MarkdownString {
  const md = new vscode.MarkdownString();
  md.appendMarkdown(`**${w.name}** — ${w.doc}`);
  if (w.positional && w.positionalDoc) {
    md.appendMarkdown(`\n\n_First argument:_ ${w.positionalDoc}`);
  }
  const props = w.props.length ? w.props.join(', ') : '—';
  md.appendMarkdown(`\n\n_Props:_ ${props}`);
  md.appendMarkdown(`\n\n_Common:_ ${COMMON_PROPS.join(', ')}`);
  if (w.container) md.appendMarkdown('\n\n_Accepts a `{ children }` block._');
  return md;
}

// ===========================================================================
// View / function parsing helpers
// ===========================================================================

interface ViewDef {
  name: string;
  params: { name: string; typeName: string }[];
  nameOffset: number;
  matchStart: number;
}

interface FuncDef {
  name: string;
  retType: string;
  nameOffset: number;
  matchStart: number;
}

function parseParamList(raw: string): { name: string; typeName: string }[] {
  if (!raw.trim()) return [];
  return raw
    .split(',')
    .map((p) => {
      const colonIdx = p.indexOf(':');
      if (colonIdx < 0) return { name: p.trim(), typeName: '' };
      return { name: p.slice(0, colonIdx).trim(), typeName: p.slice(colonIdx + 1).trim() };
    })
    .filter((p) => p.name.length > 0);
}

/** Extract all `view Name(params) { ... }` declarations from the document. */
function parseViewDefs(document: vscode.TextDocument): ViewDef[] {
  const views: ViewDef[] = [];
  const text = document.getText();
  const re = /\bview\s+([A-Z][A-Za-z0-9_]*)\s*\(([^)]*)\)/g;
  for (const m of text.matchAll(re)) {
    const name = m[1];
    const nameOffset = m.index! + m[0].indexOf(name);
    views.push({ name, params: parseParamList(m[2]), nameOffset, matchStart: m.index! });
  }
  return views;
}

/** Extract all `func RetType name(...)` declarations from the document. */
function parseFuncDefs(document: vscode.TextDocument): FuncDef[] {
  const funcs: FuncDef[] = [];
  const text = document.getText();
  // Non-greedy: capture everything between `func ` and the last lowercase identifier before `(`
  const re = /\bfunc\s+([^\n]+?)\s+([a-z_][A-Za-z0-9_]*)\s*\(/g;
  for (const m of text.matchAll(re)) {
    const name = m[2];
    const retType = m[1].trim();
    // Find where the function name sits within the matched string
    let parenPos = m[0].lastIndexOf('(');
    let nameEndInMatch = parenPos;
    while (nameEndInMatch > 0 && /\s/.test(m[0][nameEndInMatch - 1])) nameEndInMatch--;
    const nameStartInMatch = nameEndInMatch - name.length;
    funcs.push({ name, retType, nameOffset: m.index! + nameStartInMatch, matchStart: m.index! });
  }
  return funcs;
}

/** Find the `}` matching the `{` at `openIdx`, or -1. */
function findMatchingBrace(text: string, openIdx: number): number {
  let depth = 0;
  for (let i = openIdx; i < text.length; i++) {
    if (text[i] === '{') depth++;
    else if (text[i] === '}') {
      depth--;
      if (depth === 0) return i;
    }
  }
  return -1;
}

/** Count commas at paren-depth 1 in `text` (for active-parameter detection). */
function countTopLevelCommas(text: string): number {
  let depth = 0;
  let count = 0;
  for (const ch of text) {
    if (ch === '(' || ch === '[' || ch === '{') depth++;
    else if (ch === ')' || ch === ']' || ch === '}') depth--;
    else if (ch === ',' && depth === 1) count++;
  }
  return count;
}

// ===========================================================================
// Document symbols (outline panel)
// ===========================================================================

class MuiDocumentSymbolProvider implements vscode.DocumentSymbolProvider {
  provideDocumentSymbols(document: vscode.TextDocument): vscode.DocumentSymbol[] {
    const text = document.getText();
    const symbols: vscode.DocumentSymbol[] = [];

    for (const v of parseViewDefs(document)) {
      const nameStart = document.positionAt(v.nameOffset);
      const nameEnd = document.positionAt(v.nameOffset + v.name.length);
      const defStart = document.positionAt(v.matchStart);
      // Find the view's closing brace for a proper folding range
      const openBraceIdx = text.indexOf('{', v.nameOffset + v.name.length);
      const closeBraceIdx = openBraceIdx >= 0 ? findMatchingBrace(text, openBraceIdx) : -1;
      const defEnd = closeBraceIdx >= 0 ? document.positionAt(closeBraceIdx + 1) : nameEnd;

      const paramStr = v.params
        .map((p) => (p.typeName ? `${p.name}: ${p.typeName}` : p.name))
        .join(', ');
      symbols.push(
        new vscode.DocumentSymbol(
          v.name,
          paramStr ? `(${paramStr})` : '()',
          vscode.SymbolKind.Class,
          new vscode.Range(defStart, defEnd),
          new vscode.Range(nameStart, nameEnd)
        )
      );
    }

    for (const f of parseFuncDefs(document)) {
      const nameStart = document.positionAt(f.nameOffset);
      const nameEnd = document.positionAt(f.nameOffset + f.name.length);
      const defStart = document.positionAt(f.matchStart);
      symbols.push(
        new vscode.DocumentSymbol(
          f.name,
          `func ${f.retType}`,
          vscode.SymbolKind.Function,
          new vscode.Range(defStart, nameEnd),
          new vscode.Range(nameStart, nameEnd)
        )
      );
    }

    return symbols;
  }
}

// ===========================================================================
// Go-to definition (view names and function names)
// ===========================================================================

class MuiDefinitionProvider implements vscode.DefinitionProvider {
  provideDefinition(
    document: vscode.TextDocument,
    position: vscode.Position
  ): vscode.Location | undefined {
    const range = document.getWordRangeAtPosition(position, /[A-Za-z_][A-Za-z0-9_]*/);
    if (!range) return;
    const word = document.getText(range);

    if (/^[A-Z]/.test(word)) {
      for (const v of parseViewDefs(document)) {
        if (v.name === word) {
          const start = document.positionAt(v.nameOffset);
          const end = document.positionAt(v.nameOffset + word.length);
          return new vscode.Location(document.uri, new vscode.Range(start, end));
        }
      }
    }

    for (const f of parseFuncDefs(document)) {
      if (f.name === word) {
        const start = document.positionAt(f.nameOffset);
        const end = document.positionAt(f.nameOffset + word.length);
        return new vscode.Location(document.uri, new vscode.Range(start, end));
      }
    }

    return undefined;
  }
}

// ===========================================================================
// Signature help for view component calls
// ===========================================================================

class MuiSignatureHelpProvider implements vscode.SignatureHelpProvider {
  provideSignatureHelp(
    document: vscode.TextDocument,
    position: vscode.Position
  ): vscode.SignatureHelp | undefined {
    const enclosing = findEnclosingElement(document, position);
    if (!enclosing) return;

    const view = parseViewDefs(document).find((v) => v.name === enclosing);
    if (!view || view.params.length === 0) return;

    const paramStrs = view.params.map((p) =>
      p.typeName ? `${p.name}: ${p.typeName}` : p.name
    );
    const label = `${enclosing}(${paramStrs.join(', ')})`;
    const info = new vscode.SignatureInformation(
      label,
      new vscode.MarkdownString(`User-defined \`view\` component.`)
    );
    info.parameters = paramStrs.map((s) => new vscode.ParameterInformation(s));

    // Count commas from the document start up to the cursor at depth 1
    const upto = document.getText().slice(0, document.offsetAt(position));
    const activeParam = countTopLevelCommas(upto);

    const help = new vscode.SignatureHelp();
    help.signatures = [info];
    help.activeSignature = 0;
    help.activeParameter = Math.min(activeParam, view.params.length - 1);
    return help;
  }
}

// ===========================================================================
// Diagnostics
// ===========================================================================

function computeDiagnostics(document: vscode.TextDocument): vscode.Diagnostic[] {
  const cfg = vscode.workspace.getConfiguration('mui');
  if (!cfg.get('diagnostics', true)) return [];
  const checkProps = cfg.get('checkUnknownProps', true);

  const text = document.getText();
  const masked = maskCodeText(text); // strings/comments → spaces, same length
  const out: vscode.Diagnostic[] = [];
  const rangeAt = (start: number, end: number) =>
    new vscode.Range(document.positionAt(start), document.positionAt(end));

  // ---- a) bracket balance ----
  const pairs: Record<string, string> = { ')': '(', ']': '[', '}': '{' };
  const stack: { ch: string; i: number }[] = [];
  for (let i = 0; i < masked.length; i++) {
    const ch = masked[i];
    if (ch === '(' || ch === '[' || ch === '{') {
      stack.push({ ch, i });
    } else if (ch === ')' || ch === ']' || ch === '}') {
      const top = stack.pop();
      if (!top) {
        out.push(
          new vscode.Diagnostic(
            rangeAt(i, i + 1),
            `Unmatched \`${ch}\`.`,
            vscode.DiagnosticSeverity.Error
          )
        );
      } else if (top.ch !== pairs[ch]) {
        out.push(
          new vscode.Diagnostic(
            rangeAt(i, i + 1),
            `Mismatched \`${ch}\` — expected to close \`${top.ch}\`.`,
            vscode.DiagnosticSeverity.Error
          )
        );
      }
    }
  }
  for (const open of stack) {
    out.push(
      new vscode.Diagnostic(
        rangeAt(open.i, open.i + 1),
        `Unclosed \`${open.ch}\`.`,
        vscode.DiagnosticSeverity.Error
      )
    );
  }

  // ---- b) unknown enum members (only for KNOWN enums, so no false positives) ----
  const enumRe = /\b([A-Z][A-Za-z0-9_]*)\.([A-Za-z0-9_]+)/g;
  for (const m of masked.matchAll(enumRe)) {
    const type = m[1];
    const member = m[2];
    if (ENUMS[type] && !ENUMS[type].includes(member)) {
      const memberStart = m.index! + type.length + 1; // skip "Type."
      out.push(
        new vscode.Diagnostic(
          rangeAt(memberStart, memberStart + member.length),
          `\`${member}\` is not a member of \`${type}\`. ` +
            `Expected: ${ENUMS[type].join(', ')}.`,
          vscode.DiagnosticSeverity.Warning
        )
      );
    }
  }

  // ---- c) unknown props on known widgets ----
  if (checkProps) {
    const callRe = /\b([A-Z][A-Za-z0-9_]*)\s*\(/g;
    for (const m of masked.matchAll(callRe)) {
      const w = widgetByName(m[1]);
      if (!w) continue; // user view / unknown element — don't validate its args
      const openIdx = m.index! + m[0].length - 1; // index of '('
      const closeIdx = matchingParen(masked, openIdx);
      if (closeIdx < 0) continue;
      for (const p of topLevelProps(masked, openIdx + 1, closeIdx)) {
        if (!isKnownProp(w, p.name)) {
          out.push(
            new vscode.Diagnostic(
              rangeAt(p.index, p.index + p.name.length),
              `Unknown prop \`${p.name}\` on \`${w.name}\`.`,
              vscode.DiagnosticSeverity.Warning
            )
          );
        }
      }
    }
  }

  // ---- d) duplicate prop names within the same element call ----
  {
    const dupCallRe = /\b([A-Z][A-Za-z0-9_]*)\s*\(/g;
    for (const m of masked.matchAll(dupCallRe)) {
      const openIdx = m.index! + m[0].length - 1;
      const closeIdx = matchingParen(masked, openIdx);
      if (closeIdx < 0) continue;
      const props = topLevelProps(masked, openIdx + 1, closeIdx);
      const seen = new Map<string, number>();
      for (const p of props) {
        if (seen.has(p.name)) {
          out.push(
            new vscode.Diagnostic(
              rangeAt(p.index, p.index + p.name.length),
              `Duplicate prop \`${p.name}\` — already used in this \`${m[1]}\` call.`,
              vscode.DiagnosticSeverity.Warning
            )
          );
        } else {
          seen.set(p.name, p.index);
        }
      }
    }
  }

  // ---- e) prop value validation for known enum-like props ----
  {
    // Only validate props whose values are plain identifiers (not bitfields).
    const SIMPLE_PROPS: Record<string, string[]> = {
      orientation: PROP_VALUES.orientation,
      direction: PROP_VALUES.direction,
      hAlign: PROP_VALUES.hAlign,
      vAlign: PROP_VALUES.vAlign,
      weight: PROP_VALUES.weight,
    };
    // `align` can be used for both text and Stack cross-axis — skip to avoid false positives.
    const propValRe = /\b([a-z][A-Za-z0-9_]*)\s*:\s*([a-z][A-Za-z0-9_]*)\b/g;
    for (const m of masked.matchAll(propValRe)) {
      const prop = m[1];
      const val = m[2];
      const known = SIMPLE_PROPS[prop];
      if (!known) continue;
      if (!known.includes(val)) {
        const valIdx = m.index! + m[0].length - val.length;
        out.push(
          new vscode.Diagnostic(
            rangeAt(valIdx, valIdx + val.length),
            `Invalid value \`${val}\` for \`${prop}\`. Valid: ${known.map((v) => `\`${v}\``).join(', ')}.`,
            vscode.DiagnosticSeverity.Warning
          )
        );
      }
    }
  }

  return out;
}

/** Index of the `)` matching the `(` at `openIdx`, or -1. Parens only. */
function matchingParen(text: string, openIdx: number): number {
  let depth = 0;
  for (let i = openIdx; i < text.length; i++) {
    if (text[i] === '(') depth++;
    else if (text[i] === ')') {
      depth--;
      if (depth === 0) return i;
    }
  }
  return -1;
}

/**
 * Find `name:` prop labels at the top level of an argument list `[from, to)`.
 * A prop label is an identifier+`:` that starts an argument — i.e. it appears
 * right after the opening `(` or a top-level comma — so ternaries (`a ? b : c`)
 * and nested struct literals / handlers are never mistaken for props.
 */
function topLevelProps(
  text: string,
  from: number,
  to: number
): { name: string; index: number }[] {
  const props: { name: string; index: number }[] = [];
  let depth = 0; // nesting of () [] {}
  let atArgStart = true;
  let i = from;
  while (i < to) {
    const ch = text[i];
    if (ch === '(' || ch === '[' || ch === '{') {
      depth++;
      atArgStart = false;
      i++;
      continue;
    }
    if (ch === ')' || ch === ']' || ch === '}') {
      depth--;
      i++;
      continue;
    }
    if (depth === 0 && ch === ',') {
      atArgStart = true;
      i++;
      continue;
    }
    if (depth === 0 && atArgStart) {
      if (/\s/.test(ch)) {
        i++;
        continue;
      }
      // First non-space token of an argument: is it `name:`?
      const m = /^([a-z_][A-Za-z0-9_]*)\s*:/.exec(text.slice(i, to));
      if (m) {
        props.push({ name: m[1], index: i });
      }
      atArgStart = false; // rest of this argument is a value expression
      i++;
      continue;
    }
    i++;
  }
  return props;
}

/**
 * Replace the contents of line comments, block comments, and string literals
 * with spaces, preserving newlines and overall length so byte offsets still
 * map to document positions. Lets the diagnostics scan ignore brackets/`:`
 * that live inside strings or comments.
 */
function maskCodeText(text: string): string {
  const out = text.split('');
  let i = 0;
  const n = text.length;
  let state: 'code' | 'line' | 'block' | 'string' = 'code';
  while (i < n) {
    const c = text[i];
    const c2 = i + 1 < n ? text[i + 1] : '';
    if (state === 'code') {
      if (c === '/' && c2 === '/') {
        state = 'line';
        out[i] = ' ';
        out[i + 1] = ' ';
        i += 2;
        continue;
      }
      if (c === '/' && c2 === '*') {
        state = 'block';
        out[i] = ' ';
        out[i + 1] = ' ';
        i += 2;
        continue;
      }
      if (c === '"') {
        state = 'string';
        out[i] = ' ';
        i++;
        continue;
      }
      i++;
      continue;
    }
    if (state === 'line') {
      if (c === '\n') {
        state = 'code';
      } else {
        out[i] = ' ';
      }
      i++;
      continue;
    }
    if (state === 'block') {
      if (c === '*' && c2 === '/') {
        out[i] = ' ';
        out[i + 1] = ' ';
        state = 'code';
        i += 2;
        continue;
      }
      if (c !== '\n') out[i] = ' ';
      i++;
      continue;
    }
    // string
    if (c === '\\') {
      out[i] = ' ';
      if (i + 1 < n && text[i + 1] !== '\n') out[i + 1] = ' ';
      i += 2;
      continue;
    }
    if (c === '"') {
      out[i] = ' ';
      state = 'code';
      i++;
      continue;
    }
    if (c !== '\n') out[i] = ' ';
    i++;
  }
  return out.join('');
}

// ===========================================================================
// Shared helpers
// ===========================================================================

/**
 * Scan the document for `id: varname` props on widget elements and return a
 * map of varname → widget type name. Used to offer prop completions for
 * component variable references like `stack1.`.
 */
function collectComponentIds(document: vscode.TextDocument): Map<string, string> {
  const map = new Map<string, string>();
  const text = document.getText();
  // For each `id: ident` in the document, walk backwards to find the
  // enclosing element name (the `CapitalName(` that opened the arg list).
  const idRe = /\bid\s*:\s*([a-z_][A-Za-z0-9_]*)/g;
  for (const m of text.matchAll(idRe)) {
    const idName = m[1];
    const before = text.slice(0, m.index!);
    // Scan backwards tracking paren depth to find the unclosed `(`.
    let depth = 0;
    for (let i = before.length - 1; i >= 0; i--) {
      const ch = before[i];
      if (ch === ')') depth++;
      else if (ch === '(') {
        if (depth === 0) {
          // Element name is the identifier immediately before this `(`.
          const namePart = before.slice(0, i).trimEnd();
          const nm = namePart.match(/([A-Z][A-Za-z0-9_]*)$/);
          if (nm) map.set(idName, nm[1]);
          break;
        }
        depth--;
      }
    }
  }
  return map;
}

/**
 * If the cursor is inside an element's `( ... )` argument list, return that
 * element's name. Scans backwards on the current + a few previous lines,
 * tracking paren depth, and stops at a `{` (block) or `;` boundary.
 */
function findEnclosingElement(
  document: vscode.TextDocument,
  position: vscode.Position
): string | undefined {
  let depth = 0;
  for (let ln = position.line; ln >= 0 && ln > position.line - 40; ln--) {
    const text =
      ln === position.line
        ? document.lineAt(ln).text.slice(0, position.character)
        : document.lineAt(ln).text;
    for (let i = text.length - 1; i >= 0; i--) {
      const ch = text[i];
      if (ch === ')') depth++;
      else if (ch === '(') {
        if (depth === 0) {
          const before = text.slice(0, i);
          const m = before.match(/([A-Za-z_][A-Za-z0-9_]*)\s*$/);
          if (m && /^[A-Z]/.test(m[1])) return m[1];
          return undefined;
        }
        depth--;
      } else if ((ch === '{' || ch === '}') && depth === 0) {
        return undefined;
      }
    }
  }
  return undefined;
}
