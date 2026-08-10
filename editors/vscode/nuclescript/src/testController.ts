import { spawn } from "child_process";
import * as vscode from "vscode";
import { resolveCliPath } from "./cliDownload";
import { discoverTests } from "./testDiscovery";

interface IndexedTest {
  item: vscode.TestItem;
  uri: vscode.Uri;
  name: string;
}

interface CliFailure {
  message: string;
  line: number;
  column: number;
}

interface CliResult {
  name: string;
  passed: boolean;
  failures: CliFailure[];
}

interface CliDiagnostic {
  code?: string;
  message: string;
  span?: { line?: number; column?: number };
}

interface CliTestReport {
  ok: boolean;
  compile_errors: CliDiagnostic[];
  results: CliResult[];
}

export function registerTestController(context: vscode.ExtensionContext): void {
  const controller = vscode.tests.createTestController("nuclescriptTests", "NucleScript Tests");
  const indexedTests = new Map<string, IndexedTest>();
  context.subscriptions.push(controller);

  const discoverFile = async (uri: vscode.Uri): Promise<void> => {
    let document: vscode.TextDocument;
    try {
      document = await vscode.workspace.openTextDocument(uri);
    } catch {
      return;
    }
    const fileId = uri.toString();
    let fileItem = controller.items.get(fileId);
    if (!fileItem) {
      fileItem = controller.createTestItem(fileId, vscode.workspace.asRelativePath(uri), uri);
      controller.items.add(fileItem);
    }

    fileItem.children.forEach((child) => indexedTests.delete(child.id));
    fileItem.children.replace([]);
    for (const discovered of discoverTests(document.getText())) {
      const id = `${fileId}#${discovered.startOffset}:${discovered.name}`;
      const item = controller.createTestItem(id, discovered.name, uri);
      item.range = new vscode.Range(
        document.positionAt(discovered.startOffset),
        document.positionAt(discovered.endOffset),
      );
      fileItem.children.add(item);
      indexedTests.set(id, { item, uri, name: discovered.name });
    }
  };

  const discoverWorkspace = async (): Promise<void> => {
    const files = await vscode.workspace.findFiles("**/*.nsl", "**/{target,node_modules,.git}/**");
    await Promise.all(files.map(discoverFile));
  };

  controller.refreshHandler = discoverWorkspace;
  controller.resolveHandler = async (item) => {
    if (item?.uri) {
      await discoverFile(item.uri);
    } else {
      await discoverWorkspace();
    }
  };

  const watcher = vscode.workspace.createFileSystemWatcher("**/*.nsl");
  watcher.onDidCreate((uri) => void discoverFile(uri));
  watcher.onDidChange((uri) => void discoverFile(uri));
  watcher.onDidDelete((uri) => {
    const fileId = uri.toString();
    const item = controller.items.get(fileId);
    item?.children.forEach((child) => indexedTests.delete(child.id));
    controller.items.delete(fileId);
  });
  context.subscriptions.push(watcher);

  controller.createRunProfile(
    "Run NucleScript Tests",
    vscode.TestRunProfileKind.Run,
    (request, token) => runTests(controller, indexedTests, context, request, token),
    true,
  );

  void discoverWorkspace();
}

