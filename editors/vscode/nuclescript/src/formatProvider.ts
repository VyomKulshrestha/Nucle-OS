// `Format Document` / format-on-save support, implemented by shelling out
// to `nucle-cli fmt -` (reading the buffer from stdin, per
// nucle_cli/src/main.rs's `-` handling) rather than reimplementing
// NucleScript's formatting rules here or in the language server -- there
// is exactly one formatter implementation (`nucle_lang::formatter`), and
// every caller (the CLI, this extension) goes through it.
//
// Stdin, not the file on disk, is what gets formatted: VS Code invokes a
// formatting provider on the in-memory document, which may have unsaved
// edits, before writing it to disk on save.

import { spawn } from "child_process";
import * as vscode from "vscode";
import { resolveCliPath } from "./cliDownload";

export function registerFormattingProvider(context: vscode.ExtensionContext): void {
  const provider: vscode.DocumentFormattingEditProvider = {
    async provideDocumentFormattingEdits(document, _options, token) {
      const source = document.getText();
      try {
        const cliPath = await resolveCliPath(context);
        const formatted = await runFmt(cliPath, source, token);
        if (formatted === source) {
          return [];
        }
        const fullRange = new vscode.Range(document.positionAt(0), document.positionAt(source.length));
        return [vscode.TextEdit.replace(fullRange, formatted)];
      } catch (err) {
        vscode.window.showErrorMessage(`NucleScript: format failed -- ${(err as Error).message}`);
        return [];
      }
    },
  };

  context.subscriptions.push(
    vscode.languages.registerDocumentFormattingEditProvider({ scheme: "file", language: "nuclescript" }, provider)
  );
}

function runFmt(cliPath: string, source: string, token: vscode.CancellationToken): Promise<string> {
  return new Promise((resolve, reject) => {
    const child = spawn(cliPath, ["fmt", "-"]);
    let stdout = "";
    let stderr = "";

    child.stdout.on("data", (chunk: Buffer) => (stdout += chunk.toString()));
    child.stderr.on("data", (chunk: Buffer) => (stderr += chunk.toString()));
    child.on("error", (err) => {
      reject(new Error(`could not run '${cliPath}' (set nuclescript.cliPath, or build it with \`cargo build -p nucle_cli\`): ${err.message}`));
    });
    child.on("close", (code) => {
      if (code === 0) {
        resolve(stdout);
      } else {
        reject(new Error(stderr.trim() || `nucle-cli exited with code ${code}`));
      }
    });

    const cancelSubscription = token.onCancellationRequested(() => child.kill());
    child.on("close", () => cancelSubscription.dispose());

    child.stdin.write(source);
    child.stdin.end();
  });
}
