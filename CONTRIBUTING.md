# Contributing to Nucle-OS

Thanks for your interest in contributing! This guide covers the workspace
layout, how to get a dev environment running, and what to check before
opening a pull request.

## Architecture overview

Nucle-OS is a Rust workspace. Each crate owns one layer of the storage
stack:

```
nucle_codec/      — Encoding/decoding engine (binary ↔ ATCG), ternary/fountain/yin-yang codecs
nucle_synth/      — Synthesis/sequencing noise simulator (hardware mock)
nucle_ecc/        — Error correction (Reed-Solomon, fountain, consensus voting)
nucle_index/      — Retrieval & indexing (primer addressing, CRISPR-sim, vector similarity search)
nucle_vfs/        — Virtual file system: syscall-style API, persistence, audit log,
                    encryption at rest, capacity limits, integrity scanning, metrics
nucle_agent/      — Natural-language agent interface (ReAct-style planner/executor)
nucle_lang/       — NucleScript: lexer/parser/typechecker/MIR optimizer/formatter/
                    doc generator/package registry/simulation backend
nucle_hardware/   — Hardware provider adapters (Provider trait, mock/file-export providers)
nucle_lsp/        — NucleScript language server (tower-lsp over nucle_lang::analyze)
nucle_cli/        — The `nucle` command-line interface
nucle_playground/ — Interactive web playground (tiny_http server + static frontend)
nucle_demo_core/  — Shared benchmark/pipeline demo engine used by the playground
nucle_wasm/       — WebAssembly build of the playground for the in-browser demo
editors/vscode/   — The NucleScript VS Code extension (own CONTRIBUTING.md — see below)
```

See [`docs/architecture.md`](docs/architecture.md) for how these layers fit
together, and the root [`README.md`](README.md) for the full feature tour,
CLI usage, and test coverage per crate.

## Dev environment setup

### Prerequisites

- **Rust** (stable toolchain — `rustup default stable`)
- **Git**
- Node.js is only needed if you're working on the VS Code extension
  specifically (see its own [CONTRIBUTING.md](editors/vscode/nuclescript/CONTRIBUTING.md))

### 1. Clone the repo

```bash
git clone https://github.com/VyomKulshrestha/Nucle-OS.git
cd Nucle-OS
```

### 2. Build the workspace

```bash
cargo build --workspace
```

### 3. Run the full test suite

```bash
cargo test --workspace
```

The workspace has 600+ tests across every crate — see the **Test Coverage**
table in the README for what each crate actually exercises. Integration
tests that spawn the real `nucle-cli` binary (e.g. `nucle_cli/tests/*.rs`)
take a little longer since they build and run the binary as a genuine
subprocess; this is intentional — this project prefers proving behavior
against the real compiled binary over in-memory shortcuts wherever that's
practical.

### 4. Try the CLI directly

```bash
cargo run -p nucle_cli -- --help
cargo run -p nucle_cli -- store some_file.txt --redundancy 2
cargo run -p nucle_cli -- retrieve some_file.txt
```

### 5. Try the playground

```bash
cargo run -p nucle_playground
# open http://127.0.0.1:8080
```

## Code style

- **Formatting**: `cargo fmt` for anything you touch. (Note: large parts of
  the existing codebase predate consistent `rustfmt` usage, so don't be
  surprised if `cargo fmt --check` reports unrelated drift elsewhere —
  just keep your own diff clean.)
- **Linting**: `cargo clippy` is good practice locally, though it isn't a
  required CI gate today.
- **Comments**: this codebase leans toward *no* comments unless the *why*
  is genuinely non-obvious (a hidden constraint, a subtle invariant, a
  workaround for a specific bug). Well-named identifiers should carry the
  *what*; comments are for the *why*.
- **Scope discipline**: a bug fix doesn't need surrounding cleanup; a
  focused change doesn't need new abstractions "for later." This project
  has a strong bias toward the narrowest change that honestly solves the
  problem in front of it.

## Submitting a pull request

1. **Fork** the repository and create a feature branch.
2. **Make your change**, and run `cargo test --workspace` before opening
   the PR — a passing full-suite run is the bar, not just the tests for
   the crate you touched (cross-crate regressions are real in a workspace
   this integrated).
3. **Add tests** for new behavior. Prefer testing against the real
   `NucleOS`/`nucle-cli` surface over mocking internals where practical —
   see any existing `nucle_vfs/src/*.rs` test module or `nucle_cli/tests/*.rs`
   for the house style.
4. **Update docs alongside code, not after** — `README.md`'s relevant
   section (including the Test Coverage table, if you added/removed
   tests) and `docs/*.md` if you touched something they describe. A PR
   that changes behavior without updating the docs describing that
   behavior will likely get asked to do so before merge.
5. **Open the PR** against `main` with a clear description of what
   changed and why.

## Reporting bugs / requesting features

Please use the issue templates:
- [Bug report](.github/ISSUE_TEMPLATE/bug_report.md)
- [Feature request](.github/ISSUE_TEMPLATE/feature_request.md)

## Contributing to the VS Code extension

The NucleScript VS Code extension lives under
[`editors/vscode/nuclescript/`](editors/vscode/nuclescript/) and has its
own [CONTRIBUTING.md](editors/vscode/nuclescript/CONTRIBUTING.md) covering
grammar testing and the extension's own release process.

## Security issues

Please **do not** open a public issue for a security vulnerability — see
[SECURITY.md](SECURITY.md) for how to report one responsibly.

## License

By contributing, you agree that your contributions will be licensed under
this repository's [MIT License](LICENSE).
