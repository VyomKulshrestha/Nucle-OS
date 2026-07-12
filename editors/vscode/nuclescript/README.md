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
- **Run File** — a ▷ button in the editor title bar (also `Ctrl+F5` /
  `Cmd+F5`, the command palette, and the Explorer right-click menu) that
  actually executes the program: encode, store, retrieve, simulate,
  whatever the file does, with output in an integrated terminal — the
  same as pressing Run in any other language extension.
- **`Result<T, E>` / `?`** — highlighting, diagnostics, hover, and Run
  File all understand NucleScript's error propagation: `store`/`delete`
  can produce a catchable `Result<T, Str>` instead of aborting the whole
  program on failure, unwrapped or propagated with `?`.
- **Generics** — `fn name<T>(...)` over `Pool<T>`'s profile, so the same
  function works for `Illumina`/`Nanopore`/`Twist` instead of needing one
  copy per profile. Highlighting, diagnostics, hover, formatting, and
  `nucle doc` all render and check the type-parameter list correctly.
- **Pattern matching** — `match attempt { Ok(file) => file, Err(reason)
  => ... }` branches on a caught `Result` directly, instead of needing a
  second, independent function call from the caller. Highlighting,
  diagnostics, hover, and formatting all understand `match`/`Ok`/`Err`/
  `=>`.
- **User-defined `enum`s and general pattern matching** — `enum Name {
  Variant, Variant(Type) }` declares a real sum type;
  `EnumName::Variant(payload)` constructs one; `match` now handles both a
  user `enum` and the built-in `Result` pseudo-enum through one general
  engine, with free arm order and enforced exhaustiveness (every declared
  variant needs an arm, or a trailing wildcard `_` covers the rest).
  Highlighting (`enum` as a keyword, `::` for enum-variant construction),
  diagnostics, hover, formatting, and `nucle doc` all understand the new
  syntax.
- **Closures** — `fn(params) -> T { body }` is an anonymous closure
  literal with real lexical capture; `Fn(...) -> T` is its type, usable
  as a `let` annotation or a function parameter's type (genuinely
  higher-order functions). Highlighting, diagnostics, hover, and
  formatting all understand `Fn(...)` and closure literals — including
  generic closures (`fn<T>(...)`) and a closure calling itself by its own
  bound name.
- **Effect-annotated `Fn(...)` types** — `Fn(...) -> T confirm hardware`/
  `confirm physical_key` lets a function declare the effect ceiling its
  `Fn(...)`-typed parameter's call is trusted to have, so calling a
  closure received as a parameter (unlike one bound to a `let`, already
  handled) can finally have its effect analyzed accurately instead of
  being silently assumed `Pure`. No annotation means exactly the
  pre-existing behavior — purely additive, opt-in syntax reusing the same
  `confirm hardware`/`confirm physical_key` tokens already used
  elsewhere. Diagnostics (`E-FN-EFFECT-ARG-MISMATCH`), hover, formatting,
  and `nucle doc` all understand the new syntax.
- **`Ok(...)`/`Err(...)` constructors, explicit `::<Illumina>()` type
  arguments, and real parameter values** — `Result` values can now be
  built directly (`Ok(saved)`, `Err("reason")`), composed with nested
  `match`/`?`, and a generic call can spell out its type argument
  explicitly when inference alone can't resolve it. A statement-form
  `store`/`retrieve`/`delete` inside a function body, and a `File`/`Str`-
  typed parameter's real argument value, now genuinely reach `nucle run`
  — not just type-check. Highlighting, diagnostics, hover, formatting,
  and `nucle doc` all understand the new syntax.

Not yet included: autocomplete, rename/refactoring, and semantic-token
highlighting (syntax highlighting already covers most of that).

## Getting started

1. In an empty folder, create a file named `sample_a.txt` with any text
   in it — `store` below archives a real file, so it needs to exist.
   `.nsl` files resolve `store`/`retrieve` paths relative to their own
   folder, so put it next to `hello.nsl` in the next step.
2. Create `hello.nsl` in the same folder:

   ```nsl
   pool archive: DnaPool {
       codec: Ternary,
       redundancy: 2x,
       profile: Illumina
   }

   store "sample_a.txt" into archive

   retrieve from archive
   ```