async function runTests(
  controller: vscode.TestController,
  index: Map<string, IndexedTest>,
  context: vscode.ExtensionContext,
  request: vscode.TestRunRequest,
  token: vscode.CancellationToken,
): Promise<void> {
  const run = controller.createTestRun(request);
  const selected = collectSelectedTests(controller, index, request);
  selected.forEach((test) => run.enqueued(test.item));

  let cliPath: string;
  try {
    cliPath = await resolveCliPath(context);
  } catch (error) {
    const message = new vscode.TestMessage((error as Error).message);
    selected.forEach((test) => run.errored(test.item, message));
    run.end();
    return;
  }

  const byFile = new Map<string, IndexedTest[]>();
  for (const test of selected) {
    const key = test.uri.toString();
    const group = byFile.get(key) ?? [];
    group.push(test);
    byFile.set(key, group);
  }

  for (const tests of byFile.values()) {
    if (token.isCancellationRequested) {
      tests.forEach((test) => run.skipped(test.item));
      continue;
    }
    tests.forEach((test) => run.started(test.item));
    const uri = tests[0].uri;
    const openDocument = vscode.workspace.textDocuments.find((document) => document.uri.toString() === uri.toString());
    if (openDocument?.isDirty) {
      await openDocument.save();
    }

    const startedAt = Date.now();
    try {
      const execution = await executeCli(cliPath, uri.fsPath, token);
      run.appendOutput(execution.output.replace(/\r?\n/g, "\r\n"), undefined, tests[0].item);
      if (execution.report.compile_errors.length > 0) {
        const messages = execution.report.compile_errors.map((diagnostic) => diagnosticMessage(uri, diagnostic));
        tests.forEach((test) => run.errored(test.item, messages, Date.now() - startedAt));
        continue;
      }

      for (const test of tests) {
        const result = execution.report.results.find((candidate) => candidate.name === test.name);
        if (!result) {
          run.errored(test.item, new vscode.TestMessage(`nucle-cli returned no result for '${test.name}'`));
        } else if (result.passed) {
          run.passed(test.item, Date.now() - startedAt);
        } else {
          const messages = result.failures.map((failure) => failureMessage(uri, failure));
          run.failed(test.item, messages, Date.now() - startedAt);
        }
      }
    } catch (error) {
      if (token.isCancellationRequested) {
        tests.forEach((test) => run.skipped(test.item));
      } else {
        const message = new vscode.TestMessage((error as Error).message);
        tests.forEach((test) => run.errored(test.item, message, Date.now() - startedAt));
      }
    }
  }
  run.end();
}

function collectSelectedTests(
  controller: vscode.TestController,
  index: Map<string, IndexedTest>,
  request: vscode.TestRunRequest,
): IndexedTest[] {
  const excluded = new Set((request.exclude ?? []).map((item) => item.id));
  const selected = new Map<string, IndexedTest>();
  const visit = (item: vscode.TestItem): void => {
    if (excluded.has(item.id)) return;
    const test = index.get(item.id);
    if (test) selected.set(item.id, test);
    item.children.forEach(visit);
  };
  if (request.include) {
    request.include.forEach(visit);
  } else {
    controller.items.forEach(visit);
  }
  return [...selected.values()];
}

function executeCli(
  cliPath: string,
  filePath: string,
  token: vscode.CancellationToken,
): Promise<{ report: CliTestReport; output: string }> {
  return new Promise((resolve, reject) => {
    const child = spawn(cliPath, ["test", filePath, "--json"], { windowsHide: true });
    let stdout = "";
    let stderr = "";
    let settled = false;
    child.stdout.on("data", (chunk: Buffer) => (stdout += chunk.toString()));
    child.stderr.on("data", (chunk: Buffer) => (stderr += chunk.toString()));
    child.on("error", (error) => {
      if (!settled) reject(new Error(`could not run '${cliPath}': ${error.message}`));
      settled = true;
    });
    child.on("close", (code) => {
      if (settled) return;
      settled = true;
      try {
        const report = JSON.parse(stdout) as CliTestReport;
        if (!Array.isArray(report.compile_errors) || !Array.isArray(report.results)) {
          throw new Error("JSON report is missing compile_errors or results");
        }
        resolve({ report, output: stderr ? `${stdout}\n${stderr}` : stdout });
      } catch (error) {
        reject(
          new Error(
            stderr.trim() ||
              `nucle-cli exited with code ${code} and did not return a valid test report: ${(error as Error).message}`,
          ),
        );
      }
    });
    const cancellation = token.onCancellationRequested(() => child.kill());
    child.on("close", () => cancellation.dispose());
  });
}

function diagnosticMessage(uri: vscode.Uri, diagnostic: CliDiagnostic): vscode.TestMessage {
  const message = new vscode.TestMessage(
    `${diagnostic.code ? `${diagnostic.code}: ` : ""}${diagnostic.message}`,
  );
  if (diagnostic.span?.line) {
    message.location = new vscode.Location(
      uri,
      new vscode.Position(diagnostic.span.line - 1, Math.max(0, (diagnostic.span.column ?? 1) - 1)),
    );
  }
  return message;
}

function failureMessage(uri: vscode.Uri, failure: CliFailure): vscode.TestMessage {
  const message = new vscode.TestMessage(failure.message);
  message.location = new vscode.Location(
    uri,
    new vscode.Position(Math.max(0, failure.line - 1), Math.max(0, failure.column - 1)),
  );
  return message;
}
