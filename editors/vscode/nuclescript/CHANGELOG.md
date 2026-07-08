# Changelog

All notable changes to the NucleScript VS Code extension are documented
here. Versions correspond to `editors/vscode/nuclescript/package.json`'s
`version` field, not the main `Nucle-OS` repository's own versioning.

## [Unreleased]

- Nothing yet.

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
