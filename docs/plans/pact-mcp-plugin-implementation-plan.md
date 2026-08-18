# Pact MCP Support — Implementation Plan

> **Audience:** implementing engineer/agent (Sonnet 5).
> **Status:** design approved; ready for implementation.
> **Architecture:** one shared contract spec → a language-neutral **Rust engine** (matching + wire transports) → thin **native in-memory adapters** (TS / Python / Go) + **loopback-HTTP fallback** (Java / .NET).
> **MVP order:** stdio wire path first, then Streamable HTTP + auth; native in-memory DX layered on top.

This document is the source of truth. Follow it top to bottom. Where it says **VERIFY UPSTREAM**, do not invent an API — fetch the current upstream docs/source and match reality. Pin exact crate/proto/SDK versions and record them in `docs/decisions/`.

---

## 0. Context & strategic framing (read once, don't skip)

We are adding **Model Context Protocol (MCP)** support to Pact, the way `pact-graphql-plugin` added GraphQL. MCP is JSON-RPC 2.0 over one of two transports:

- **stdio** — client launches the server as a subprocess; newline-delimited JSON-RPC over stdin/stdout (no embedded newlines; stderr = logs only).
- **Streamable HTTP** — client POSTs JSON-RPC to one MCP endpoint; server replies with a JSON body or an SSE stream; sessions tracked via `Mcp-Session-Id`. A lifecycle handshake (`initialize` → `initialized`, with capability + protocol-version negotiation) precedes all operations.

Primitives (provider = MCP **server**; consumer = MCP **client**/AI agent): `tools/list`, `tools/call` (build first), then `resources/list|read`, `prompts/list|get`.

**This work is consumer-driven, example-based contract testing.** It captures the specific calls an agent makes and the responses it expects. It is **not** schema-conformance — that is the Specmatic-parity play and belongs in **Drift** (see §13). PactFlow **BDCT** ties the two together (does the agent's usage fit the server's declared schema?) — that triangle is the competitive moat.

### 0.1 The decisive design insight (why the architecture below)

The elegant "no wire transport — connect the user's **real** `Client` to their **real** `Server` in-memory" experience depends on each language's MCP SDK shipping an **in-memory linked transport**. That is **not uniform**:

| Language | In-memory linked transport | Mechanism |
|---|---|---|
| TypeScript | ✅ first-class | `InMemoryTransport.createLinkedPair()` |
| Python | ✅ first-class | `mcp.shared.memory.create_client_server_memory_streams()` |
| Go | ✅ first-class | go-sdk `NewInMemoryTransports()` |
| Java | ⚠️ none built-in | pluggable transports; stdio + Streamable HTTP only |
| .NET | ⚠️ none built-in | pluggable transports; stdio + HTTP only |

So the in-memory path is a **DX enhancement for three languages, not a substitute for a shared engine** — and it is missing in exactly the enterprise languages (Java/.NET) SmartBear most cares about. Therefore:

