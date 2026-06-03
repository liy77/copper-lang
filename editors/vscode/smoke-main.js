// Reproduce the two cases from the screenshots against main.crs:
//   1. `Greeting::new("Alice").gree|`  → should suggest `greet`
//   2. `msg.|` where msg: &str         → should suggest String methods

const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

const lspPath = path.resolve(__dirname, '..', '..', 'target', 'debug', 'copper-lsp.exe');
const child = spawn(lspPath, [], { stdio: ['pipe', 'pipe', 'pipe'] });

let buf = Buffer.alloc(0);
let nextId = 1;
const pending = new Map();
function send(method, params, isReq) {
  const obj = isReq
    ? { jsonrpc: '2.0', id: nextId++, method, params }
    : { jsonrpc: '2.0', method, params };
  const body = Buffer.from(JSON.stringify(obj));
  child.stdin.write(Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body]));
  return obj.id;
}
child.stderr.on('data', (d) => process.stderr.write(`[lsp-stderr] ${d}`));
child.stdout.on('data', (chunk) => {
  buf = Buffer.concat([buf, chunk]);
  while (true) {
    const sep = buf.indexOf('\r\n\r\n');
    if (sep < 0) return;
    const m = buf.slice(0, sep).toString('utf8').match(/Content-Length:\s*(\d+)/i);
    if (!m) return;
    const len = parseInt(m[1], 10);
    if (buf.length < sep + 4 + len) return;
    const body = buf.slice(sep + 4, sep + 4 + len).toString('utf8');
    buf = buf.slice(sep + 4 + len);
    const msg = JSON.parse(body);
    if (msg.id != null && pending.has(msg.id)) {
      pending.get(msg.id)(msg);
      pending.delete(msg.id);
    }
  }
});
function call(method, params) {
  return new Promise((r) => {
    const id = send(method, params, true);
    pending.set(id, r);
  });
}

(async () => {
  await call('initialize', { processId: process.pid, rootUri: null, capabilities: {} });
  send('initialized', {}, false);

  // Case 1: Greeting::new("Alice").gree
  const text1 = [
    'class Greeting {',
    '  name: String',
    '  Greeting(name: String) { self.name = name }',
    '  String greet(self) { return self.name }',
    '}',
    '',
    'Greeting::new("Alice").gree',     // line 6, cursor at end (char 27)
  ].join('\n');
  const uri1 = 'file:///c1.crs';
  send('textDocument/didOpen', {
    textDocument: { uri: uri1, languageId: 'copper', version: 1, text: text1 },
  }, false);
  const r1 = await call('textDocument/completion', {
    textDocument: { uri: uri1 },
    position: { line: 6, character: 27 },
    context: { triggerKind: 1 },
  });
  const items1 = Array.isArray(r1.result) ? r1.result : (r1.result?.items ?? []);
  console.log(`\n--- Greeting::new("Alice").gree → ${items1.length} items ---`);
  for (const it of items1.slice(0, 8)) {
    const k = ({ 5: 'FIELD', 2: 'METHOD', 4: 'CTOR' })[it.kind] ?? it.kind;
    console.log(`  ${k} ${it.label} :: ${it.detail || ''}`);
  }

  // Case 2: msg.| inside `unsafe func void shout(msg: &str) { ... msg.| }`
  const text2 = [
    'unsafe func void shout(msg: &str) {',
    '  msg.',                            // line 1, char 6
    '}',
  ].join('\n');
  const uri2 = 'file:///c2.crs';
  send('textDocument/didOpen', {
    textDocument: { uri: uri2, languageId: 'copper', version: 1, text: text2 },
  }, false);
  const r2 = await call('textDocument/completion', {
    textDocument: { uri: uri2 },
    position: { line: 1, character: 6 },
    context: { triggerKind: 2, triggerCharacter: '.' },
  });
  const items2 = Array.isArray(r2.result) ? r2.result : (r2.result?.items ?? []);
  const labels2 = items2.map((i) => i.label);
  console.log(`\n--- msg. (msg: &str) → ${items2.length} items ---`);
  console.log(`  has to_uppercase? ${labels2.includes('to_uppercase')}`);
  console.log(`  first 8: ${labels2.slice(0, 8).join(', ')}`);

  // Case 3 (regression): main.crs contents for `Greeting::new("Alice").gree`
  // straight from the user's file:
  const main = fs.readFileSync(
    path.resolve(__dirname, '..', '..', 'main.crs'),
    'utf8'
  );
  const uri3 = 'file:///main.crs';
  send('textDocument/didOpen', {
    textDocument: { uri: uri3, languageId: 'copper', version: 1, text: main },
  }, false);
  // Find a `.|` location in main.crs.
  const lines = main.split('\n');
  const idx = lines.findIndex((l) => l.includes('Greeting::new('));
  if (idx >= 0) {
    const line = lines[idx];
    const dotPos = line.indexOf('.', line.indexOf(')'));
    if (dotPos >= 0) {
      const r3 = await call('textDocument/completion', {
        textDocument: { uri: uri3 },
        position: { line: idx, character: dotPos + 1 },
        context: { triggerKind: 2, triggerCharacter: '.' },
      });
      const items3 = Array.isArray(r3.result) ? r3.result : (r3.result?.items ?? []);
      console.log(`\n--- main.crs line ${idx + 1} after dot → ${items3.length} items ---`);
      for (const it of items3.slice(0, 8)) {
        const k = ({ 5: 'FIELD', 2: 'METHOD' })[it.kind] ?? it.kind;
        console.log(`  ${k} ${it.label} :: ${it.detail || ''}`);
      }
    }
  }

  await call('shutdown', null);
  send('exit', null, false);
  setTimeout(() => process.exit(0), 200);
})().catch((e) => { console.error(e); process.exit(1); });
