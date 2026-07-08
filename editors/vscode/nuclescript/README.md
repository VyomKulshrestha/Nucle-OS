# NucleScript for VS Code

Syntax highlighting and a live language server for `.nsl` files —
NucleScript, the declarative DNA-storage operations language for
[NucleOS](https://github.com/VyomKulshrestha/Nucle-OS).

**Repository & issues:** [github.com/VyomKulshrestha/Nucle-OS](https://github.com/VyomKulshrestha/Nucle-OS)

## Features

- **Syntax highlighting** for every NucleScript keyword, type, and
  literal form (`3x`, `99.5%`, `10MB`, dates), kept in sync with the real
  compiler grammar.
- **Live diagnostics** as you type — the same errors/warnings, error
  codes, and source spans the `nucle check` CLI command reports.
- **Hover** for pool/function/strand/sequence/binding signatures.
- **Go to Definition** — jump from a use site to its declaration.
- **Document outline** — every top-level symbol, for the breadcrumb and
  outline views.
- **Format Document / format on save** — NucleScript's one canonical,
  zero-configuration style. Enable format-on-save the normal VS Code way:
  ```json
  { "[nuclescript]": { "editor.formatOnSave": true } }
  ```

Not yet included: autocomplete, rename/refactoring, and semantic-token
highlighting (syntax highlighting already covers most of that).

## Requirements

This extension is a client for two command-line tools from the NucleOS
project — it doesn't bundle a compiler itself:

| Tool | Needed for | If it's not found |
|---|---|---|
| `nucle-lsp` | diagnostics, hover, go to definition, outline | Downloaded automatically for your OS/architecture on first use — nothing to install manually in most cases. |
| `nucle-cli` | `Format Document` / format on save | Must be on your `PATH`, or pointed at via the `nuclescript.cliPath` setting (see below). No auto-download yet. |

If you already have a Rust toolchain and want to build these yourself
instead of using the downloaded/`PATH` copy, see
[building from source](https://github.com/VyomKulshrestha/Nucle-OS#building).

## Settings

| Setting | Default | Description |
|---|---|---|
| `nuclescript.serverPath` | `nucle-lsp` | Path to the `nucle-lsp` binary. Set an absolute path to override the automatic lookup/download. |
| `nuclescript.cliPath` | `nucle-cli` | Path to the `nucle-cli` binary, used for formatting. Set an absolute path if it isn't on `PATH`. |

## Troubleshooting

- **No diagnostics/hover, or an error naming `nuclescript.serverPath`:**
  the extension couldn't find or download a `nucle-lsp` binary for your
  platform. Build one (`cargo build -p nucle_lsp --release` from a
  [NucleOS](https://github.com/VyomKulshrestha/Nucle-OS) checkout) and
  point `nuclescript.serverPath` at it.
- **`Format Document` shows an error naming `nuclescript.cliPath`:**
  `nucle-cli` isn't on `PATH`. Build it
  (`cargo build -p nucle_cli --release`) and either add it to `PATH` or
  set `nuclescript.cliPath` to the built binary's location.

## Contributing

Local development setup, grammar testing, and the release process live
in [CONTRIBUTING.md](https://github.com/VyomKulshrestha/Nucle-OS/blob/main/editors/vscode/nuclescript/CONTRIBUTING.md)
in the repository — not duplicated here since this page is what installs
show, not what contributors need.

## License

MIT — see [LICENSE](https://github.com/VyomKulshrestha/Nucle-OS/blob/main/editors/vscode/nuclescript/LICENSE).
