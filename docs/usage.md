# Usage — HTTP transport & auth

The engine supports two MCP transports for both consumer mocks and provider
verification: **stdio** and **Streamable HTTP**. Matching always runs in the Rust
engine.

## Secrets: `${ENV}`, never in the pact

Auth material lives ONLY on the verification/transport config — it is **never**
written into the pact file. Reference secrets with `${ENV}` interpolation,
resolved from the process environment at verification time:

```jsonc
{ "type": "bearer",  "token": "${MCP_TOKEN}" }
{ "type": "apiKey",  "header": "X-API-Key", "value": "${MCP_API_KEY}" }
{ "type": "headers", "headers": { "X-Tenant": "acme", "X-Sig": "${SIG}" } }
```

A missing env var is a hard error (never a silent empty header). The invariant
"secrets never land in the persisted pact" is enforced by construction
(`config.rs` has no auth input) and covered by a test
(`auth.rs::secrets_never_land_in_the_persisted_pact_fragment`, extended to the
`oauth` kind below).

Auth is injected on **every** HTTP request, including the `initialize` handshake.
stdio transports pass auth via env/args (no HTTP headers).

## OAuth2 (client credentials)

For machine-to-machine / CI verification against an OAuth2-protected MCP
server, use `type: "oauth"` — client-credentials (SEP-1046) is the only
supported grant (see ADR 0011 for why dynamic client registration / the
interactive authorization-code flow are out of scope for provider
verification):

```jsonc
{
  "type": "oauth",
  "grant": "client_credentials",       // optional — the only supported grant
  "clientId": "${MCP_OAUTH_CLIENT_ID}",
  "clientSecret": "${MCP_OAUTH_CLIENT_SECRET}",
  "scopes": ["mcp:verify"],            // optional
  "resource": "https://mcp.example.com/mcp" // optional; defaults to the server's base URL
}
```

Like the other auth kinds, `clientId`/`clientSecret` support `${ENV}`
interpolation and are never persisted to the pact. Unlike them, the token
isn't resolved up front: the engine runs rmcp's client-credentials flow
(discover metadata, exchange credentials for a token) when it connects to the
server, and the resulting `Authorization: Bearer …` is injected — and
refreshed — on every request by the same transport that handles static
headers. A failed exchange (bad credentials, unreachable authorization
server, missing `resource`) surfaces as a clear verification failure, the
same as a 401 does for the other auth kinds.

```ts
await new McpProviderVerifier({ provider: "weather-mcp", pactUrls: [...] })
  .withServerTransport({
    type: "http",
    url: "https://mcp.example.com/mcp",
    auth: {
      type: "oauth",
      clientId: "${MCP_OAUTH_CLIENT_ID}",
      clientSecret: "${MCP_OAUTH_CLIENT_SECRET}",
      scopes: ["mcp:verify"],
    },
  })
  .verify();
```

## Provider verification — standard pact-js Verifier (recommended)

With the plugin installed (`scripts/install-local.sh`, or a release install
under `~/.pact/plugins/mcp-<version>/`), MCP pacts verify through the **stock
pact-js `Verifier`** — no bespoke runner (ADR 0008). Pacts emitted by
`McpPact` are already stamped with the transport routing the verifier needs.

```ts
import { Verifier } from "@pact-foundation/pact";

// Streamable HTTP provider (running/deployed server):
await new Verifier({
  provider: "weather-mcp",
  providerBaseUrl: `http://127.0.0.1:${port}`,
  pactUrls: ["./pacts/weather-agent-weather-mcp.json"],
  transports: [{ protocol: "mcp-http", port }],
}).verifyProvider();

// stdio provider — the verifier can't carry a spawn spec, so pass it via env:
process.env.PACT_MCP_SERVER_COMMAND = "node";
process.env.PACT_MCP_SERVER_ARGS = "dist/server.js";
await new Verifier({
  provider: "weather-mcp",
  providerBaseUrl: "http://127.0.0.1:65500", // unused for plugin transports
  pactUrls: ["./pacts/weather-agent-weather-mcp.json"],
}).verifyProvider();
```

For `mcp-http`, auth comes from `PACT_MCP_AUTH` (a JSON auth config as below,
`${ENV}` interpolation applies) and a non-root endpoint path from
`PACT_MCP_SERVER_PATH`.

## Provider verification — `McpProviderVerifier` (thin wrapper)

`McpProviderVerifier` is a thin convenience wrapper over the same
`new Verifier(...)` shown above: it assembles the MCP transport config
(`transports` / `PACT_MCP_SERVER_COMMAND` / `PACT_MCP_AUTH` / …) and forwards
every other option to pact-js. So it inherits broker fetch, publishing,
`can-i-deploy`, and Pact's real reporting.

```ts
import { McpProviderVerifier } from "@pactflow/pact-mcp-plugin";

