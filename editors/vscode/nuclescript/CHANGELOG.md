# Changelog

All notable changes to the NucleScript VS Code extension are documented
here. Versions correspond to `editors/vscode/nuclescript/package.json`'s
`version` field, not the main `Nucle-OS` repository's own versioning.

## [0.1.18]

- Requires VS Code 1.91 or later: the language client moved to
  `vscode-languageclient` 10.
- A closure passed to a `Fn(...)` parameter, inline or `let`-bound, is
  seen through: the call carries the closure's real effect, and `nucle
  plan` narrates the store inside it where the function calls the
  parameter.
- Run File on ternary, Yin-Yang and Fountain pools writes Reed-Solomon
  parity strands the length of the data strands instead of four times it,
  and the decoder corrects wrong strands wherever they sit, so files come
  back through noisier reads with less DNA.
- Advanced the clean-install `nucle-cli` fallback to the paired NucleOS
  v0.1.53 runtime, which contains all of the above.

## [0.1.17]

- `codec: Hedges` pools and `encode ... using hedges` pipelines are
  highlighted and run: Run File stores the file with the HEDGES codec,
  which decodes each sequencing read on its own through insertions and
  deletions and writes its Reed-Solomon parity as HEDGES strands too.
- `storage: silica at 4C` (alias `sio2`) is highlighted, and retention
  requirements on silica pools are checked against the silica-particle
  decay kinetics of Grass et al. 2015.
- Advanced the clean-install `nucle-cli` fallback to the paired NucleOS
  v0.1.46 runtime, the first that knows both.

## [0.1.16]

- Run File and diagnostics understand real package imports: an imported
  pool, recovery profile or function is part of the program, imported
  functions accept declared pools and recovery profiles as arguments, and
  diagnostics in imported code point at the import line.
- `for` loops over file names pass each name to functions, loop bodies can
  bind a `let` every iteration, closures returned from functions are
  callable, and named functions can recurse.
- Added the runnable `package_workflow.nsl` example.
- Advanced the clean-install `nucle-cli` fallback to the paired NucleOS
  v0.1.35 runtime, which contains all of the above.

## [0.1.15]

- Highlighting for pool `storage: <medium> at <temp>C` conditions and store
  `retain: <years>y` requirements: the `storage`, `retain` and `at`
  keywords, the `encapsulated`/`bone` media (and their aliases), and signed
  temperature and duration literals such as `-20C` and `1000y`.
- Added the runnable `retention_policy.nsl` example and refreshed grammar
  snapshots for the new `tiered_archive.nsl` and `wet_lab_run.nsl` examples.
- Advanced the clean-install `nucle-cli` fallback to the paired NucleOS
  v0.1.32 runtime, the first whose compiler understands `storage`/`retain`,
  each pool as its own namespace, and pool-valued `match` typing.

## [0.1.14]

- Added `IDT`, `idtopools`, and `idt-opools` highlighting, backed by the
  compiler's typed `Pool<IDT, E>` profile and IDT-specific synthesis checks.
- Added a runnable IDT vendor example and refreshed all TextMate grammar
  snapshots.
- Prepared the clean-install CLI fallback for the paired NucleOS v0.1.28
  release so Run File and formatting use a compiler that understands IDT once
  that release's binaries are published.

## [0.1.13]

- Fountain pool declarations and pipeline encode steps now run through the
  real VFS backend instead of emitting unsupported-codec warnings or failing
  at execution time. The syntax grammar already recognized every Fountain
  alias; its documentation and snapshots now include the runnable
  `fountain_roundtrip.nsl` example and state the rank-dependent recovery limit.
- Advanced the clean-install `nucle-cli` fallback to the paired NucleOS
  v0.1.27 runtime, so **Run File** uses a binary that contains this Fountain
  execution path instead of silently falling back to pre-Fountain v0.1.26.
- Added deterministic integration coverage through the NucleScript/VFS path:
  four missing data droplets recover at the four-parity Reed-Solomon boundary,
  while a survivor set with no parity and fewer droplets than source segments
  fails in Fountain decoding as necessarily rank-deficient.

## [0.1.12]

- Added context-aware completion, find references, guarded
  prepare/rename, semantic tokens, and UTF-16-correct LSP ranges.
- Added native VS Code Test Explorer integration backed by
  `nucle test --json`, including source-mapped failures, dirty-file saves,
  cancellation, and unit-tested top-level test discovery.
- Updated the bundled CLI fallback to the verified NucleOS v0.1.26
  release so formatting, running, and Test Explorer use current syntax.
- Refreshed all 24 TextMate grammar snapshots and patched the transitive
  `brace-expansion` vulnerability; `npm audit` now reports zero findings.
- Aligned editor behavior with the compiler's completed value checking:
  whole-program function resolution, ordinary argument/return validation,
  scalar Result/enum/match values, and strict `protect` typing.

## [0.1.11]

- **Fixed: the auto-downloaded `nucle-cli` binary was stale.**
  `src/cliDownload.ts`'s pinned release tag hadn't been bumped since
  v0.1.5, so a fresh install's "Run File"/"Format Document" silently
  downloaded a `nucle-cli` that predates user-defined `enum`s/`match`
  (main repo v0.1.6) and effect-annotated `Fn(...)` types (v0.1.7)
  entirely — it would fail to parse either syntax, including this
  extension's own example files
  ([`recovery_plan.nsl`](https://github.com/VyomKulshrestha/Nucle-OS/blob/main/docs/examples/recovery_plan.nsl),
  [`effect_annotated_closure.nsl`](https://github.com/VyomKulshrestha/Nucle-OS/blob/main/docs/examples/effect_annotated_closure.nsl)).
  This was later advanced to the verified v0.1.26 release in v0.1.12.

## [0.1.10]

- **`Fn(...)` types can now declare an effect ceiling.** `Fn(...) -> T
  confirm hardware`/`confirm physical_key` lets a function declare what
  effect its `Fn(...)`-typed parameter's call is trusted to have, closing
  the one real gap closures left open: a call to a closure received as a
  parameter couldn't have its effect analyzed before, since the concrete
  closure a caller passes isn't known until runtime — silently treated as
  `Pure` even if genuinely destructive. No annotation means exactly
  today's behavior; this is purely additive, opt-in syntax reusing the
  same `confirm hardware`/`confirm physical_key` tokens already used
  elsewhere, so highlighting already understands it with no grammar
  changes. Diagnostics (`E-FN-EFFECT-ARG-MISMATCH`), hover, formatting,
  and `nucle doc` all understand the new syntax — see the updated README
  and
  [`docs/examples/effect_annotated_closure.nsl`](https://github.com/VyomKulshrestha/Nucle-OS/blob/main/docs/examples/effect_annotated_closure.nsl)
  in the main repo.
- Bumped the downloaded `nucle-lsp`/`nucle-cli` binaries to the release
  that actually understands the new syntax, same reason as every prior
  language-feature bump.

## [0.1.9]

- **NucleScript now supports user-defined `enum`s and a general
  pattern-matching/exhaustiveness engine.** `enum Name { Variant,
  Variant(Type) }` declares a real sum type; `EnumName::Variant(payload)`
  constructs one; `match` now handles both a user `enum` and the built-in
  `Result` pseudo-enum through one general engine — arm order is free
  (checked by variant name, not position), and exhaustiveness is enforced
  (every declared variant needs an arm, or a trailing wildcard `_` covers
  the rest). Highlighting (`enum` as a keyword, `::` for both enum-variant
  construction and turbofish), live diagnostics, hover, formatting, and
  `nucle doc` all understand the new syntax — see the updated README and
  [`docs/examples/recovery_plan.nsl`](https://github.com/VyomKulshrestha/Nucle-OS/blob/main/docs/examples/recovery_plan.nsl)
  in the main repo.
- Bumped the downloaded `nucle-lsp`/`nucle-cli` binaries to the release
  that actually understands the new syntax, same reason as every prior
  language-feature bump.

## [0.1.8]

- **Closed the deliberate gaps left by `Result<T, E>`/`?`, generics,
  pattern matching, and closures.** `Ok(...)`/`Err(...)` now construct a
  `Result` directly and compose with `match`/`?` (nested `match`, `?`
  applied straight to a `match` expression, `Ok`/`Err` as match-arm
  bodies); a generic call can spell out an explicit `name::<Illumina>()`
  type argument for the one case inference alone can't resolve; a
  statement-form `store`/`retrieve`/`delete` inside a function body, and
  a `File`/`Str`-typed parameter's real argument value, now genuinely
  reach `nucle run` instead of being silently inert; closures can be
  generic (`fn<T>(...)`) and self-recursive (calling themselves by their
  own bound name); and `nucle plan`/`nucle explain` now narrate through a
  `let`-bound closure's own body. Highlighting, live diagnostics, hover,
  formatting, and `nucle doc` all understand the new syntax — see the
  updated README and the extended
  [`result_fallback_store.nsl`](https://github.com/VyomKulshrestha/Nucle-OS/blob/main/docs/examples/result_fallback_store.nsl)/
  [`generic_pool_recovery.nsl`](https://github.com/VyomKulshrestha/Nucle-OS/blob/main/docs/examples/generic_pool_recovery.nsl)/
  [`match_result_fallback.nsl`](https://github.com/VyomKulshrestha/Nucle-OS/blob/main/docs/examples/match_result_fallback.nsl)/
  [`closure_retry.nsl`](https://github.com/VyomKulshrestha/Nucle-OS/blob/main/docs/examples/closure_retry.nsl),
  plus the new
  [`explicit_type_args_and_file_param.nsl`](https://github.com/VyomKulshrestha/Nucle-OS/blob/main/docs/examples/explicit_type_args_and_file_param.nsl),
  in the main repo.
- Bumped the downloaded `nucle-lsp`/`nucle-cli` binaries to the release
  that actually understands the new syntax, same reason as every prior
  language-feature bump.

## [0.1.7]

- **NucleScript now supports closures / higher-order functions.**
  `fn(params) -> T { body }` is an anonymous closure literal — bind one
  to a variable, pass one as an argument (`Fn(...) -> T` is its type),
  and it captures real values from its enclosing scope. Highlighting,
  live diagnostics, hover, and formatting all understand `Fn(...)` and
  closure literals — see the updated README and
  [`docs/examples/closure_retry.nsl`](https://github.com/VyomKulshrestha/Nucle-OS/blob/main/docs/examples/closure_retry.nsl)
  in the main repo.
- Bumped the downloaded `nucle-lsp`/`nucle-cli` binaries to the release
  that actually understands the new syntax, same reason as every prior
  language-feature bump.

## [0.1.6]

- **NucleScript now supports pattern matching over `Result<T, E>`.**
  `match attempt { Ok(file) => file, Err(reason) => ... }` branches on a
  caught `Result` directly inside one function, instead of needing a
  second, independent function call from the caller. Highlighting, live
  diagnostics, hover, formatting, and `nucle doc` all understand the new
  `match`/`Ok`/`Err`/`=>` syntax — see the updated README and
  [`docs/examples/match_result_fallback.nsl`](https://github.com/VyomKulshrestha/Nucle-OS/blob/main/docs/examples/match_result_fallback.nsl)
  in the main repo.
- Bumped the downloaded `nucle-lsp`/`nucle-cli` binaries to the release
  that actually understands the new syntax, same reason as 0.1.4's/
  0.1.5's bumps.

## [0.1.5]

- **NucleScript now supports generics over `Pool<T>`'s profile.** `fn
  name<T>(...)` lets one function work with a `Pool<Illumina>`,
  `Pool<Nanopore>`, or `Pool<Twist>` argument instead of needing a
  hardcoded copy per profile. Highlighting, live diagnostics, hover,
  formatting, and `nucle doc` all understand the new `<T>` syntax — see
  the updated README and
  [`docs/examples/generic_pool_recovery.nsl`](https://github.com/VyomKulshrestha/Nucle-OS/blob/main/docs/examples/generic_pool_recovery.nsl)
  in the main repo.
- Bumped the downloaded `nucle-lsp`/`nucle-cli` binaries to the release
  that actually understands the new syntax, same reason as 0.1.4's
  `Result<T,E>`/`?` bump.

## [0.1.4]

- **NucleScript now supports `Result<T, E>` and `?` for real error
  handling.** `store`/`delete` can appear in expression position,
  producing a catchable `Result<T, Str>` instead of aborting the whole
  program on a VFS failure; `?` unwraps or propagates it. Highlighting,
  live diagnostics, hover, formatting, and Run File all understand the
  new syntax — see the updated README and
  [`docs/examples/result_fallback_store.nsl`](https://github.com/VyomKulshrestha/Nucle-OS/blob/main/docs/examples/result_fallback_store.nsl)
  in the main repo.
- Bumped the downloaded `nucle-lsp`/`nucle-cli` binaries to the release
  that actually understands the new syntax — a fresh install of this
  version downloads binaries that can parse `Result<T, Str>`/`?`; a
  fresh install of 0.1.3 or earlier would not.

## [0.1.3]

- Fixed the README's "Getting started" example: it told you to create
  `hello.nsl` with a `store "sample_a.txt" into archive` line, but never
  said to actually create `sample_a.txt` — `nucle-cli run` resolves
  `store`/`retrieve` paths relative to the `.nsl` file's own folder (not
  the CLI's working directory), so a fresh copy-paste failed with
  `failed to read '...\sample_a.txt': The system cannot find the file
  specified.`. The steps now say to create that file first, and the
  "you'll see real output" line explains what that specific error means.

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
