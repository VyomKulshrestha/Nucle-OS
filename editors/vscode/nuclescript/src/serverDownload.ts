// Resolves the `nucle-lsp` binary to run, in priority order:
//   1. An explicit `nuclescript.serverPath` setting (anything other than
//      the bare default command name) -- the user already told us where
//      it is, so don't second-guess that with a download.
//   2. `nucle-lsp` on PATH -- the normal case for a developer working in
//      this repo who built it with `cargo build -p nucle_lsp`.
//   3. A prebuilt binary for this platform, downloaded once from the
//      project's GitHub Releases and cached in the extension's global
//      storage -- the case a marketplace install (no local Rust
//      toolchain) needs.
//
// Uses Node's built-in `https` module rather than the global `fetch`
// some VS Code versions bundle, since `engines.vscode` here doesn't
// guarantee a Node runtime new enough to have it.

import * as fs from "fs";
import * as https from "https";
import * as path from "path";
import * as vscode from "vscode";

const RELEASE_REPO = "VyomKulshrestha/Nucle-OS";
const DEFAULT_SERVER_PATH = "nucle-lsp";

function isOnPath(command: string): boolean {
  const exts = process.platform === "win32" ? [".exe", ".cmd", ""] : [""];
  const pathDirs = (process.env.PATH ?? "").split(path.delimiter);
  for (const dir of pathDirs) {
    for (const ext of exts) {
      if (fs.existsSync(path.join(dir, command + ext))) {
        return true;
      }
    }
  }
  return false;
}

/** The release asset name published for this platform/architecture, or
 * `undefined` if no prebuilt binary exists for it (arm64 Linux/Windows,
 * for instance) -- caller falls back to asking the user to build it. */
function platformAssetName(): string | undefined {
  const { platform, arch } = process;
  if (platform === "win32" && arch === "x64") return "nucle-lsp-windows-x64.exe";
  if (platform === "linux" && arch === "x64") return "nucle-lsp-linux-x64";
  if (platform === "darwin" && arch === "x64") return "nucle-lsp-macos-x64";
  if (platform === "darwin" && arch === "arm64") return "nucle-lsp-macos-arm64";
  return undefined;
}

function httpsGetFollowingRedirects(url: string, redirectsLeft = 5): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    https
      .get(url, { headers: { "User-Agent": "nuclescript-vscode-extension" } }, (res) => {
        const { statusCode, headers } = res;
        if (statusCode && statusCode >= 300 && statusCode < 400 && headers.location) {
          res.resume();
          if (redirectsLeft <= 0) {
            reject(new Error("Too many redirects while downloading nucle-lsp"));
            return;
          }
          httpsGetFollowingRedirects(headers.location, redirectsLeft - 1).then(resolve, reject);
          return;
        }
        if (statusCode !== 200) {
          res.resume();
          reject(new Error(`Failed to download nucle-lsp: HTTP ${statusCode}`));
          return;
        }
        const chunks: Buffer[] = [];
        res.on("data", (chunk: Buffer) => chunks.push(chunk));
        res.on("end", () => resolve(Buffer.concat(chunks)));
        res.on("error", reject);
      })
      .on("error", reject);
  });
}

async function downloadServerBinary(context: vscode.ExtensionContext, assetName: string): Promise<string> {
  const destDir = context.globalStorageUri.fsPath;
  const destPath = path.join(destDir, assetName);
  if (fs.existsSync(destPath)) {
    return destPath;
  }

  const version = (context.extension.packageJSON as { version: string }).version;
  const url = `https://github.com/${RELEASE_REPO}/releases/download/nucle-lsp-v${version}/${assetName}`;

  await vscode.window.withProgress(
    { location: vscode.ProgressLocation.Notification, title: "Downloading NucleScript language server..." },
    async () => {
      const bytes = await httpsGetFollowingRedirects(url);
      await fs.promises.mkdir(destDir, { recursive: true });
      await fs.promises.writeFile(destPath, bytes);
      if (process.platform !== "win32") {
        await fs.promises.chmod(destPath, 0o755);
      }
    },
  );

  return destPath;
}

export async function resolveServerPath(context: vscode.ExtensionContext): Promise<string> {
  const config = vscode.workspace.getConfiguration("nuclescript");
  const configured = config.get<string>("serverPath", DEFAULT_SERVER_PATH);

  if (configured !== DEFAULT_SERVER_PATH) {
    return configured;
  }
  if (isOnPath(configured)) {
    return configured;
  }

  const assetName = platformAssetName();
  if (!assetName) {
    throw new Error(
      `No prebuilt nucle-lsp binary is published for ${process.platform}/${process.arch}. ` +
        `Build it yourself (cargo build -p nucle_lsp --release) and set the "nuclescript.serverPath" ` +
        `setting to the binary's path.`,
    );
  }

  return downloadServerBinary(context, assetName);
}
