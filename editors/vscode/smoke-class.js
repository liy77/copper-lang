const { spawn } = require('child_process');
const path = require('path');

const lspPath =
  process.argv[2] ||
  path.resolve(__dirname, '..', '..', 'target', 'debug', 'copper-lsp.exe');

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
  return new Promise((resolve) => {
    const id = send(method, params, true);
    pending.set(id, resolve);
  });
}

(async () => {
  await call('initialize', { processId: process.pid, rootUri: null, capabilities: {} });
  send('initialized', {}, false);

  // A program with a Greeter class, a typed `var` binding, and an `obj.`
  // half-typed expression.
  const text = [
    'class Greeter {',
    '  name: String',
    '',
    '  Greeter(name: String) {',
    '    self.name = name',
    '  }',
    '',
    '  void hello(self) {',
    '    println!("hi")',
    '  }',
    '',
    '  String shout(self, prefix: str) {',
    '    return prefix',
    '  }',
    '}',
    '',
    'mut g = Greeter("Brian")',
    'g.',                                       // line 16, char 2 — completion target
    '',
    'class User {',
    '  age: i32',
    '  void birthday(self) {',
    '    self.', // line 21 char 9 — self.<cursor>
    '  }',
    '}',
    ''
  ].join('\n');
  const uri = 'file:///class.crs';
  send('textDocument/didOpen', {
    textDocument: { uri, languageId: 'copper', version: 1, text },
  }, false);

  await new Promise((r) => setTimeout(r, 200));

  async function check(label, line, character) {
    const r = await call('textDocument/completion', {
      textDocument: { uri },
      position: { line, character },
      context: { triggerKind: 2, triggerCharacter: '.' },
    });
    const items = Array.isArray(r.result) ? r.result : (r.result?.items ?? []);
    console.log(`\n--- ${label} (line ${line} char ${character}) → ${items.length} items ---`);
    for (const it of items) {
      const kindName = ({ 5: 'FIELD', 2: 'METHOD' })[it.kind] ?? it.kind;
      console.log(`  ${kindName}  ${it.label}${it.detail ? ' :: ' + it.detail : ''}`);
    }
  }

  await check('g.', 17, 2);          // line 17 (0-indexed) = `g.`
  await check('self. inside birthday', 22, 9);

  // Static-member access: `Greeter::` should surface `new` (synthesised
  // from the constructor) and not the global keyword soup.
  const text2 = 'class Greeter {\n  Greeter(name: String) { self.name = name }\n}\n\nGreeter::';
  const uri2 = 'file:///static.crs';
  send('textDocument/didOpen', {
    textDocument: { uri: uri2, languageId: 'copper', version: 1, text: text2 },
  }, false);
  const r = await call('textDocument/completion', {
    textDocument: { uri: uri2 },
    position: { line: 4, character: 9 },
    context: { triggerKind: 2, triggerCharacter: ':' },
  });
  const items = Array.isArray(r.result) ? r.result : (r.result?.items ?? []);
  console.log(`\n--- Greeter:: → ${items.length} items ---`);
  for (const it of items) {
    console.log(`  ${it.label} :: ${it.detail || ''}`);
  }

  // String literal `name.` → should surface to_uppercase, len, etc.
  const text3 = 'mut name = "brian"\nname.';
  const uri3 = 'file:///stdmethods.crs';
  send('textDocument/didOpen', {
    textDocument: { uri: uri3, languageId: 'copper', version: 1, text: text3 },
  }, false);
  const r3 = await call('textDocument/completion', {
    textDocument: { uri: uri3 },
    position: { line: 1, character: 5 },
    context: { triggerKind: 2, triggerCharacter: '.' },
  });
  const items3 = Array.isArray(r3.result) ? r3.result : (r3.result?.items ?? []);
  console.log(`\n--- name. (String) → ${items3.length} items, first 8 ---`);
  for (const it of items3.slice(0, 8)) {
    console.log(`  ${it.label} :: ${it.detail || ''}`);
  }
  const hasUpper = items3.some((i) => i.label === 'to_uppercase');
  console.log(`  has to_uppercase? ${hasUpper}`);

  // Vec literal → push, pop, etc.
  const text4 = 'mut nums = vec![1, 2, 3]\nnums.';
  const uri4 = 'file:///vec.crs';
  send('textDocument/didOpen', {
    textDocument: { uri: uri4, languageId: 'copper', version: 1, text: text4 },
  }, false);
  const r4 = await call('textDocument/completion', {
    textDocument: { uri: uri4 },
    position: { line: 1, character: 5 },
    context: { triggerKind: 2, triggerCharacter: '.' },
  });
  const items4 = Array.isArray(r4.result) ? r4.result : (r4.result?.items ?? []);
  const labels4 = items4.map((i) => i.label).slice(0, 10);
  console.log(`\n--- nums. (Vec) → ${items4.length} items, sample ---`);
  console.log(`  ${labels4.join(', ')}`);

  await call('shutdown', null);
  send('exit', null, false);
  setTimeout(() => process.exit(0), 200);
})().catch((e) => { console.error(e); process.exit(1); });
