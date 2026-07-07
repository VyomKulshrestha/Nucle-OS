# NucleScript for VS Code

Syntax highlighting **and live language server support** for `.nsl`
files — NucleScript, the declarative DNA-storage operations language for
NucleOS. Packaging is marketplace-ready (icon, changelog, license, a
release workflow that builds `nucle-lsp` for every platform, and an
in-extension downloader so an install doesn't need a local Rust
toolchain) — but it **isn't published yet**. That's a deliberate, manual
step; see [Publishing to the Marketplace](#publishing-to-the-marketplace)
below for exactly what's left and why it isn't automated.

## What's included

- TextMate grammar (`syntaxes/nuclescript.tmLanguage.json`) covering every
  keyword, type, constant, string, number literal form (`3x`, `99.5%`,
  `10MB`, dates), and `//` comment the compiler (`nucle_lang/src/lexer.rs`,
  `parser.rs`) actually accepts today.
- `language-configuration.json` — comment syntax, bracket matching, and
  auto-closing pairs for `{}`/`[]`/`()`/`"..."`.
- A minimal client (`src/extension.ts`) that spawns `nucle_lsp` (the
  `nucle-lsp` binary, built from `../../../nucle_lsp`) over stdio and
  connects it via `vscode-languageclient`. This gets you, live as you
  type:
  - **Diagnostics** — the exact same errors/warnings `nucle check` reports,
    with the same error codes and spans.
  - **Hover** — pool/function/strand/sequence/binding signatures.
  - **Go to definition** — jump from a use site to its declaration.
  - **Document outline** — every top-level symbol, for the editor's
    breadcrumb/outline view.
- **`Format Document` / format-on-save** (`src/formatProvider.ts`) —
  NucleScript's one canonical, zero-configuration style (`gofmt`-style),
  applied by shelling out to `nucle-cli fmt -` (the buffer's current
  content, piped over stdin, so it formats unsaved edits too — not
  reimplemented in TypeScript). Enable it the normal VS Code way, e.g. a
  workspace setting:
  ```json
  { "[nuclescript]": { "editor.formatOnSave": true } }
  ```

Not included yet: autocomplete, rename/refactoring, or semantic-token
highlighting (the TextMate grammar already covers highlighting) — see the
repo root for the current implementation plan.

## Building and running the language server

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

## Installing locally

Either way, first install dependencies and compile the client:

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
code --install-extension nuclescript-0.1.0.vsix
```

Either way, restart VS Code (or run "Developer: Reload Window" from the
command palette) and open any `.nsl` file — e.g. anything under
[`docs/examples/`](../../../docs/examples/) — to see it highlighted.

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
npx vscode-tmgrammar-snap -s source.nuclescript -g syntaxes/nuclescript.tmLanguage.json -u ../../../docs/examples/*.nsl   # regenerate snapshots after an intentional grammar change
```

## Publishing to the Marketplace

Everything code-side is in place; what's left is deliberately manual,
since it requires credentials/decisions only the repo owner can make:

1. **Register a publisher.** Create one at the
   [Visual Studio Marketplace publisher management page](https://marketplace.visualstudio.com/manage)
   (needs a Microsoft/Azure DevOps account). The `publisher` field in
   `package.json` is currently `"nuclescript"` — either register that
   exact ID, or update `package.json` to match whatever ID you register.
2. **Create a Personal Access Token** in Azure DevOps scoped to
   `Marketplace (Manage)`, then add it as a repository secret named
   `VSCE_PAT` (GitHub repo → Settings → Secrets and variables → Actions).
3. **Push a release tag** matching `nucle-lsp-v<version>` (the exact
   version in `package.json`, e.g. `nucle-lsp-v0.1.0`). This triggers
   [`.github/workflows/release-vscode-extension.yml`](../../../.github/workflows/release-vscode-extension.yml),
   which:
   - Builds `nucle-lsp` for Windows/Linux/macOS (x64 and arm64) and
     attaches them to a GitHub Release under that tag — this is what
     `src/serverDownload.ts` downloads from for end users.
   - Packages the extension into a `.vsix` (always, as a build artifact,
     so you can sanity-check it even before publishing).
   - Publishes to the Marketplace **only if `VSCE_PAT` is set** — the
     step is a no-op otherwise, so the rest of the workflow (binary
     releases, VSIX packaging) works fine before you've done steps 1-2.

Until step 3 happens for a given version bump, `nuclescript.serverPath`'s
download fallback has nothing to fetch — local development (`nucle-lsp`
on `PATH`, per the section above) is unaffected either way.
