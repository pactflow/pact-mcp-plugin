## Developer Documentation

### Architecture

```
Shared contract spec (docs/spec/)  ── the pact schema + MCP matching semantics
        │ implemented by
Rust engine (rust/pact-mcp-plugin/) ── Pact plugin over gRPC: matching, generation,
        │                              stdio + Streamable HTTP transports, auth, mocks
        ├── TypeScript adapter (adapters/ts/pact-mcp/) ── thin DX; matching stays in the engine
        └── any Pact language (Java/.NET/Go/…) via the engine over a real transport
```

The engine is the universal backbone; each language connects its **real** MCP client/server to the engine's mock/verifier over a real transport (stdio or loopback HTTP). This is the one approach that works across every MCP SDK (all have stdio + HTTP client transports), including Java/.NET. See [`docs/plans/pact-mcp-plugin-implementation-plan.md`](docs/plans/pact-mcp-plugin-implementation-plan.md) and the [ADRs](docs/decisions/).

### Repository layout

```
docs/
  spec/           # shared contract spec + conformance fixtures (the anti-divergence gate)
  plans/          # implementation plan (source of truth)
  decisions/      # ADRs
  usage.md        # HTTP + auth usage
rust/
  pact-mcp-plugin/ # the engine (single binary Pact plugin)
adapters/
  ts/pact-mcp/    # TypeScript adapter (@pactflow/pact-mcp-plugin)
examples/         # runnable consumer/provider examples + fixture MCP servers
pact-plugin.json  # Pact plugin manifest (name: mcp)
```

### Build & test

```sh
# Rust engine
cd rust && cargo test -p pact-mcp-plugin

# TypeScript adapter (drives the engine)
cd adapters/ts/pact-mcp && npm install && npm test
```

## Releasing

CI (`.github/workflows/ci.yml`) runs on every push/PR: the Rust engine (Linux +
macOS build/test/clippy, Windows build smoke, conformance as a named gate) and
the full TypeScript E2E suite (stock pact-js `Verifier`, provider states,
resources/prompts, multi-interaction) plus a publishable `npm pack` check.

Releases are **conventional-commit driven** via
[release-please](https://github.com/googleapis/release-please)
(`.github/workflows/release-please.yml`), so use Conventional Commits on `main`
(`feat:` → minor, `fix:` → patch, `feat!:`/`BREAKING CHANGE:` → major).

1. On each push to `main`, release-please maintains a **release PR** that bumps
   the version — `pact-plugin.json`, `adapters/ts/pact-mcp/package.json`, and
   `rust/Cargo.toml` are kept in lockstep — and updates `CHANGELOG.md`.
2. **Merge the release PR** to cut the release. In the same workflow run:
   - **build-release** builds the engine for linux/osx (x86_64 + aarch64) and windows and attaches per-platform `*.gz` + `.sha256`, a version-stamped `pact-plugin.json`, and `install-plugin.sh` to the GitHub Release (naming follows the pact-plugin ecosystem convention).
   - **publish-npm** publishes `@pactflow/pact-mcp-plugin`; end users' `postinstall` then provisions the matching engine binary.

Publishing uses npm **trusted publishing** (OIDC) — no `NPM_TOKEN` secret.
Configure a trusted publisher for the package on npmjs.com pointing at this repo
and the **`release-please.yml`** workflow (that's where `npm publish` runs); the
job requests an `id-token` and publishes with automatic provenance.

> **First release:** the package name must exist on npm before trusted
> publishing works. Publish a one-time placeholder (`@pactflow/pact-mcp-plugin@0.0.1`),
> configure the trusted publisher, then let release-please cut `0.1.0`.