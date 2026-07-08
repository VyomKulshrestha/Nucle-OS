# Contributing to the NucleScript VS Code extension

This is developer-facing documentation for working on the extension
itself — for using it, see [README.md](README.md) (the same file shown
on the Marketplace listing).

## Building and running the language server locally

The extension needs a `nucle-lsp` binary, resolved in this order (see
`src/serverDownload.ts`):

1. An explicit `nuclescript.serverPath` setting (VS Code Settings → search
   "nuclescript") — always wins if set to anything other than the default.
2. `nucle-lsp` on `PATH` — the normal case for local development. Build it
   from the repo root with:
   ```bash
   cargo build -p nucle_lsp --release
   ```
   then put `target/release/` (or `target/debug/`) on `PATH`.
3. Otherwise, a prebuilt binary for your OS/architecture is downloaded
   once from this repo's GitHub Releases (tag `nucle-lsp-v<version>`,
   matching this extension's own `package.json` version) and cached in
   the extension's global storage — this is what makes a marketplace
   install work without a local Rust toolchain. If no prebuilt binary
   exists for your platform, the extension shows an error telling you to
   build one and point `nuclescript.serverPath` at it.

Formatting needs a separate `nucle-cli` binary (the language server and
CLI are different executables), resolved via the `nuclescript.cliPath`
setting (default `nucle-cli`, looked up on `PATH`). Build it with:
```bash
cargo build -p nucle_cli --release
```
Unlike `nucle-lsp`, there's currently no download fallback for
`nucle-cli` — if it isn't on `PATH`, `Format Document` shows an error
naming the setting to point at it instead.

## Installing your local build

First install dependencies and compile the client:

```bash
cd editors/vscode/nuclescript
npm install
npm run compile
```

**Option A — symlink for active development** (grammar changes take
effect after a "Developer: Reload Window" in VS Code with no rebuild;
client (`.ts`) changes need `npm run compile` first, then reload):

```bash
# macOS/Linux
ln -s "$(pwd)/editors/vscode/nuclescript" ~/.vscode/extensions/nuclescript-dev

# Windows (PowerShell, run as your normal user)
New-Item -ItemType SymbolicLink -Path "$env:USERPROFILE\.vscode\extensions\nuclescript-dev" -Target "$(Get-Location)\editors\vscode\nuclescript"
```

**Option B — package and install a VSIX** (closer to how a real install
would behave, but requires repackaging after every change):

```bash
cd editors/vscode/nuclescript
npm install -g @vscode/vsce
vsce package
code --install-extension nuclescript-<version>.vsix
```

Either way, restart VS Code (or run "Developer: Reload Window" from the
command palette) and open any `.nsl` file — e.g. anything under
[`docs/examples/`](../../docs/examples/) — to see it highlighted.

## Testing the grammar

`npm test` runs [`vscode-tmgrammar-snap`](https://github.com/PanAeon/vscode-tmgrammar-test)
against every file in `docs/examples/`, snapshotting the token scope for
each line. Snapshots live alongside the tested files
(`docs/examples/*.nsl.snap`) and are checked into the repo — a change to
the grammar (or to the compiler's keyword set, if the grammar isn't
updated to match) that alters tokenization shows up as a diff, not a
silent regression.

```bash
cd editors/vscode/nuclescript
npm install
npm test              # compare against committed snapshots
npx vscode-tmgrammar-snap -s source.nuclescript -g syntaxes/nuclescript.tmLanguage.json -u ../../docs/examples/*.nsl   # regenerate snapshots after an intentional grammar change
```

## Publishing an update to the Marketplace

The publisher (`nuclescript`) is registered and the extension is live.
There are two ways to ship a new version, and either works:

**Manual** — no PAT, no Azure DevOps, nothing beyond the Marketplace UI:

```bash
cd editors/vscode/nuclescript
npm install
npx @vscode/vsce package
```

Bump `"version"` in `package.json` first (the Marketplace won't accept
re-uploading an already-published version number). Then, on the
[publisher management page](https://marketplace.visualstudio.com/manage/publishers/nuclescript),
open the NucleScript extension → **Update** → drag in the new `.vsix`.

**Automated, via CI** — for a version bump that also needs new
`nucle-lsp` binaries (any change to `nucle_lsp` itself):

1. **Create a Personal Access Token** in Azure DevOps scoped to
   `Marketplace (Manage)`, then add it as a repository secret named
   `VSCE_PAT` (GitHub repo → Settings → Secrets and variables → Actions)
   — one-time setup, skip if already done.
2. **Push a release tag** matching `nucle-lsp-v<version>` (the exact
   version in `package.json`, e.g. `nucle-lsp-v0.1.1`). This triggers
   [`.github/workflows/release-vscode-extension.yml`](../../.github/workflows/release-vscode-extension.yml)
   (also runnable manually via `workflow_dispatch` for an existing tag),
   which builds `nucle-lsp` for Windows/Linux/macOS (x64 and arm64) and
   attaches them to a GitHub Release under that tag — what
   `src/serverDownload.ts` downloads from for end users — packages the
   extension into a `.vsix`, and publishes to the Marketplace if
   `VSCE_PAT` is set (a no-op otherwise, so the binary release and VSIX
   packaging still work without it).

Either way, a `nucle-lsp-v<version>` tag needs to exist with binaries
attached for `nuclescript.serverPath`'s download fallback to have
anything to fetch for that version — local development (`nucle-lsp` on
`PATH`, per the section above) is unaffected either way.
