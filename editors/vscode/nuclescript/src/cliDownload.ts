// Resolves the `nucle-cli` binary, used both for `Format Document` and
// for `NucleScript: Run File`. Same three-tier resolution as
// serverDownload.ts (explicit setting -> PATH -> download-and-cache), but
// the download comes from the main NucleOS repo's own release tags
// (`v<version>`) rather than `nucle-lsp-v<extension-version>` --
// nucle-cli ships as part of NucleOS itself and is versioned
// independently of this extension.

import * as vscode from "vscode";
import { downloadAndCacheBinary, isOnPath } from "./download";

const RELEASE_REPO = "VyomKulshrestha/Nucle-OS";
const DEFAULT_CLI_PATH = "nucle-cli";

// Pinned to the newest NucleOS release known to publish nucle-cli
// binaries for every platform (.github/workflows/release.yml). Bump this
// when a newer `v*` release ships with an updated nucle-cli -- e.g.
// v0.1.2 exists specifically because v0.1.1's nucle-cli predates
// generics (`fn name<T>(...)` over `Pool<T>`'s profile) entirely, so a
// fresh install downloading it would fail to parse any .nsl file using
// the new syntax.
const CLI_RELEASE_TAG = "v0.1.2";

/** The release asset name published for this platform, or `undefined` if
 * none exists (arm64 Linux/Windows) -- caller falls back to asking the
 * user to build it themselves. */
function platformAssetName(): string | undefined {
  const { platform, arch } = process;
  if (platform === "win32" && arch === "x64") return "nucle-cli-windows-x86_64.exe";
  if (platform === "linux" && arch === "x64") return "nucle-cli-linux-x86_64";
  // One asset covers both Mac architectures: Rosetta 2 runs an x86_64
  // binary transparently on Apple Silicon.
  if (platform === "darwin") return "nucle-cli-macos-x86_64";
  return undefined;
}

export async function resolveCliPath(context: vscode.ExtensionContext): Promise<string> {
  const config = vscode.workspace.getConfiguration("nuclescript");
  const configured = config.get<string>("cliPath", DEFAULT_CLI_PATH);

  if (configured !== DEFAULT_CLI_PATH) {
    return configured;
  }
  if (isOnPath(configured)) {
    return configured;
  }

  const assetName = platformAssetName();
  if (!assetName) {
    throw new Error(
      `No prebuilt nucle-cli binary is published for ${process.platform}/${process.arch}. ` +
        `Build it yourself (cargo build -p nucle_cli --release) and set the "nuclescript.cliPath" ` +
        `setting to the binary's path.`,
    );
  }

  const url = `https://github.com/${RELEASE_REPO}/releases/download/${CLI_RELEASE_TAG}/${assetName}`;
  return downloadAndCacheBinary(context, url, assetName, "Downloading nucle-cli...");
}
