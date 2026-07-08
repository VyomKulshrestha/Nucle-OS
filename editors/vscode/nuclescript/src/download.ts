// Shared "download once, cache in global storage" logic for prebuilt
// binaries (nucle-lsp, nucle-cli) fetched from GitHub Releases -- the
// marketplace-install path that doesn't require a local Rust toolchain.
// Split out of serverDownload.ts so cliDownload.ts can reuse it instead
// of duplicating the redirect-following HTTPS fetch.

import * as fs from "fs";
import * as https from "https";
import * as path from "path";
import * as vscode from "vscode";

export function isOnPath(command: string): boolean {
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

function httpsGetFollowingRedirects(url: string, redirectsLeft = 5): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    https
      .get(url, { headers: { "User-Agent": "nuclescript-vscode-extension" } }, (res) => {
        const { statusCode, headers } = res;
        if (statusCode && statusCode >= 300 && statusCode < 400 && headers.location) {
          res.resume();
          if (redirectsLeft <= 0) {
            reject(new Error(`Too many redirects while downloading ${url}`));
            return;
          }
          httpsGetFollowingRedirects(headers.location, redirectsLeft - 1).then(resolve, reject);
          return;
        }
        if (statusCode !== 200) {
          res.resume();
          reject(new Error(`Failed to download ${url}: HTTP ${statusCode}`));
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

export async function downloadAndCacheBinary(
  context: vscode.ExtensionContext,
  url: string,
  assetName: string,
  progressTitle: string,
): Promise<string> {
  const destDir = context.globalStorageUri.fsPath;
  const destPath = path.join(destDir, assetName);
  if (fs.existsSync(destPath)) {
    return destPath;
  }

  await vscode.window.withProgress(
    { location: vscode.ProgressLocation.Notification, title: progressTitle },
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
