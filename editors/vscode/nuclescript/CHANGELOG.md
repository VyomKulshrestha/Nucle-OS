# Changelog

All notable changes to the NucleScript VS Code extension are documented
here. Versions correspond to `editors/vscode/nuclescript/package.json`'s
`version` field, not the main `Nucle-OS` repository's own versioning.

## [Unreleased]

- Nothing yet.

## [0.1.2]

- **Added `NucleScript: Run File`** (▷ button in the editor title bar,
  `Ctrl+F5`/`Cmd+F5`, command palette, Explorer context menu) — runs
  `nucle-cli run <file>` in an integrated terminal, so a program can
  actually be executed (encode/store/retrieve/simulate) from the editor,
  not just checked or formatted.
- **`nucle-cli` is now auto-downloaded**, the same way `nucle-lsp` already
  was: on first use (formatting or Run File) it's looked up on `PATH`,
  and if not found there, a prebuilt binary is fetched once from the
  NucleOS project's GitHub Releases and cached (`src/cliDownload.ts`,
  sharing the fetch/cache logic in the new `src/download.ts` with
  `serverDownload.ts`). Previously `nucle-cli` had no download path at
  all — the README said to build it from source or find it on `PATH`,
  which was the actual gap behind "I installed the extension, now what,
  there's no compiler."
- Added a "Getting started" section to the README with a runnable example
  and documented that the four official `@nuclescript/*` packages resolve
  for hover/diagnostics with no install step, since they're compiled
  directly into `nucle-lsp` and `nucle-cli`.

## [0.1.1]

- **Rewrote the README for the audience that actually reads it.** The
  0.1.0 README was written for repo contributors (local dev symlink
  setup, grammar-snapshot testing, publishing steps) with no link back
  to the source repository at all — exactly backwards for a page whose
  primary readers are end users deciding whether to install, not people
  building the extension. Split it: README.md is now a short, install-
  focused page (features, requirements, settings, troubleshooting, a
  prominent repo link), and the contributor-facing content moved to a
  new `CONTRIBUTING.md` that isn't bundled into the `.vsix`.
- Corrected the README to reflect that the extension is actually
  published (it previously said "isn't published yet" even after the
  first upload) and documented the real update process — a manual
  `.vsix` upload for a docs/version-only change, or the CI workflow (now
  fixed — see below) for anything touching `nucle_lsp` itself.
- Fixed `.github/workflows/release-vscode-extension.yml`: a
  `secrets.VSCE_PAT` reference inside a step's `if:` condition isn't a
  recognized expression at all, which silently broke the *entire*
  workflow's parsing — every push to `main` logged an instant phantom
  failure, and a real `nucle-lsp-v*` tag push produced no run whatsoever.
  Moved the "publish only if configured" check into the shell script
  itself, and added a `workflow_dispatch` trigger for re-running an
  existing tag manually.
- Dropped the Intel Mac (`macos-13`) build from the release workflow —
  GitHub's shared runner pool for it became effectively unavailable
  (observed directly: two separate runs stuck queued on that exact job
  for 30+ minutes, while every other platform in the same run finished
  in under 2.5 minutes). `src/serverDownload.ts` now correctly falls back
  to "build it yourself" for Intel Mac instead of a raw download 404;
  Apple Silicon Macs are unaffected.

## [0.1.0]

Initial release.

- **Syntax highlighting** — a TextMate grammar
  (`syntaxes/nuclescript.tmLanguage.json`) covering every keyword, type,
  profile/codec constant, string, and number literal form (`3x`, `99.5%`,
  size-in-bytes, dates) the compiler accepts, derived directly from
  `nucle_lang`'s lexer/parser rather than invented independently.
  Snapshot-tested against every file in `docs/examples/`.
- **Language server** (`nucle_lsp`, spawned over stdio via
  `vscode-languageclient`):
  - Live diagnostics — the same errors/warnings, error codes, and source
    spans `nucle check` reports, published as you type.
  - Hover — pool/function/strand/sequence/binding signatures.
  - Go to definition — jump from a use site to its declaration.
  - Document outline — every top-level symbol.
- **`Format Document` / format-on-save** (`src/formatProvider.ts`) —
  NucleScript's one canonical style, applied by shelling out to
  `nucle-cli fmt -` over stdin. New `nuclescript.cliPath` setting (default
  `nucle-cli`, looked up on `PATH`) points the extension at the CLI
  binary; unlike `nucle-lsp` there's no download fallback for it yet.
- `language-configuration.json` — comment syntax, bracket matching, and
  auto-closing pairs.
- Marketplace packaging: bundled icon, this changelog, a `nucle-lsp`
  binary auto-download step (`src/serverDownload.ts`) so an
  installed-from-marketplace extension doesn't require a local Rust
  toolchain, and a release workflow that builds those binaries for every
  platform.
