# Pact MCP Plugin

Consumer-driven **contract testing for [Model Context Protocol (MCP)](https://modelcontextprotocol.io)** servers and clients, built as a [Pact](https://pact.io) plugin.

Test that an AI agent (MCP **client**) and the MCP **server** it depends on agree on the shape of `tools/call`, `tools/list`, `resources/read|list`, `prompts/get|list`, and their results — without spinning up the whole stack. You author expectations against your **real** `@modelcontextprotocol/sdk` `Client`, and verify them against your **real** MCP server. No stubbing, no service decomposition.

> **Status:** MVP+. stdio + Streamable HTTP transports, HTTP auth (bearer / API key / custom headers / OAuth2 client credentials), provider verification **through the stock pact-js `Verifier`**, provider states, consumer mocks, multi-interaction pacts, and a TypeScript adapter — all proven end-to-end against real MCP clients/servers and the real Pact toolchain. See the [roadmap](#roadmap).

## Why

- **MCP has no contract-testing story.** Agents break when a server changes a tool's arguments or result shape; servers break their consumers without knowing.
- Core Pact can't test MCP: it has **no stdio transport** (and most MCP servers are stdio-first), no Streamable HTTP/SSE handling, and no knowledge of the MCP handshake or tool semantics. This plugin adds all of that.
- **One matching engine, every language.** Matching lives once in a Rust engine; language adapters are thin DX layers over it, so pacts are portable (author in TS, verify anywhere).

## Capabilities

| | stdio | Streamable HTTP |
|---|---|---|
| Provider verification | ✅ | ✅ (+ bearer / API key / custom headers) |
| OAuth2 client-credentials auth | — | ✅ (SEP-1046; see ADR 0011) |
| Standard pact-js `Verifier` support | ✅ | ✅ |
| Provider states (`given(...)` + `stateHandlers`) | ✅ | ✅ |
| Consumer mock | ✅ | ✅ (loopback) |
| `tools/call` / `tools/list`, `resources/read` / `resources/list`, `prompts/get` / `prompts/list` | ✅ | ✅ |
| Auto `initialize` handshake + capability negotiation | ✅ | ✅ |
| Matching in the shared Rust engine | ✅ | ✅ |
| TypeScript adapter DX | ✅ | ✅ |

Auth secrets use `${ENV}` interpolation and are **never written to the pact**.

## Quick start (TypeScript)

Install the package — its `postinstall` also downloads the engine binary (the
Pact `mcp` plugin) for your platform into `~/.pact/plugins/`:

```sh
npm install @pactflow/pact-mcp-plugin
```

Not using npm (e.g. verifying from the JVM/.NET/Go verifier)? Install the engine
binary directly:

```sh
curl -fsSL https://raw.githubusercontent.com/mefellows/pact-mcp-plugin/main/scripts/install-plugin.sh | bash
# …or from source:  ./scripts/install-local.sh
```

**Consumer** — drive your real MCP client against a Pact-synthesized mock:

```ts
import { McpPact, like } from "@pactflow/pact-mcp-plugin";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";

await new McpPact({ consumer: "weather-agent", provider: "weather-mcp" })
  .whenClientCallsTool("get_weather", { city: "Melbourne" })
  .willRespondWith({ content: [{ type: "text", text: like("Sunny, 22C") }], isError: false })
  .executeTest(async ({ transport }) => {
    const client = new Client({ name: "weather-agent", version: "1.0.0" }); // your REAL client
    await client.connect(transport);
    const res = await client.callTool({ name: "get_weather", arguments: { city: "Melbourne" } });
    expect(res.content[0].text).toContain("22C");
  });
// -> writes ./pacts/weather-agent-weather-mcp.json
```

**Provider** — verify that pact against your real server, over stdio or HTTP:

```ts
import { McpProviderVerifier } from "@pactflow/pact-mcp-plugin";

await new McpProviderVerifier({ provider: "weather-mcp", pactUrls: ["./pacts/weather-agent-weather-mcp.json"] })
  .withServerTransport({ type: "stdio", command: "node", args: ["dist/server.js"] })
  // or: { type: "http", url: "https://mcp.example.com/mcp", auth: { type: "bearer", token: "${MCP_TOKEN}" } }
  .verify();
```

Pacts also verify through the **stock pact-js `Verifier`** (broker fetch, result publishing, `can-i-deploy` — see [`docs/usage.md`](docs/usage.md) and [`docs/bdct-walkthrough.md`](docs/bdct-walkthrough.md)). Provider states, `tools/list` / `resources/*` / `prompts/*` expectations, and multi-interaction tests are covered in [`adapters/ts/pact-mcp/README.md`](adapters/ts/pact-mcp/README.md).

## Architecture

```
Shared contract spec (docs/spec/)  ── the pact schema + MCP matching semantics
        │ implemented by
Rust engine (rust/pact-mcp-plugin/) ── Pact plugin over gRPC: matching, generation,
        │                              stdio + Streamable HTTP transports, auth, mocks
        ├── TypeScript adapter (adapters/ts/pact-mcp/) ── thin DX; matching stays in the engine
        └── any Pact language (Java/.NET/Go/…) via the engine over a real transport
```

The engine is the universal backbone; each language connects its **real** MCP client/server to the engine's mock/verifier over a real transport (stdio or loopback HTTP). This is the one approach that works across every MCP SDK (all have stdio + HTTP client transports), including Java/.NET. See [`docs/plans/pact-mcp-plugin-implementation-plan.md`](docs/plans/pact-mcp-plugin-implementation-plan.md) and the [ADRs](docs/decisions/).

## Repository layout

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

## Build & test

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

## Roadmap

- OAuth2 dynamic client registration + the interactive authorization-code flow
  (client-credentials, the CI-friendly machine-to-machine grant, shipped —
  see ADR 0011; the `AuthProvider`/rmcp wiring makes this additive)
- A fixture authorization-server end-to-end (discover -> token -> verify)
  covering the full OAuth2 client-credentials round trip, beyond today's
  unit/mocked-token-endpoint tests
- Python / Go adapters + Java/.NET loopback examples (the shared spec + engine make these additive)
- Optional in-memory adapter DX for TS/Python/Go

## License

[MIT](LICENSE) © 2026 Matt Fellows