3. Click the ▷ **Run File** button in the top-right of the editor (or
   press `Ctrl+F5` / `Cmd+F5`). Nothing else to install: both `nucle-lsp`
   (highlighting/diagnostics/hover) and `nucle-cli` (formatting and
   running) download themselves the first time they're needed, cached
   after that.

You'll see real output in the terminal — the encode/store/retrieve
result, strand counts, redundancy — not just a syntax check. (A "failed
to read... sample_a.txt" error means step 1 was skipped, or the file
ended up in a different folder than `hello.nsl`.)

More complete examples (per-store options, a full simulate → consensus
vote → encode/protect/store/verify pipeline, catching a real store
failure with `Result<T, E>`/`?` instead of aborting the run, one function
generic over three different pool profiles, branching on a caught
`Result` directly with `match`, a genuinely higher-order function with
real closure capture, a self-recursive closure, an explicit
`::<Illumina>()` type argument paired with a real `File`-typed parameter
value, a user-defined `enum` matched via a nested `match` with
exhaustiveness enforced by a trailing wildcard, and an effect-annotated
`Fn(...)` parameter whose confirmed destructive closure actually runs
through two layers of function calls) live in
[`docs/examples/`](https://github.com/VyomKulshrestha/Nucle-OS/tree/main/docs/examples)
in the main repo — start with `store.nsl`, then `hero.nsl`, then
`result_fallback_store.nsl`, then `generic_pool_recovery.nsl`, then
`match_result_fallback.nsl`, then `closure_retry.nsl`, then
`explicit_type_args_and_file_param.nsl`, then `recovery_plan.nsl`, then
`effect_annotated_closure.nsl`.

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

Nothing — the extension is a client for two command-line tools from the
NucleOS project, but both are fetched for you automatically the first
time they're needed:

| Tool | Needed for | If no prebuilt binary exists for your platform |
|---|---|---|
| `nucle-lsp` | diagnostics, hover, go to definition, outline | Windows/Linux x64 and Apple Silicon Mac are covered. Otherwise, build it (`cargo build -p nucle_lsp --release` from a [NucleOS](https://github.com/VyomKulshrestha/Nucle-OS) checkout) and point `nuclescript.serverPath` at it. |
| `nucle-cli` | Format Document / format on save, and **Run File** | Windows/Linux x64 and Mac (Intel or Apple Silicon, via Rosetta) are covered. Otherwise, build it (`cargo build -p nucle_cli --release`) and point `nuclescript.cliPath` at it. |

Already have `nucle-lsp`/`nucle-cli` on `PATH` (e.g. a local Rust build)?
That's used automatically instead of downloading anything — see
[building from source](https://github.com/VyomKulshrestha/Nucle-OS#building).

## Settings

| Setting | Default | Description |
|---|---|---|
| `nuclescript.serverPath` | `nucle-lsp` | Path to the `nucle-lsp` binary. Set an absolute path to override the automatic lookup/download. |
| `nuclescript.cliPath` | `nucle-cli` | Path to the `nucle-cli` binary, used for formatting and **Run File**. Set an absolute path to override the automatic lookup/download. |

## Troubleshooting

- **No diagnostics/hover, or an error naming `nuclescript.serverPath`:**
  the extension couldn't find or download a `nucle-lsp` binary for your
  platform/architecture. Build one (`cargo build -p nucle_lsp --release`
  from a [NucleOS](https://github.com/VyomKulshrestha/Nucle-OS) checkout)
  and point `nuclescript.serverPath` at it.
- **`Format Document` or `Run File` shows an error naming
  `nuclescript.cliPath`:** same thing for `nucle-cli` — build it
  (`cargo build -p nucle_cli --release`) and point `nuclescript.cliPath`
  at the built binary.
- **`Run File` does nothing:** it only runs `.nsl` files — make sure the
  active editor tab is a NucleScript file, and check the "NucleScript"
  terminal panel for the actual error output.

## Contributing

Local development setup, grammar testing, and the release process live
in [CONTRIBUTING.md](https://github.com/VyomKulshrestha/Nucle-OS/blob/main/editors/vscode/nuclescript/CONTRIBUTING.md)
in the repository — not duplicated here since this page is what installs
show, not what contributors need.

## License

MIT — see [LICENSE](https://github.com/VyomKulshrestha/Nucle-OS/blob/main/editors/vscode/nuclescript/LICENSE).