// stdio — the plugin spawns the real server per interaction
await new McpProviderVerifier({ provider: "weather-mcp", pactUrls: ["./pacts/…json"] })
  .withServerTransport({ type: "stdio", command: "node", args: ["dist/server.js"] })
  .verify();

// http — verify a running / deployed server, with auth
await new McpProviderVerifier({ provider: "weather-mcp", pactUrls: ["./pacts/…json"] })
  .withServerTransport({ type: "http", url: "https://mcp.example.com/mcp", auth: { type: "bearer", token: "${MCP_TOKEN}" } })
  .verify();

// from a broker, publishing results (the real-world path)
await new McpProviderVerifier({ provider: "weather-mcp", providerVersion: gitSha, publishVerificationResult: true })
  .withServerTransport({ type: "stdio", command: "node", args: ["dist/server.js"] })
  .fromPactBroker({ url: brokerUrl, token, consumerVersionSelectors: [{ mainBranch: true }] })
  .verify();
```

`.verify()` resolves with the verifier output and rejects (with pact-js's own
reporting) on failure. Use `.withVerifierOptions({ … })` to pass any raw pact-js
option through.

The Streamable HTTP client handles `Mcp-Session-Id`, `Accept: application/json,
text/event-stream`, and both JSON-body and SSE response modes automatically
(rmcp — see ADR 0007). A 401 from missing/invalid auth surfaces as a clear
verification failure.

The low-level engine also exposes a `verify` CLI (used by its gRPC path, not by
the adapter): `pact-mcp-plugin verify --pact pact.json --command node --arg dist/server.js`.

## Quieting engine logs

The engine is quiet by default (WARN). Raise verbosity with `PACT_MCP_LOG` or
`RUST_LOG` (full env-filter syntax), e.g. `PACT_MCP_LOG=info` or
`PACT_MCP_LOG=pact_mcp_plugin=debug,rmcp=warn`.

## Provider states

Declare states with `given(...)` (standard V4 `providerStates` — ADR 0009):

```ts
await new McpPact({ consumer: "weather-agent", provider: "weather-mcp" })
  .given("the Hobart weather is known", { city: "Hobart", weather: "Windy, 12C" })
  .whenClientCallsTool("get_weather", { city: "Hobart" })
  // ...
```

Apply them at verification time by whichever route fits your runner:

1. **Standard pact-js Verifier:** ordinary `stateHandlers` — they fire before
   each plugin-transport interaction with no extra wiring.
2. **Engine-spawned stdio servers:** the engine sets
   `PACT_MCP_PROVIDER_STATES` (JSON `[{name, params}]`) on the child process;
   the server seeds itself at startup. One spawn per interaction — no leakage.
3. **`McpProviderVerifier.stateHandlers({...})`:** in-process callbacks invoked
   before verification.

## Consumer mock over HTTP

```ts
import { McpPact, like } from "@pactflow/pact-mcp-plugin";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";

await new McpPact({ consumer: "weather-agent", provider: "weather-mcp", mockTransport: "http" })
  .whenClientCallsTool("get_weather", { city: "Melbourne" })
  .willRespondWith({ content: [{ type: "text", text: like("Sunny, 22C") }], isError: false })
  .executeTest(async ({ transport }) => {
    const client = new Client({ name: "weather-agent", version: "1.0.0" });
    await client.connect(transport); // real Streamable HTTP client -> loopback mock
    const res = await client.callTool({ name: "get_weather", arguments: { city: "Melbourne" } });
    expect(res.content[0].text).toContain("22C");
  });
```

`mockTransport: "http"` stands up a loopback Streamable HTTP MCP mock on an
ephemeral port; the default (`"stdio"`) uses the engine mock over a stdio pipe.
Matching is identical (Rust engine) for both.
