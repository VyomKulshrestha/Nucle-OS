// Minimal client: resolve the `nucle-lsp` binary (see serverDownload.ts),
// spawn it over stdio, and connect. No language logic lives here --
// every diagnostic, hover, and definition answer comes from the server
// (nucle_lsp/src/backend.rs), which is itself a thin adapter over
// nucle_lang::analyze. Keeping this file this small is deliberate.

import * as vscode from "vscode";
import { LanguageClient, LanguageClientOptions, ServerOptions, TransportKind } from "vscode-languageclient/node";
import { resolveServerPath } from "./serverDownload";

let client: LanguageClient | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  let command: string;
  try {
    command = await resolveServerPath(context);
  } catch (err) {
    vscode.window.showErrorMessage(`NucleScript: ${(err as Error).message}`);
    return;
  }

  const serverOptions: ServerOptions = {
    command,
    transport: TransportKind.stdio,
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "nuclescript" }],
  };

  client = new LanguageClient("nuclescript", "NucleScript Language Server", serverOptions, clientOptions);
  await client.start();
  context.subscriptions.push({ dispose: () => client?.stop() });
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}
