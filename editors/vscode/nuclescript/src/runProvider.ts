// `NucleScript: Run File` -- runs `nucle-cli run <file>` for the active
// .nsl file in the integrated terminal (via the Tasks API, so argument
// quoting is handled per-shell instead of hand-rolled). nucle-cli is
// resolved the same way nucle-lsp is: PATH first, then an auto-downloaded
// prebuilt binary (see cliDownload.ts) -- so running a program needs
// nothing installed by hand beyond the extension itself.

import * as vscode from "vscode";
import { resolveCliPath } from "./cliDownload";

export function registerRunCommand(context: vscode.ExtensionContext): void {
  context.subscriptions.push(
    vscode.commands.registerCommand("nuclescript.runFile", async (uri?: vscode.Uri) => {
      const document = uri
        ? await vscode.workspace.openTextDocument(uri)
        : vscode.window.activeTextEditor?.document;

      if (!document || document.languageId !== "nuclescript") {
        vscode.window.showErrorMessage("NucleScript: open a .nsl file to run it.");
        return;
      }

      if (document.isDirty) {
        await document.save();
      }

      let cliPath: string;
      try {
        cliPath = await resolveCliPath(context);
      } catch (err) {
        vscode.window.showErrorMessage(`NucleScript: ${(err as Error).message}`);
        return;
      }

      const fileName = document.fileName.split(/[\\/]/).pop();
      const task = new vscode.Task(
        { type: "nuclescript-run" },
        vscode.TaskScope.Workspace,
        `Run ${fileName}`,
        "nuclescript",
        new vscode.ShellExecution(cliPath, ["run", document.fileName]),
      );
      task.presentationOptions = {
        reveal: vscode.TaskRevealKind.Always,
        panel: vscode.TaskPanelKind.Dedicated,
        clear: true,
      };
      await vscode.tasks.executeTask(task);
    }),
  );
}
