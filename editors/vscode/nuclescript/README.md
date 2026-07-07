# NucleScript for VS Code

Syntax highlighting for `.nsl` files — NucleScript, the declarative
DNA-storage operations language for NucleOS. This is a local/dev
extension for now; it isn't published to the Marketplace or Open VSX.

## What's included

- TextMate grammar (`syntaxes/nuclescript.tmLanguage.json`) covering every
  keyword, type, constant, string, number literal form (`3x`, `99.5%`,
  `10MB`, dates), and `//` comment the compiler (`nucle_lang/src/lexer.rs`,
  `parser.rs`) actually accepts today.
- `language-configuration.json` — comment syntax, bracket matching, and
  auto-closing pairs for `{}`/`[]`/`()`/`"..."`.

Not included yet: live diagnostics, hover, go-to-definition, or
autocomplete — those need a language server (tracked as a later step; see
the repo root for the current implementation plan).

## Installing locally

**Option A — symlink for active development** (changes to the grammar
take effect after a "Developer: Reload Window" in VS Code, no repackaging
needed):

```bash
# macOS/Linux
ln -s "$(pwd)/editors/vscode/nuclescript" ~/.vscode/extensions/nuclescript-dev

# Windows (PowerShell, run as your normal user)
New-Item -ItemType SymbolicLink -Path "$env:USERPROFILE\.vscode\extensions\nuclescript-dev" -Target "$(Get-Location)\editors\vscode\nuclescript"
```

**Option B — package and install a VSIX** (closer to how a real install
would behave, but requires repackaging after every grammar change):

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
