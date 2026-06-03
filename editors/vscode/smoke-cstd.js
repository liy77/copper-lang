const { spawn } = require('child_process');
const path = require('path');

const lspPath =
  process.argv[2] ||
  path.resolve(__dirname, '..', '..', 'target', 'debug', 'copper-lsp.exe');

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
    } else if (msg.method === 'window/logMessage') {
      console.log(`[srv] ${msg.params.message}`);
    } else if (msg.method === 'textDocument/publishDiagnostics') {
      console.log(`[diag] uri=${msg.params.uri} count=${msg.params.diagnostics.length}`);
      for (const d of msg.params.diagnostics) {
        console.log(`  - ${d.message}`);
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
  await call('initialize', { processId: process.pid, rootUri: null, capabilities: {} });
  send('initialized', {}, false);

  // Open matching.crs verbatim (the file the user was looking at).
  const fs = require('fs');
  const text = fs.readFileSync(
    path.resolve(__dirname, '..', '..', 'examples', 'matching.crs'),
    'utf8'
  );
  const uri = 'file:///matching.crs';
  send('textDocument/didOpen', {
    textDocument: { uri, languageId: 'copper', version: 1, text },
  }, false);

  // Wait for diagnostics.
  await new Promise((r) => setTimeout(r, 300));

  // Trigger completion at start of file - we want to see cstd entries.
  const comp = await call('textDocument/completion', {
    textDocument: { uri },
    position: { line: 0, character: 0 },
    context: { triggerKind: 1 },
  });
  const items = comp.result || [];
  const arr = Array.isArray(items) ? items : items.items || [];
  const cstdLike = arr.filter((i) => i.detail && i.detail.includes('func '));
  console.log(`completion items total=${arr.length}`);
  console.log(`func-detail items: ${cstdLike.length}`);
  for (const it of cstdLike.slice(0, 8)) {
    console.log(`  ${it.label}: ${it.detail}`);
  }

  async function checkSignature(snippet, label) {
    const sigUri = `file:///${label}.crs`;
    send('textDocument/didOpen', {
      textDocument: { uri: sigUri, languageId: 'copper', version: 1, text: snippet },
    }, false);
    const sig = await call('textDocument/signatureHelp', {
      textDocument: { uri: sigUri },
      position: { line: 0, character: snippet.length },
      context: { triggerKind: 2, triggerCharacter: '(', isRetrigger: false },
    });
    console.log(`signatureHelp ${label}:`, JSON.stringify(sig.result, null, 2));
  }
  await checkSignature('input(', 'input');
  await checkSignature('Some(', 'Some');
  await checkSignature('Ok(', 'Ok');
  await checkSignature('println!(', 'println');
  await checkSignature('format!(', 'format');

  async function checkHover(text, line, character, label) {
    const hoverUri = `file:///hov-${label}.crs`;
    send('textDocument/didOpen', {
      textDocument: { uri: hoverUri, languageId: 'copper', version: 1, text },
    }, false);
    const hov = await call('textDocument/hover', {
      textDocument: { uri: hoverUri },
      position: { line, character },
    });
    const value = hov.result?.contents?.value;
    console.log(`hover ${label}: ${value ? value.split('\n')[0] : '(none)'}`);
  }
  await checkHover('Some(42)', 0, 1, 'Some');
  await checkHover('let x = None', 0, 9, 'None');
  await checkHover('Vec<i32>', 0, 1, 'Vec');
  await checkHover('println!("hi")', 0, 3, 'println');

  await call('shutdown', null);
  send('exit', null, false);
  setTimeout(() => process.exit(0), 200);
})().catch((e) => { console.error(e); process.exit(1); });
