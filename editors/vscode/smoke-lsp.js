// Smoke test the copper-lsp binary directly via JSON-RPC over stdio.
// Sends `initialize` + `initialized` + `didOpen`, prints diagnostics, exits.
//
// Usage: node smoke-lsp.js [path-to-copper-lsp.exe]

const { spawn } = require('child_process');
const path = require('path');

const lspPath =
  process.argv[2] ||
  path.resolve(__dirname, '..', '..', 'target', 'release', 'copper-lsp.exe');

const child = spawn(lspPath, [], { stdio: ['pipe', 'pipe', 'pipe'] });

let buf = Buffer.alloc(0);
let nextId = 1;
const pending = new Map();

function send(method, params, isRequest) {
  const obj = isRequest
    ? { jsonrpc: '2.0', id: nextId++, method, params }
    : { jsonrpc: '2.0', method, params };
  const body = Buffer.from(JSON.stringify(obj));
  const header = Buffer.from(`Content-Length: ${body.length}\r\n\r\n`);
  child.stdin.write(Buffer.concat([header, body]));
  return obj.id;
}

child.stderr.on('data', (d) => process.stderr.write(`[lsp-stderr] ${d}`));

child.stdout.on('data', (chunk) => {
  buf = Buffer.concat([buf, chunk]);
  while (true) {
    const sep = buf.indexOf('\r\n\r\n');
    if (sep < 0) return;
    const header = buf.slice(0, sep).toString('utf8');
    const m = header.match(/Content-Length:\s*(\d+)/i);
    if (!m) {
      console.error('bad header:', header);
      return;
    }
    const len = parseInt(m[1], 10);
    if (buf.length < sep + 4 + len) return;
    const body = buf.slice(sep + 4, sep + 4 + len).toString('utf8');
    buf = buf.slice(sep + 4 + len);
    const msg = JSON.parse(body);
    if (msg.id != null && pending.has(msg.id)) {
      pending.get(msg.id)(msg);
      pending.delete(msg.id);
    } else if (msg.method) {
      console.log(`<-- notification: ${msg.method}`);
      if (msg.method === 'textDocument/publishDiagnostics') {
        console.log(`    uri=${msg.params.uri}`);
        console.log(`    diagnostics=${JSON.stringify(msg.params.diagnostics, null, 2)}`);
      }
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
  console.log(`launching: ${lspPath}`);

  const init = await call('initialize', {
    processId: process.pid,
    rootUri: null,
    capabilities: {},
  });
  console.log('initialize OK:', init.result?.serverInfo);

  send('initialized', {}, false);

  // Open a buggy doc — missing `}` should produce a diagnostic.
  const uri = 'file:///smoke.crs';
  const text = 'func void hello() {\n  x = 1\n';
  send(
    'textDocument/didOpen',
    {
      textDocument: {
        uri,
        languageId: 'copper',
        version: 1,
        text,
      },
    },
    false
  );

  // Ask for symbols.
  const syms = await call('textDocument/documentSymbol', {
    textDocument: { uri },
  });
  console.log('symbols:', JSON.stringify(syms.result, null, 2));

  // Hover on `func` (line 0, char 2).
  const hover = await call('textDocument/hover', {
    textDocument: { uri },
    position: { line: 0, character: 2 },
  });
  console.log('hover:', JSON.stringify(hover.result, null, 2));

  // Wait a bit so the publishDiagnostics notification has time to land.
  await new Promise((r) => setTimeout(r, 250));

  await call('shutdown', null);
  send('exit', null, false);

  setTimeout(() => process.exit(0), 200);
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
