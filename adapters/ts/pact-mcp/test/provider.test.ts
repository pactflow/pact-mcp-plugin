import { describe, it, expect } from "vitest";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { McpPact, McpProviderVerifier, like } from "../src";

const repoRoot = join(__dirname, "..", "..", "..", "..");
const fixtureServer = join(repoRoot, "examples", "fixtures", "weather-server.mjs");

async function emitPact(dir: string, expectedText: unknown): Promise<string> {
  await new McpPact({ consumer: "weather-agent", provider: "weather-mcp", dir })
    .whenClientCallsTool("get_weather", { city: "Melbourne" })
    .willRespondWith({ content: [{ type: "text", text: expectedText }], isError: false })
    .executeTest(async ({ transport }) => {
      const client = new Client({ name: "weather-agent", version: "1.0.0" });
      await client.connect(transport);
      await client.callTool({ name: "get_weather", arguments: { city: "Melbourne" } });
      await client.close();
    });
  return join(dir, "weather-agent-weather-mcp.json");
}

// These assert config assembly only — no engine or verifier needed, so they're
// fast and deterministic. They pin that the wrapper delegates to pact-js.
describe("McpProviderVerifier — pact-js VerifierOptions assembly", () => {
  it("maps a stdio transport to server-spawn env (no HTTP transport entry)", () => {
    const { options, env } = new McpProviderVerifier({ provider: "weather-mcp", pactUrls: ["p.json"] })
      .withServerTransport({ type: "stdio", command: "node", args: ["dist/server.js"] })
      .buildVerifierConfig();

    expect(env.PACT_MCP_SERVER_COMMAND).toBe("node");
    expect(env.PACT_MCP_SERVER_ARGS).toBe("dist/server.js");
    expect(options.transports).toBeUndefined();
    expect(options.pactUrls).toEqual(["p.json"]);
    expect(options.providerBaseUrl).toMatch(/^https?:\/\//);
  });

  it("maps an http transport to a mcp-http transport entry + path/auth env", () => {
    const { options, env } = new McpProviderVerifier({ provider: "weather-mcp", pactUrls: ["p.json"] })
      .withServerTransport({
        type: "http",
        url: "https://mcp.example.com:8443/mcp",
        auth: { type: "bearer", token: "${T}" },
      })
      .buildVerifierConfig();

    expect(options.transports).toEqual([{ protocol: "mcp-http", port: 8443 }]);
    expect(options.providerBaseUrl).toBe("https://mcp.example.com:8443");
    expect(env.PACT_MCP_SERVER_PATH).toBe("/mcp");
    expect(JSON.parse(env.PACT_MCP_AUTH)).toEqual({ type: "bearer", token: "${T}" });
  });

  // ADR 0011 / plan T5: no engine round-trip here (that's the deferred e2e) —
  // just pin that an `oauth` HttpAuth serializes to PACT_MCP_AUTH the same
  // way the other auth kinds do.
  it("maps an oauth client-credentials auth to PACT_MCP_AUTH", () => {
    const { env } = new McpProviderVerifier({ provider: "weather-mcp", pactUrls: ["p.json"] })
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
      .buildVerifierConfig();

    expect(JSON.parse(env.PACT_MCP_AUTH)).toEqual({
      type: "oauth",
      clientId: "${MCP_OAUTH_CLIENT_ID}",
      clientSecret: "${MCP_OAUTH_CLIENT_SECRET}",
      scopes: ["mcp:verify"],
    });
  });

  it("maps an oauth auth with an explicit grant and resource to PACT_MCP_AUTH", () => {
    const { env } = new McpProviderVerifier({ provider: "weather-mcp", pactUrls: ["p.json"] })
      .withServerTransport({
        type: "http",
        url: "https://mcp.example.com/mcp",
        auth: {
          type: "oauth",
          grant: "client_credentials",
          clientId: "my-client",
          clientSecret: "my-secret",
          resource: "https://mcp.example.com/mcp",
        },
      })
      .buildVerifierConfig();

    expect(JSON.parse(env.PACT_MCP_AUTH)).toEqual({
      type: "oauth",
      grant: "client_credentials",
      clientId: "my-client",
      clientSecret: "my-secret",
      resource: "https://mcp.example.com/mcp",
    });
  });

  it("forwards broker source, selectors, publish + state handlers to pact-js", () => {
    const handler = async () => ({});
    const { options } = new McpProviderVerifier({
      provider: "weather-mcp",
      providerVersion: "abc123",
      providerVersionBranch: "main",
      publishVerificationResult: true,
    })
      .withServerTransport({ type: "stdio", command: "node", args: ["s.js"] })
      .fromPactBroker({
        url: "https://broker.example",
        token: "tok",
        consumerVersionSelectors: [{ mainBranch: true }],
        enablePending: true,
      })
      .stateHandlers({ "weather is available": handler })
      .buildVerifierConfig();

    expect(options.pactBrokerUrl).toBe("https://broker.example");
    expect(options.pactBrokerToken).toBe("tok");
    expect(options.consumerVersionSelectors).toEqual([{ mainBranch: true }]);
    expect(options.enablePending).toBe(true);
    expect(options.publishVerificationResult).toBe(true);
    expect(options.providerVersion).toBe("abc123");
    expect(options.providerVersionBranch).toBe("main");
    expect((options.stateHandlers as Record<string, unknown>)["weather is available"]).toBe(handler);
  });

  it("requires a server transport and a pact source", () => {
    expect(() => new McpProviderVerifier({ provider: "x" }).buildVerifierConfig()).toThrow(/server transport/i);
    expect(() =>
      new McpProviderVerifier({ provider: "x" })
        .withServerTransport({ type: "stdio", command: "node" })
        .buildVerifierConfig()
    ).toThrow(/pact/i);
  });
});

describe("McpProviderVerifier — end to end via the standard pact-js Verifier", () => {
  it("verifies a passing pact against the real fixture server over stdio", async () => {
    const dir = mkdtempSync(join(tmpdir(), "pact-prov-"));
    const pact = await emitPact(dir, like("Sunny, 22C"));

    const output = await new McpProviderVerifier({ provider: "weather-mcp", pactUrls: [pact], logLevel: "error" })
      .withServerTransport({ type: "stdio", command: "node", args: [fixtureServer] })
      .verify();

    expect(JSON.parse(output).errors).toEqual([]);
  }, 120000);

  it("rejects with pact-js's own reporting when the provider does not satisfy the pact", async () => {
    const dir = mkdtempSync(join(tmpdir(), "pact-prov-"));
    // Exact text the fixture won't return -> a real mismatch at verification.
    const pact = await emitPact(dir, "Totally wrong forecast");

    const run = new McpProviderVerifier({ provider: "weather-mcp", pactUrls: [pact], logLevel: "error" })
      .withServerTransport({ type: "stdio", command: "node", args: [fixtureServer] })
      .verify();

    await expect(run).rejects.toThrow();
  }, 120000);
});
