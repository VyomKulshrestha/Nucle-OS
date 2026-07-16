# Security Policy

Nucle-OS stores real file data (encoded as simulated DNA) and includes a
few genuinely security-relevant features — encryption at rest, an
OS-user allowlist gating destructive hardware operations, and a network
metrics endpoint. This document explains what those features actually
protect against, what they don't, and how to report a vulnerability.

## Supported Versions

We actively maintain and patch the `main` branch and the latest tagged
release only.

| Version        | Supported |
| -------------- | --------- |
| `main` (latest release) | ✅ Active |
| Older releases | ❌ Not supported |

## Scope, and what each security-relevant feature actually guarantees

Read [`docs/architecture.md`](docs/architecture.md) for the full detail —
the short version, so a report can be scoped accurately:

- **Encryption at rest** (`nucle encrypt-pool`/`decrypt-pool`) — a random
  data-encryption key (ChaCha20-Poly1305) protects `pool_dir/state.json`;
  that key is itself wrapped under a passphrase-derived key (Argon2id).
  This protects data *at rest* (a stolen disk, a copied `pool_dir`). It
  does **not** protect `audit.log`/`config.json` (filenames/timestamps,
  not file content — a documented, deliberate scope limit), and there is
  **no passphrase-recovery mechanism by design** — a lost passphrase
  means permanently lost data, not a support ticket.
- **The `--confirm` OS-user allowlist** (`nucle confirm-users`) — this is
  explicitly **not** real access control. It's an environment-variable
  read (`$USER`/`%USERNAME%`), trivially spoofable by anyone with local
  shell access. It catches accidents (a script running as the wrong
  user), not a determined local attacker.
- **Per-tenant pool isolation** (`--tenant`) — genuine isolation via
  separate storage (each tenant is a wholly independent pool directory),
  but tenant identity is just a name you pass — there's no credential
  behind it. Anyone who can run the CLI can pass any `--tenant` value.
- **`nucle serve`** — a metrics-only HTTP exporter (no store/retrieve
  surface). Binds to `127.0.0.1` by default; `--bind 0.0.0.0` is an
  explicit opt-in to expose it beyond localhost. It has no
  authentication of any kind — treat `--bind 0.0.0.0` as "anyone who can
  reach this port can read this pool's file counts, redundancy, and
  audit-event tallies," and firewall accordingly.

**Out of scope for this policy:**
- Vulnerabilities in upstream Rust crate dependencies — please report
  those to the crate's own maintainers (or via [RustSec](https://rustsec.org/))
- Social engineering
- The VS Code extension's automatic binary download mechanism working as
  designed (downloading a prebuilt `nucle-cli`/`nucle-lsp` for your
  platform) is expected behavior, not a vulnerability by itself

## Reporting a Vulnerability

**Please do not open a public GitHub issue for a security vulnerability.**
A public issue exposes the problem before a fix exists.

Instead, use one of:

1. **GitHub Private Security Advisory** *(preferred)* — via this
   repository's [Security tab](https://github.com/VyomKulshrestha/Nucle-OS/security/advisories/new).
2. **Email the maintainer** — [@VyomKulshrestha](https://github.com/VyomKulshrestha)
   via the contact email on their GitHub profile.

### What to include

- A clear description of the vulnerability and where it lives (crate,
  file, function)
- Steps to reproduce
- The potential impact (e.g., data exposure, a way to bypass the
  confirm-allowlist or tenant isolation, a crash in the metrics
  exporter)
- Any suggested fix or mitigation (optional, appreciated)

## Response Timeline

| Timeframe | Action |
|-----------|--------|
| Within 48 hours | Acknowledgement of your report |
| Within 7 days | Severity assessment and confirmation |
| Within 30 days | A fix developed and tested |
| Within 45 days | A patch released |
| After the patch | Public disclosure, coordinated with you |

We ask that you give us a reasonable window to fix the issue before
disclosing publicly, and avoid accessing or modifying data beyond what's
needed to demonstrate the issue.

## Severity Levels

| Level | Description |
|-------|--------------|
| 🔴 Critical | Remote code execution, decrypting an encrypted pool without the passphrase |
| 🟠 High | Bypassing tenant isolation or the confirm-allowlist in a way that's more than the documented spoofability, unauthenticated access via `nucle serve` beyond reading metrics |
| 🟡 Medium | Information disclosure beyond what's already documented as exposed |
| 🟢 Low | Minor issues with limited real-world impact |

## Contact

- **Maintainer**: [@VyomKulshrestha](https://github.com/VyomKulshrestha)
- **Private Advisory**: [Submit here](https://github.com/VyomKulshrestha/Nucle-OS/security/advisories/new)

## Acknowledgements

Security researchers who responsibly disclose a real vulnerability will
be credited in release notes, unless you'd prefer to stay anonymous.
