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

import * as vscode from "vscode";
import { downloadAndCacheBinary, isOnPath } from "./download";

const RELEASE_REPO = "VyomKulshrestha/Nucle-OS";
const DEFAULT_SERVER_PATH = "nucle-lsp";

/** The release asset name published for this platform/architecture, or
 * `undefined` if no prebuilt binary exists for it (arm64 Linux/Windows,
 * or Intel Mac -- GitHub's shared runner pool for `macos-13` became
 * effectively unavailable, so that build was dropped from the release
 * workflow) -- caller falls back to asking the user to build it, rather
 * than this returning a name that would just 404. */
function platformAssetName(): string | undefined {
  const { platform, arch } = process;
  if (platform === "win32" && arch === "x64") return "nucle-lsp-windows-x64.exe";
  if (platform === "linux" && arch === "x64") return "nucle-lsp-linux-x64";
  if (platform === "darwin" && arch === "arm64") return "nucle-lsp-macos-arm64";
  return undefined;
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

  const version = (context.extension.packageJSON as { version: string }).version;
  const url = `https://github.com/${RELEASE_REPO}/releases/download/nucle-lsp-v${version}/${assetName}`;
  return downloadAndCacheBinary(context, url, assetName, "Downloading NucleScript language server...");
}
