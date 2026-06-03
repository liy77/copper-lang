// Launches the `mui-lsp` language server and connects to it as an LSP client.
//
// When a server binary is found, the LSP provides completion, hover, color
// swatches, document symbols, go-to-definition and diagnostics — all from the
// real `mui-syntax` parser, in any LSP editor. When no binary can be located,
// `start()` returns undefined and the extension falls back to its in-process
// TypeScript providers (see extension.ts).

import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from 'vscode-languageclient/node';

const EXE = process.platform === 'win32' ? 'mui-lsp.exe' : 'mui-lsp';

/** Try to locate a `mui-lsp` binary. Mirrors how cforge discovers `mui-dev`. */
function locateServer(context: vscode.ExtensionContext): string | undefined {
  // 1) Explicit user setting.
  const configured = vscode.workspace
    .getConfiguration('mui')
    .get<string>('lsp.path');
  if (configured && fs.existsSync(configured)) return configured;

  // 2) Environment override.
  const env = process.env.MUI_LSP_BIN;
  if (env && fs.existsSync(env)) return env;

  // 3) Bundled with the extension (packaged .vsix).
  const bundled = path.join(context.extensionPath, 'bin', EXE);
  if (fs.existsSync(bundled)) return bundled;

  // 4) Dev workspace: a built binary under copper-lang/target/.
  //    The extension lives at copper-lang/editors/vscode-mui, so walk up to the
  //    repo root and look in target/{release,debug}.
  let dir = context.extensionPath;
  for (let i = 0; i < 6; i++) {
    for (const profile of ['release', 'debug']) {
      const cand = path.join(dir, 'target', profile, EXE);
      if (fs.existsSync(cand)) return cand;
    }
    const parent = path.dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }

  // 5) On PATH (resolved by the OS when we spawn just the name).
  //    We can't cheaply test this here, so only use it as a last resort when
  //    the user opted in via the setting `mui.lsp.usePath`.
  if (vscode.workspace.getConfiguration('mui').get<boolean>('lsp.usePath')) {
    return EXE;
  }
  return undefined;
}

/**
 * Start the MUI language server. Returns the running client, or `undefined`
 * when the LSP is disabled or no server binary could be found (the caller then
 * registers the in-process fallback providers).
 */
export async function start(
  context: vscode.ExtensionContext
): Promise<LanguageClient | undefined> {
  if (!vscode.workspace.getConfiguration('mui').get<boolean>('lsp.enabled', true)) {
    return undefined;
  }
  const command = locateServer(context);
  if (!command) {
    return undefined;
  }

  const serverOptions: ServerOptions = {
    run: { command, transport: TransportKind.stdio },
    debug: { command, transport: TransportKind.stdio },
  };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: 'file', language: 'mui' },
      { scheme: 'file', language: 'crm' },
    ],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher('**/*.{mui,crm}'),
    },
  };

  const client = new LanguageClient(
    'mui-lsp',
    'MUI Language Server',
    serverOptions,
    clientOptions
  );
  try {
    await client.start();
    return client;
  } catch (e) {
    void vscode.window.showWarningMessage(
      `MUI: failed to start language server (${String(
        e
      )}). Falling back to built-in features.`
    );
    return undefined;
  }
}
