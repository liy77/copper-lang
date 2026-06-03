const { spawn } = require('child_process');
const path = require('path');

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

  const text = [
    'class Greeting {',
    '  name: String',
    '  Greeting(name: String) { self.name = name }',
    '  String greet(self) { return self.name }',
    '}',
    '',
    'Greeting::new("Alice").greet()',
  ].join('\n');
  const uri = 'file:///hover.crs';
  send('textDocument/didOpen', {
    textDocument: { uri, languageId: 'copper', version: 1, text },
  }, false);

  async function hov(label, line, character) {
    const r = await call('textDocument/hover', {
      textDocument: { uri },
      position: { line, character },
    });
    const v = r.result?.contents?.value;
    console.log(`\n=== ${label} (line ${line} char ${character}) ===`);
    console.log(v ? v : '(no hover)');
  }

  // Line 6 = "Greeting::new("Alice").greet()"
  // chars:   01234567890123456789012345678901
  //                    ^new (10-12)        ^greet (23-27)
  await hov('hover on `new`', 6, 11);
  await hov('hover on `greet` (chained)', 6, 25);

  // Class declaration line — hover on `greet` in `String greet(self)` line.
  await hov('hover on `greet` in declaration', 3, 10);
  await hov('hover on `name` field', 1, 4);

  // Stdlib hover: `msg.to_uppercase()` where msg: &str.
  const text2 = [
    'unsafe func void shout(msg: &str) {',
    '  println!(">>> {} <<<", msg.to_uppercase())',
    '}',
  ].join('\n');
  const uri2 = 'file:///std-hover.crs';
  send('textDocument/didOpen', {
    textDocument: { uri: uri2, languageId: 'copper', version: 1, text: text2 },
  }, false);
  // Line 1 = "  println!(">>> {} <<<", msg.to_uppercase())"
  //          0         1         2         3         4
  //          0123456789012345678901234567890123456789012345
  // `to_uppercase` starts at char 30. Hover at 33 (mid-word).
  const r2 = await call('textDocument/hover', {
    textDocument: { uri: uri2 },
    position: { line: 1, character: 33 },
  });
  console.log('\n=== hover on `to_uppercase` (msg: &str) ===');
  console.log(r2.result?.contents?.value || '(no hover)');

  // Variable hover: `mut abs = x < 0 ? -x : x` → mut abs: i32
  const text4 = [
    'func void demo(x: i32) {',
    '  mut abs = x < 0 ? -x : x',
    '  mut greeting = "hello"',
    '  mut count = 42',
    '  println!("{}", abs)',
    '}',
  ].join('\n');
  const uri4 = 'file:///vars.crs';
  send('textDocument/didOpen', {
    textDocument: { uri: uri4, languageId: 'copper', version: 1, text: text4 },
  }, false);
  async function vhov(label, line, character) {
    const r = await call('textDocument/hover', {
      textDocument: { uri: uri4 },
      position: { line, character },
    });
    console.log(`\n=== ${label} ===\n${r.result?.contents?.value || '(no hover)'}`);
  }
  // line 1 = "  mut abs = x < 0 ? -x : x" — `abs` at chars 6-9
  await vhov('hover on `abs`', 1, 7);
  // line 2 = "  mut greeting = \"hello\"" — `greeting` at chars 6-14
  await vhov('hover on `greeting`', 2, 9);
  // line 3 = "  mut count = 42" — `count` at chars 6-11
  await vhov('hover on `count`', 3, 8);
  // Param `x`: in line 1 "  mut abs = x < 0 ? -x : x", the first `x` is
  // at char 12 (inclusive). Hover at 12.
  await vhov('hover on param `x` (use)', 1, 12);

  // Vec method:
  const text3 = 'mut nums = vec![1, 2, 3]\nnums.push(4)';
  const uri3 = 'file:///vec-hover.crs';
  send('textDocument/didOpen', {
    textDocument: { uri: uri3, languageId: 'copper', version: 1, text: text3 },
  }, false);
  // Line 1 = "nums.push(4)" → push starts at char 5.
  const r3 = await call('textDocument/hover', {
    textDocument: { uri: uri3 },
    position: { line: 1, character: 7 },
  });
  console.log('\n=== hover on `push` (Vec) ===');
  console.log(r3.result?.contents?.value || '(no hover)');

  await call('shutdown', null);
  send('exit', null, false);
  setTimeout(() => process.exit(0), 200);
})().catch((e) => { console.error(e); process.exit(1); });