- **Rust engine = the universal backbone** (matching + stdio/HTTP wire transports). Makes Java/.NET/Ruby/etc. work without per-SDK heroics, and enables over-the-wire verification of deployed servers everywhere.
- **Native in-memory adapters (TS/Python/Go)** = thin optional sugar giving the zero-wire DX on top of the same engine/spec.
- **Loopback HTTP** = the SDK-only fallback for Java/.NET (start the user's real server on `127.0.0.1:0`, connect the real client — real objects, no stubbing, just an ephemeral local socket).

---

## 1. Architecture overview

```
                    ┌─────────────────────────────────────────────┐
                    │   SHARED CONTRACT SPEC  (docs/spec/)         │
                    │   • pact interaction schema (application/    │
                    │     mcp+json)  • MCP matching semantics       │
                    │   Every implementation MUST honor this.       │
                    └───────────────────────┬─────────────────────┘
                                            │ implemented by
                    ┌───────────────────────▼─────────────────────┐
                    │   RUST ENGINE  (rust/pact-mcp-plugin)         │
                    │   Pact plugin over gRPC:                      │
                    │   • ConfigureInteraction / CompareContents /  │
                    │     GenerateContent   (matching+generation)   │
                    │   • StartMockServer / Verify  (wire path)     │
                    │   • L1 transports: stdio, Streamable HTTP+SSE │
                    │   • auth: bearer / apiKey / headers           │
                    └───────┬───────────────────────────┬─────────┘
                            │ drives (gRPC)              │ drives (gRPC)
        ┌───────────────────▼──────────┐     ┌──────────▼───────────────────┐
        │  NATIVE IN-MEMORY ADAPTERS   │     │  ANY PACT LANGUAGE via engine │
        │  (TS / Python / Go)          │     │  (Java, .NET, Ruby, …)        │
        │  real Client ↔ real Server   │     │  wire transport (stdio/HTTP)  │
        │  via SDK in-memory transport │     │  or loopback-HTTP fallback    │
        │  → best DX, zero wire        │     │  → real Client ↔ real Server  │
        └──────────────────────────────┘     └───────────────────────────────┘
```

Key properties:
- **One matching implementation** (Rust). Adapters never re-implement matching; they marshal payloads to the engine and render results. This prevents divergence.
- **In-memory vs wire is orthogonal to matching.** Adapters choose how the user's Client/Server connect (in-memory link, loopback HTTP, real stdio/HTTP); the engine only cares about the JSON-RPC request/response pair to match.
- **Pacts are portable.** A TS-authored pact verifies via the engine in Java, and vice versa, because both honor the shared spec.

---

## 2. Reference implementations (study before coding)

**VERIFY UPSTREAM** — read and mirror:
1. **`pact-protobuf-plugin`** (pactflow org — `pactflow/pact-protobuf-plugin`, NOT `pact-foundation`, Rust) — canonical Rust plugin implementing transport + content matcher + mock server + verifier. Closest structural template (proto wiring, tonic bootstrap, startup handshake, catalogue).
2. **`pact-plugins`** (pact-foundation) — `docs/` + `plugin.proto` (`io.pact.plugin.PactPlugin`). Defines every gRPC method.
3. **`pact-graphql-plugin`** (mefellows, Rust) — pattern for a thin native helper (`js/pact-*-helper`) + packaging/DX.
4. **`rmcp`** (official Rust MCP SDK) — transports (stdio child-process, Streamable HTTP w/ custom headers) + typed protocol models. **VERIFY** the surface; fall back to raw `serde_json` JSON-RPC over the transport if gaps exist.
5. **MCP SDKs for adapters** — TS `@modelcontextprotocol/sdk` (`InMemoryTransport.createLinkedPair`), Python `mcp` (`mcp.shared.memory`), Go go-sdk (`NewInMemoryTransports`). **VERIFY** exact current APIs.
6. **MCP spec** — lifecycle, transports, tools/resources/prompts. Pin supported protocol version(s) (e.g. `2025-06-18`) and negotiate.

---

## 3. Repository layout

```
pact-mcp-plugin/
├── docs/
│   ├── plans/ (this file)
│   ├── spec/                       # THE SHARED SPEC (§4) — pact schema + matching semantics
│   │   ├── interaction-schema.md
│   │   ├── matching-semantics.md
│   │   └── conformance/            # golden pact fixtures + expected match results
│   ├── decisions/                  # ADRs: versions, model mapping, transport choices
│   └── usage.md
├── rust/
│   └── pact-mcp-plugin/            # the engine (single binary)
│       ├── Cargo.toml
│       ├── build.rs                # tonic-build compiles plugin.proto
│       ├── proto/plugin.proto      # vendored from pact-plugins (pin version)
│       └── src/
│           ├── main.rs             # bootstrap: startup JSON line, tonic serve
│           ├── server.rs           # PactPlugin gRPC dispatch
│           ├── catalogue.rs        # InitPlugin catalogue entries
│           ├── content/            # matching/generation (configure/matcher/generator)
│           ├── mcp/                # model, jsonrpc, handshake, tools, resources, prompts
│           ├── transport/          # mod (trait), stdio, http (+SSE, session id)
│           ├── auth.rs             # AuthProvider trait: bearer/apiKey/headers (+ OAuth2 later)
│           ├── mock.rs             # StartMockServer / mock MCP server
│           └── verify.rs           # Prepare/VerifyInteraction
├── adapters/
│   └── ts/pact-mcp/                # native in-memory adapter (npm) — TS FIRST; python/go later
├── examples/
│   ├── fixtures/                   # tiny real MCP servers (stdio + http) for tests
│   ├── ts-inmemory/  provider-stdio/  http-consumer/
│   └── (later) python-inmemory/  go-inmemory/  java-loopback/
├── pact-plugin.json                # plugin manifest
└── scripts/                        # install.sh, release packaging
```

---

## 4. THE SHARED SPEC (build this first; everything conforms to it)

Located in `docs/spec/`. This is language-neutral and is the contract between the engine and all adapters. Get it right before writing engine internals.

### 4.1 Interaction shape — MCP operation as a Pact V4 *synchronous message*
Model each MCP operation as a request/response sync message (transport-agnostic; the transport only matters at verification time). Persisted under plugin-specific config:
```jsonc
{
  "pact:content-type": "application/mcp+json",
  "mcp": {
    "operation": "tools/call",         // tools/list | tools/call | resources/read | prompts/get
    "request":  { "name": "get_weather", "arguments": { "city": "Melbourne" } },
    "response": { "content": [ { "type": "text", "text": "..." } ],
                  "isError": false, "structuredContent": { /* optional */ } },
    "server":   { "transport": "stdio" } // optional provider-verification hint: stdio | http
  }
}
```
- JSON-RPC envelope (`id`, `jsonrpc`, `method` from `operation`) is **synthesized by the engine**, never authored.
- Handshake (`initialize`/negotiation/`initialized`) is connection-level, done once by mock/verifier — **not** a per-interaction artifact.
- Matching rules + generators use Pact's standard maps, keyed by JSON path into `request`/`response`, produced by `ConfigureInteraction`.

### 4.2 MCP matching semantics (default rules; §5 expands)
- `tools/call` result `content[]`: type-aware (text → `type`+`text`; image/resource → `type`+declared fields), consumer-configurable exact vs type-only.
- `isError`: exact by default. JSON-RPC `error`: `code` exact, `message` by type, `data` by structure.
- `structuredContent`: full JSON matching with the consumer's matchers.
- `tools/list`: subset/each-like — assert only the tools (by `name` + `inputSchema` shape) the consumer uses; the server may expose more (consumer-driven).

### 4.3 Conformance fixtures
`docs/spec/conformance/` holds golden `{interaction, actual, expectedResult}` triples. **Both the engine and every adapter run these as tests.** This is the anti-divergence mechanism — treat a conformance failure as a release blocker.

---

## 5. Matching rules (engine detail)

Reuse Pact's JSON matching engine for payload bodies; add the MCP-aware defaults from §4.2. `CompareContents` receives expected + actual MCP payloads, decodes both, applies rules, returns typed mismatches with JSON-path context and human-readable messages. All behavior must be expressible as spec conformance fixtures (§4.3).

---

## 6. Pact plugin mechanics (engine) — get these right

**VERIFY UPSTREAM against current `plugin.proto`.**

- **Manifest/install:** `pact-plugin.json` (`name: mcp`, `version`, `executableType: exec`, per-platform `entryPoint`). Installs to `~/.pact/plugins/mcp-<version>/`.
- **Startup handshake:** bind ephemeral gRPC port; print **one** stdout line `{"port":<n>,"serverKey":"<rand>"}`, then serve. Everything else → stderr. Validate `serverKey` gRPC metadata on every call.
- **Catalogue (`InitPlugin`):** content-matcher + content-generator for `application/mcp+json`; transports `mcp-stdio` and (later) `mcp-http`; mock-server capability.
- **Methods:** `InitPlugin`, `UpdateCatalogue`, `ConfigureInteraction`, `CompareContents`, `GenerateContent`, `StartMockServer`, `ShutdownMockServer`, `GetMockServerResults`, `PrepareInteractionForVerification`, `VerifyInteraction`. (Behaviors per §4/§7/§8.)

---

## 7. Consumer side — how the user's real Client connects

The mock MCP peer is **synthesized by Pact** from the interactions; the user writes no mock. How the real Client reaches it depends on the language tier:

### 7.1 Native in-memory adapters (TS / Python / Go)
- Adapter builds a mock MCP `Server` whose handlers consult the engine (via gRPC `CompareContents`/`GenerateContent`) to validate the incoming request and return the configured response (or record a mismatch).
- Adapter links it to the user's real Client using the SDK's in-memory pair (`createLinkedPair` / `create_client_server_memory_streams` / `NewInMemoryTransports`) and yields the client handle to the user.
- On teardown: assert all interactions occurred + no mismatches → write pact.

### 7.2 Wire / loopback (Java / .NET and any engine-driven language)
- **Engine mock over stdio:** the engine runs in mock mode as a spawnable command (`pact-mcp-plugin mock --pact <session>`) that the user's Client launches as its stdio server; it answers initialize/tools/list and matches tools/call. (Resolves the "stdio mock has no port" problem — the helper returns `{command,args,env}`.)
- **Engine mock over HTTP:** `StartMockServer` returns a URL the user's Client connects to.
- **Loopback-HTTP fallback (SDK-only, no engine mock needed):** for provider tests especially, start the user's real server on `127.0.0.1:0` and connect the real client — real objects, ephemeral socket, no decomposition.

---

## 8. Provider verification (engine)

`VerifyInteraction` per pact: build transport (stdio spawn / HTTP connect+auth / in-memory or loopback via adapter) → `initialize` + negotiate + assert required capability → apply **provider state** (map to server setup: arg/env for stdio, or a state endpoint/handler) → for each interaction send the request, `CompareContents` the response, collect mismatches → tear down. Must run from the standard Pact verifier (CLI / pact-js / JVM / .NET) once the plugin is installed — no bespoke runner. Adapters may also drive verification in-process (in-memory/loopback) while delegating matching to the engine.

---

## 9. Transports & auth (engine, L1)

Trait `McpTransport { connect, initialize, request(id-correlated), notify, close }`.
- **stdio** (Phase 1): spawn `command/args/env/cwd`; newline JSON-RPC; id correlation; child stderr → logs; timeouts → verification errors. Prefer `rmcp` child-process transport; else `tokio::process`.
- **Streamable HTTP** (Phase 2): POST JSON-RPC; handle JSON + `text/event-stream` responses; `Mcp-Session-Id`; `Accept: application/json, text/event-stream`. Deprecated HTTP+SSE transport = non-goal.
- **Auth** (Phase 2, HTTP): `AuthProvider` trait — `bearer` / `apiKey{header,value}` / `headers{}`; `${ENV}` interpolation; injected on all requests incl. initialize; **never persist resolved secrets to the pact**. stdio auth via env/args. Leave the trait seam for OAuth2 (Phase 4).

---

## 10. Native adapter DX (target APIs)

**Initial scope: TypeScript only.** Python and Java snippets below are illustrative of how the same pattern generalizes (deferred — §15); do not build them yet.

Thin layer: assemble config → drive the engine → connect the user's real Client/Server via the SDK's in-memory transport. **No matching logic in adapters.** Run the §4.3 conformance fixtures.

**TypeScript**
```ts
await new McpPact({ consumer: "weather-agent", provider: "weather-mcp" })
  .whenClientCallsTool("get_weather", { city: "Melbourne" })
  .willRespondWith({ content: [{ type: "text", text: like("Sunny, 22°C") }], isError: false })
  .executeTest(async (transport) => {
    const client = new Client({ name: "weather-agent", version: "1.0.0" });
    await client.connect(transport);                       // in-memory link to Pact mock
    const res = await client.callTool({ name: "get_weather", arguments: { city: "Melbourne" } });
    expect(res.content[0].text).toContain("°C");
  });
```

**Python** — same shape via `mcp.shared.memory`; `interaction.test()` yields client streams for `ClientSession(*client_streams)`.

**Provider (any language)** — hand over the real server factory; adapter/engine replays:
```ts
await new McpProviderVerifier({ provider: "weather-mcp", pactUrls: ["./pacts/..json"] })
  .withServer(() => createWeatherServer())                 // your real McpServer, unchanged
  .stateHandlers({ "melbourne weather is available": async () => seedDb("Melbourne") })
  .verify();
// or over the wire:  .withServerTransport({ type: "stdio", command: "node", args: ["dist/server.js"] })
```

**Java/.NET** — loopback-HTTP form (§7.2): `McpStreamableHttp.serve(realServer, "127.0.0.1:0")` then verify against `http.url()`.

---

## 11. Phased delivery & task breakdown (TDD)

Write the failing test first; keep tasks bite-sized; commit per task; don't advance a phase until its example runs end-to-end. **Do not move to adapters until the engine passes the shared conformance fixtures.**

### Phase 0 — Spec + scaffolding ✅
- [x] 0.1 Write `docs/spec/` (interaction schema §4.1, matching semantics §4.2) + first conformance fixtures §4.3.
- [x] 0.2 Cargo workspace + crate; `tonic`/`tonic-build`; vendor `plugin.proto`; `build.rs`.
- [x] 0.3 Bootstrap: ephemeral port, startup JSON line, empty `PactPlugin`; test a driver-style `InitPlugin` call.
- [x] 0.4 `rmcp` smoke test: spawn a trivial stdio MCP server, do `initialize`. **VERIFY UPSTREAM.** (ADR 0003)
- [x] 0.5 ADRs: pinned versions, MCP→Pact model, architecture (this doc). (ADRs 0001–0005)

### Phase 1 — Engine stdio vertical slice (tools/call) ✅
- [x] 1.1 `mcp/model.rs` serde types (§4.1) + round-trip tests.
- [x] 1.2 `jsonrpc.rs` envelope synthesis + id correlation.
- [x] 1.3 `ConfigureInteraction` for `tools/call` → contents + matching rules + generators (assert persisted fragment). (ADR 0004: two-part sync message + inline DSL)
- [x] 1.4 `CompareContents` MCP-aware matching (content blocks, `isError`, JSON-RPC errors) — **passes §4.3 conformance fixtures**.
- [x] 1.5 `transport/stdio.rs` against a fixture stdio server (`examples/fixtures/`).
- [x] 1.6 `handshake.rs` initialize + negotiation.
- [x] 1.7 `verify.rs` Prepare/Verify over stdio (pass + fail cases, real fixture server).
- [x] 1.8 `mock.rs` stdio mock mode (`pact-mcp-plugin mock ...`) — MCP SDK client spawns it and calls a tool.
- [x] 1.9 `examples/provider-stdio` runs green end-to-end. **Phase 1 demo.**

### Phase 2 — Engine Streamable HTTP + auth ✅
- [x] 2.1 `transport/http.rs` (JSON + SSE, `Mcp-Session-Id`) vs fixture HTTP server. (ADR 0007: rmcp Streamable HTTP client)
- [x] 2.2 `auth.rs` `AuthProvider` (bearer/apiKey/headers, `${ENV}`) injected everywhere incl. initialize; test "secrets never persisted".
- [x] 2.3 `mcp-http` transport + HTTP mock via `StartMockServer` (URL). `examples/consumer-http-mock` green.
- [x] 2.4 Docs: auth + HTTP usage. (`docs/usage.md`)

### Phase 3 — Native in-memory adapter (TypeScript only) ✅ (via engine stdio/HTTP mock, not in-memory — ADR 0006)
- [x] 3.1 **TS adapter** (`adapters/ts/pact-mcp`): consumer + provider DX (§10), delegating matching to the engine; runs §4.3 conformance fixtures; `examples/consumer-stdio-mock` + `examples/ts-roundtrip` green. **Note:** pact-js exposes no live sync-message mock transport, so the consumer flow uses the engine's stdio/HTTP mock instead of `InMemoryTransport.createLinkedPair()` (see ADR 0006); the DSL currently authors `tools/call` only. Remaining surface gaps tracked in `gap-analysis-next-phase.md` (G4).

> Python/Go adapters and the Java loopback example are **deferred** (§15). The shared spec + engine make them additive later, not a refactor.

### Phase 3.5 — Hardening
- [ ] resources/* + prompts/* (model + matching + verify + mock + fixtures).
- [ ] Provider-state contract finalized + documented (§8).
- [ ] Cross-platform release packaging (§12); CI matrix (Rust + each adapter + conformance).
- [ ] PactFlow publish + `can-i-deploy` walkthrough (BDCT positioning).

### Phase 4 — OAuth2

- [x] OAuth2 **client-credentials** (SEP-1046) `AuthProvider`: discovery,
  token acquire/refresh, injection, reusing rmcp 2.2.0's built-in OAuth2
  implementation (the `auth` cargo feature) rather than hand-rolling it.
  Slotted into the §9 `ResolvedAuth` seam with no core refactor beyond adding
  an `oauth` carrying field. See
  `docs/plans/oauth-client-credentials-plan.md` and ADR 0011 for the full
  design and the T0–T7 task breakdown. Tests are unit/mocked (a stubbed
  `/token` endpoint on the existing HTTP fixture server) — engine
  (`cargo test`) + TS DSL (`npm test`) both green.
- [ ] **Deferred:** dynamic client registration (RFC 7591) and the
  interactive authorization-code flow — rmcp's `register_client` hardcodes
  the interactive grant, which cannot run unattended in CI (see ADR 0011
  §"Why dynamic client registration is off the CI path"). The `AuthProvider`/
  rmcp wiring added for client-credentials makes this purely additive.
- [ ] **Deferred:** a fixture authorization server (`/.well-known/*`,
  `/token`, optional `/register`) + protected MCP fixture, for a full
  discover → token → verify end-to-end test beyond today's unit/mocked
  coverage.
- [ ] **Deferred:** `private_key_jwt` client-credentials
  (`auth-client-credentials-jwt` rmcp feature) and cross-spawn token caching
  (rmcp's `AuthClient` already refreshes within a session).

---

## 12. Distribution & release

- Single static Rust binary per platform (linux x86_64/arm64, macOS x86_64/arm64, windows x86_64). Release archive = binary + `pact-plugin.json`; installs under `~/.pact/plugins/mcp-<version>/`. Support `pact-plugin-cli install <source>` (**VERIFY UPSTREAM**) + manual `install.sh`.
- Adapters published to npm / PyPI / Go module, each pinning the engine version they were tested against and declaring a peer dep on the language's Pact library.
- CI attaches signed/checksummed artifacts; release notes record tested Pact / SDK / `rmcp` / `plugin.proto` versions.

---

## 13. Companion — Drift-MCP (strategy only; separate track)

Record, don't build here. Drift ingests an MCP server's `tools/list` self-description (or an authored tool schema) as the "spec" and verifies the running server conforms — argument/result-shape conformance + **schema-lie detection**. This is the direct Specmatic-parity capability and is spec-first (opposite paradigm to this consumer-driven work). **The moat:** publish the consumer contract (this work) and the provider contract (Drift-MCP) to PactFlow; cross-contract verification answers *"can this agent safely deploy against this MCP server?"* — which Specmatic can't cleanly match.

---

## 14. Testing strategy

- **Unit:** model serde, JSON-RPC envelopes, matching rules, auth injection/interpolation.
- **Conformance (§4.3):** engine + the TS adapter run the golden fixtures — the anti-divergence gate; failure blocks release. (Future Python/Go adapters must run them too.)
- **Component:** transports vs fixture MCP servers (`examples/fixtures/`).
- **Contract/integration:** full consumer→pact→provider round trip per transport and per adapter (the phase examples double as CI integration tests).
- **Negative:** capability-not-supported, tool mismatch, error responses, auth missing/invalid, process crash/timeout, secret-never-persisted.

---

## 15. Non-goals (MVP)

OAuth2 dynamic client registration + the interactive authorization-code flow (client-credentials shipped in Phase 4 — see ADR 0011), deprecated HTTP+SSE transport, MCP `sampling`/`roots`/`elicitation`, schema-conformance-from-`tools/list` (that's Drift-MCP, §13), notifications beyond `initialized`, multi-server orchestration, first-class in-memory transports for Java/.NET (use loopback fallback; contributing them upstream is optional future work). **Python/Go native adapters and the Java loopback example are deferred** — the TS adapter ships first; the shared spec + engine make the rest additive.

---

## 16. Open items to confirm in Phase 0 (VERIFY UPSTREAM — don't guess)

1. Current `plugin.proto` message shapes + the pact-js / pact-jvm / pact-net V4 plugin builder API for synchronous-message plugin interactions.
2. `rmcp` capability surface (stdio child-process w/ env; Streamable HTTP w/ custom headers; typed handshake/tools models). Fall back to raw `serde_json` over transport if gaps exist.
3. Exact in-memory APIs: TS `InMemoryTransport.createLinkedPair`, Python `mcp.shared.memory.*`, Go `NewInMemoryTransports` (names/signatures on the pinned SDK versions).
4. Whether the mock-server catalogue entry can express a spawnable stdio-command mock, or the adapter must own the spawn (§7.2).
5. Provider-state delivery supported by the standard verifier for plugin transports (§8).
```
