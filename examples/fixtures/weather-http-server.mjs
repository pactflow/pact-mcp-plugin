#!/usr/bin/env node
// Real Streamable HTTP MCP fixture server exposing a `get_weather` tool.
//
// Optionally auth-protected via env vars (checked BEFORE dispatch, incl. the
// initialize POST):
//   REQUIRE_BEARER=<token>                      -> requires `Authorization: Bearer <token>`
//   REQUIRE_API_KEY_HEADER + REQUIRE_API_KEY_VALUE -> requires that header == value
// Optionally serves a mocked OAuth2 client-credentials token endpoint on the
// SAME origin (so rmcp's discovery legacy-fallback `/token` derivation finds
// it — see ADR 0011 T3): unauthenticated `POST /token` always returns
//   OAUTH_ACCESS_TOKEN=<token> -> {"access_token": "<token>", "token_type": "Bearer", "expires_in": 3600}
// Combine with REQUIRE_BEARER=<same token> to simulate the resource server
// accepting the exchanged token. This is a unit-test stub, NOT a real
// authorization server (no discovery metadata, no client validation).
// Prints one line `{"port":<n>}` to stdout once listening (so tests can capture
// the ephemeral port). PORT env overrides the port (0 = ephemeral).
import { createServer } from "node:http";
import { randomUUID } from "node:crypto";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/streamableHttp.js";
import { z } from "zod";

const WEATHER = { Melbourne: "Sunny, 22C", Sydney: "Cloudy, 19C" };

function buildServer() {
  const server = new McpServer({ name: "weather-http-fixture", version: "1.0.0" });
  server.registerTool(
    "get_weather",
    { title: "Get Weather", description: "Get the current weather for a city", inputSchema: { city: z.string() } },
    async ({ city }) => {
      const text = WEATHER[city];
      if (!text) return { isError: true, content: [{ type: "text", text: `Unknown city: ${city}` }] };
      return { content: [{ type: "text", text }], isError: false };
    }
  );
  return server;
}

const REQUIRE_BEARER = process.env.REQUIRE_BEARER;
const REQUIRE_API_KEY_HEADER = process.env.REQUIRE_API_KEY_HEADER;
const REQUIRE_API_KEY_VALUE = process.env.REQUIRE_API_KEY_VALUE;
const OAUTH_ACCESS_TOKEN = process.env.OAUTH_ACCESS_TOKEN;

function authOk(req) {
  if (REQUIRE_BEARER) {
    return req.headers["authorization"] === `Bearer ${REQUIRE_BEARER}`;
  }
  if (REQUIRE_API_KEY_HEADER) {
    return req.headers[REQUIRE_API_KEY_HEADER.toLowerCase()] === REQUIRE_API_KEY_VALUE;
  }
  return true;
}

// Session id -> transport.
const transports = {};

async function readBody(req) {
  const chunks = [];
  for await (const chunk of req) chunks.push(chunk);
  if (chunks.length === 0) return undefined;
  return JSON.parse(Buffer.concat(chunks).toString("utf8"));
}

const httpServer = createServer(async (req, res) => {
  // Mocked token endpoint (unauthenticated, on purpose — the token exchange
  // request itself carries the client credentials, not a bearer token).
  if (OAUTH_ACCESS_TOKEN && req.method === "POST" && req.url.split("?")[0] === "/token") {
    for await (const _chunk of req) {
      // drain the client_credentials-grant body; this stub doesn't validate it.
    }
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ access_token: OAUTH_ACCESS_TOKEN, token_type: "Bearer", expires_in: 3600 }));
    return;
  }

  if (!authOk(req)) {
    res.writeHead(401, { "Content-Type": "application/json", "WWW-Authenticate": "Bearer" });
    res.end(JSON.stringify({ error: "unauthorized" }));
    return;
  }

  const sessionId = req.headers["mcp-session-id"];
  let transport;

  if (req.method === "POST") {
    const body = await readBody(req);
    if (sessionId && transports[sessionId]) {
      transport = transports[sessionId];
    } else {
      // New session (initialize).
      transport = new StreamableHTTPServerTransport({
        sessionIdGenerator: () => randomUUID(),
        onsessioninitialized: (sid) => {
          transports[sid] = transport;
        },
      });
      transport.onclose = () => {
        if (transport.sessionId) delete transports[transport.sessionId];
      };
      const mcp = buildServer();
      await mcp.connect(transport);
    }
    await transport.handleRequest(req, res, body);
    return;
  }

  if ((req.method === "GET" || req.method === "DELETE") && sessionId && transports[sessionId]) {
    await transports[sessionId].handleRequest(req, res);
    return;
  }

  res.writeHead(400).end("bad request");
});

const port = process.env.PORT ? Number(process.env.PORT) : 0;
httpServer.listen(port, "127.0.0.1", () => {
  const addr = httpServer.address();
  process.stdout.write(JSON.stringify({ port: addr.port }) + "\n");
});
