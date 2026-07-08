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

## Getting started

This extension is editor tooling — highlighting, checking, hovers,
navigation, formatting — not a compiler in itself. Here's the shortest
path to seeing it work, and to actually running a program.

1. Create a file named `hello.nsl`:

   ```nsl
   pool archive: DnaPool {
       codec: Ternary,
       redundancy: 2x,
       profile: Illumina
   }

   store "sample_a.txt" into archive

   retrieve from archive
   ```

   Highlighting, live diagnostics, and hover work immediately — `nucle-lsp`
   downloads itself the first time you open a `.nsl` file, nothing to
   install.
2. To *run* it — actually encode/store/retrieve, not just check it — you
   need the `nucle-cli` binary. It isn't auto-downloaded yet; see
   Requirements below for where to get one, then run
   `nucle-cli run hello.nsl`.

More complete examples (per-store options, a full simulate → consensus
vote → encode/protect/store/verify pipeline) live in
[`docs/examples/`](https://github.com/VyomKulshrestha/Nucle-OS/tree/main/docs/examples)
in the main repo — start with `store.nsl`, then `hero.nsl`.

**Using the official packages:** imports like `from "nuclescript/presets"`
(`@nuclescript/presets`, `@nuclescript/profiles`, `@nuclescript/benchmarks`,
`@nuclescript/recovery` — see the
[package registry](https://github.com/orgs/Nuclescript/packages))
get full hover and diagnostics with no install step: those four packages
are compiled directly into both `nucle-lsp` and `nucle-cli`, not fetched
over the network. `nucle-cli package install/lock/verify` is only for
generating a `nucle.lock` for reproducible builds — the editor doesn't
need it.

## Requirements

This extension is a client for two command-line tools from the NucleOS
project — it doesn't bundle a compiler itself:

| Tool | Needed for | If it's not found |
|---|---|---|
| `nucle-lsp` | diagnostics, hover, go to definition, outline | Downloaded automatically for your OS/architecture on first use — nothing to install manually in most cases. |
| `nucle-cli` | `Format Document` / format on save, and actually running NucleScript programs (`nucle-cli run`, `store`, `retrieve`, `simulate`, ...) | Not auto-downloaded yet. Grab a prebuilt binary for Windows/Linux/macOS from the [NucleOS release](https://github.com/VyomKulshrestha/Nucle-OS/releases/tag/v0.1.0), put it on your `PATH`, or point `nuclescript.cliPath` at it. |

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
