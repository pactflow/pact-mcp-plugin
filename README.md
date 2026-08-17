# Pact MCP Plugin

Consumer-driven **contract testing for [Model Context Protocol (MCP)](https://modelcontextprotocol.io)** servers and clients, built as a [Pact](https://pact.io) plugin.

Test that an AI agent (MCP **client**) and the MCP **server** it depends on agree on the shape of `tools/call`, `tools/list`, `resources/read|list`, `prompts/get|list`, and their results — without spinning up the whole stack. You author expectations against your **real** `@modelcontextprotocol/sdk` `Client`, and verify them against your **real** MCP server. No stubbing, no service decomposition.

## Why

- **MCP has no contract-testing story.** Agents break when a server changes a tool's arguments or result shape; servers break their consumers without knowing.
- Core Pact can't test MCP: it has **no stdio transport** (and most MCP servers are stdio-first), no Streamable HTTP/SSE handling, and no knowledge of the MCP handshake or tool semantics. This plugin adds all of that.
- **One matching engine, every language.** Matching lives once in a Rust engine; language adapters are thin DX layers over it, so pacts are portable (author in TS, verify anywhere).

## Capabilities

| | stdio | Streamable HTTP |
|---|---|---|
| Provider verification | ✅ | ✅ (+ bearer / API key / custom headers) |
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

## License

[MIT](LICENSE) © 2026 Matt Fellows
