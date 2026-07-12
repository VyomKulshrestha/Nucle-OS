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
// whenever a newer `v*` release ships with an updated nucle-cli, or a
// fresh install downloading it will fail to parse -- or silently
// mis-execute -- any .nsl file using syntax added since the pinned tag.
// v0.1.8 covers user-defined enums/general `match` (v0.1.6), effect-
// annotated `Fn(...)` types (v0.1.7), and concurrent hardware submission
// (v0.1.8, no new .nsl syntax) -- the two syntax-adding releases were
// missed here for a while (this constant is separate from nucle-lsp's
// own version-templated download in serverDownload.ts, which tracks the
// extension's own version automatically and was never at risk).
const CLI_RELEASE_TAG = "v0.1.8";

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
