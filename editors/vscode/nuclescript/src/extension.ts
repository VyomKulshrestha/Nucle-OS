// Minimal client: spawn `nucle-lsp` over stdio and connect. No logic
// lives here beyond that -- every diagnostic, hover, and definition
// answer comes from the server (nucle_lsp/src/backend.rs), which is
// itself a thin adapter over nucle_lang::analyze. Keeping this file this
// small is deliberate, not an oversight.

import * as vscode from "vscode";
import { LanguageClient, LanguageClientOptions, ServerOptions, TransportKind } from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export function activate(context: vscode.ExtensionContext): void {
  const config = vscode.workspace.getConfiguration("nuclescript");
  const command = config.get<string>("serverPath", "nucle-lsp");

  const serverOptions: ServerOptions = {
    command,
    transport: TransportKind.stdio,
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "nuclescript" }],
  };

  client = new LanguageClient("nuclescript", "NucleScript Language Server", serverOptions, clientOptions);
  client.start();
  context.subscriptions.push({ dispose: () => client?.stop() });
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}
