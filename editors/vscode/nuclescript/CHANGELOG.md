# Changelog

All notable changes to the NucleScript VS Code extension are documented
here. Versions correspond to `editors/vscode/nuclescript/package.json`'s
`version` field, not the main `Nucle-OS` repository's own versioning.

## [Unreleased]

- **`Format Document` / format-on-save** (`src/formatProvider.ts`) —
  NucleScript's one canonical style, applied by shelling out to
  `nucle-cli fmt -` over stdin. New `nuclescript.cliPath` setting (default
  `nucle-cli`, looked up on `PATH`) points the extension at the CLI
  binary; unlike `nucle-lsp` there's no download fallback for it yet.
- Marketplace packaging groundwork: bundled icon, this changelog, and a
  `nucle-lsp` binary auto-download step (`src/serverDownload.ts`) so an
  installed-from-marketplace extension doesn't require a local Rust
  toolchain. Publishing itself (registering a publisher, running `vsce
  publish`) is a deliberate manual step — see the extension README.

## [0.1.0]

Initial local/dev release.

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
- `language-configuration.json` — comment syntax, bracket matching, and
  auto-closing pairs.
