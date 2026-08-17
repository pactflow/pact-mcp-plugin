// McpProviderVerifier — a THIN wrapper over the standard pact-js `Verifier`.
//
// All verification, broker interaction (fetch pacts, consumer version
// selectors, publish results, can-i-deploy), provider-state invocation, and
// reporting are handled by pact-js itself. The Rust engine participates as the
// installed Pact `mcp` plugin: pact-js routes each interaction to it over the
// stamped `transport` (mcp-stdio / mcp-http — see ADR 0008). This wrapper only
// assembles the MCP-specific transport config and forwards everything else.

import { Verifier, VerifierOptions } from "@pact-foundation/pact";

/** Auth config for an HTTP target (values may use `${ENV}` interpolation; never persisted). */
export type HttpAuth =
  | { type: "bearer"; token: string }
  | { type: "apiKey"; header: string; value: string }
  | { type: "headers"; headers: Record<string, string> }
  | {
      /** OAuth 2.0 client-credentials (SEP-1046). Only supported grant — see ADR 0011. */
      type: "oauth";
      grant?: "client_credentials";
      clientId: string;
      clientSecret: string;
      scopes?: string[];
      /** RFC 8707 resource indicator; defaults to the server's base URL when omitted. */
      resource?: string;
    };

/** Provider-state setup callback, keyed by state name; receives the state params. */
export type StateHandler = (params: Record<string, unknown>) => Promise<unknown> | unknown;

export interface StdioServerTransport {
  type: "stdio";
  /** Command to launch the real provider MCP server (e.g. "node"). */
  command: string;
  /** Arguments (e.g. ["dist/server.js"]). */
  args?: string[];
  /** Extra env for the spawned server (e.g. to seed provider state). */
  env?: Record<string, string>;
}

export interface HttpServerTransport {
  type: "http";
  /** URL of the running MCP server's Streamable HTTP endpoint (e.g. https://host/mcp). */
  url: string;
  /** Optional auth injected on every request; secrets may use `${ENV}`. */
  auth?: HttpAuth;
}

export type ServerTransport = StdioServerTransport | HttpServerTransport;

/** A subset of pact-js consumer version selectors (the common fields). */
export interface ConsumerVersionSelector {
  tag?: string;
  latest?: boolean;
  branch?: string;
  mainBranch?: boolean;
  matchingBranch?: boolean;
  deployedOrReleased?: boolean;
  deployed?: boolean;
  released?: boolean;
  environment?: string;
  consumer?: string;
  fallbackTag?: string;
}

/** Fetch pacts from a Pact Broker / PactFlow rather than local files. */
export interface PactBrokerSource {
  url: string;
  /** Bearer token (PactFlow) — or use username/password for Basic auth. */
  token?: string;
  username?: string;
  password?: string;
  consumerVersionSelectors?: ConsumerVersionSelector[];
  enablePending?: boolean;
  includeWipPactsSince?: string;
}

export interface McpProviderVerifierOptions {
  provider: string;
  /** Local pact files to verify (or use `.fromPactBroker(...)`). */
  pactUrls?: string[];
  /** Provider application version — required to publish results. */
  providerVersion?: string;
  providerVersionBranch?: string;
  /** Publish verification results back to the broker. */
  publishVerificationResult?: boolean;
  logLevel?: VerifierOptions["logLevel"];
}

/** Set env vars, returning the previous values so they can be restored. */
function applyEnv(env: Record<string, string>): Record<string, string | undefined> {
  const saved: Record<string, string | undefined> = {};
  for (const [k, v] of Object.entries(env)) {
    saved[k] = process.env[k];
    process.env[k] = v;
  }
  return saved;
}

function restoreEnv(saved: Record<string, string | undefined>): void {
  for (const [k, v] of Object.entries(saved)) {
    if (v === undefined) delete process.env[k];
    else process.env[k] = v;
  }
}

export class McpProviderVerifier {
  private serverTransport?: ServerTransport;
  private handlers: Record<string, StateHandler> = {};
  private broker?: PactBrokerSource;
  private pactFiles: string[];
  private extra: Partial<VerifierOptions> = {};

  constructor(private readonly opts: McpProviderVerifierOptions) {
    this.pactFiles = [...(opts.pactUrls ?? [])];
  }

  /**
   * The provider's real MCP server:
   *  - `{ type: 'stdio', command, args, env? }` — the plugin spawns it per interaction.
   *  - `{ type: 'http', url, auth? }` — verify a running/deployed server.
   */
  withServerTransport(transport: ServerTransport): this {
    this.serverTransport = transport;
    return this;
  }

  /** Add local pact files to verify. */
  withPactFiles(urls: string[]): this {
    this.pactFiles.push(...urls);
    return this;
  }

  /** Fetch pacts from a Pact Broker / PactFlow (selectors, pending, publish, …). */
  fromPactBroker(broker: PactBrokerSource): this {
    this.broker = broker;
    return this;
  }

  /** Standard pact-js provider-state handlers, keyed by state name. */
  stateHandlers(handlers: Record<string, StateHandler>): this {
    this.handlers = { ...this.handlers, ...handlers };
    return this;
  }

  /** Escape hatch: merge any raw pact-js `VerifierOptions` (request filters, hooks, …). */
  withVerifierOptions(options: Partial<VerifierOptions>): this {
    this.extra = { ...this.extra, ...options };
    return this;
  }

  /**
   * Assemble the pact-js `VerifierOptions` and the env vars the MCP transport
   * needs. Pure and side-effect free (the env is returned, not applied) so it
   * can be unit-tested.
   */
  buildVerifierConfig(): { options: VerifierOptions; env: Record<string, string> } {
    if (!this.serverTransport) {
      throw new Error(
        "A server transport is required — call withServerTransport({ type: 'stdio' | 'http', ... })."
      );
    }
    if (this.pactFiles.length === 0 && !this.broker) {
      throw new Error(
        "A pact source is required — pass pactUrls / withPactFiles([...]) or fromPactBroker({ url, ... })."
      );
    }

    const t = this.serverTransport;
    const env: Record<string, string> = {};
    let providerBaseUrl: string;
    let transports: VerifierOptions["transports"];

    if (t.type === "stdio") {
      // pact-js can't carry a spawn spec, so the plugin reads it from env (ADR 0008).
      env.PACT_MCP_SERVER_COMMAND = t.command;
      env.PACT_MCP_SERVER_ARGS = (t.args ?? []).join(" ");
      for (const [k, v] of Object.entries(t.env ?? {})) env[k] = v;
      // providerBaseUrl is unused for a plugin/stdio transport but pact-js requires a URL.
      providerBaseUrl = "http://127.0.0.1:0";
    } else {
      const u = new URL(t.url);
      const port = Number(u.port || (u.protocol === "https:" ? "443" : "80"));
      providerBaseUrl = `${u.protocol}//${u.hostname}:${port}`;
      transports = [{ protocol: "mcp-http", port }];
      if (u.pathname && u.pathname !== "/") env.PACT_MCP_SERVER_PATH = u.pathname;
      if (t.auth) env.PACT_MCP_AUTH = JSON.stringify(t.auth);
    }

    const options: VerifierOptions = {
      provider: this.opts.provider,
      providerBaseUrl,
      ...(transports ? { transports } : {}),
      ...(this.pactFiles.length ? { pactUrls: this.pactFiles } : {}),
      ...(this.broker
        ? {
            pactBrokerUrl: this.broker.url,
            ...(this.broker.token ? { pactBrokerToken: this.broker.token } : {}),
            ...(this.broker.username ? { pactBrokerUsername: this.broker.username } : {}),
            ...(this.broker.password ? { pactBrokerPassword: this.broker.password } : {}),
            ...(this.broker.consumerVersionSelectors
              ? { consumerVersionSelectors: this.broker.consumerVersionSelectors }
              : {}),
            ...(this.broker.enablePending !== undefined ? { enablePending: this.broker.enablePending } : {}),
            ...(this.broker.includeWipPactsSince
              ? { includeWipPactsSince: this.broker.includeWipPactsSince }
              : {}),
          }
        : {}),
      ...(this.opts.providerVersion ? { providerVersion: this.opts.providerVersion } : {}),
      ...(this.opts.providerVersionBranch ? { providerVersionBranch: this.opts.providerVersionBranch } : {}),
      ...(this.opts.publishVerificationResult !== undefined
        ? { publishVerificationResult: this.opts.publishVerificationResult }
        : {}),
      ...(this.opts.logLevel ? { logLevel: this.opts.logLevel } : {}),
      ...(Object.keys(this.handlers).length
        ? { stateHandlers: this.handlers as unknown as VerifierOptions["stateHandlers"] }
        : {}),
      ...this.extra,
    };

    return { options, env };
  }

  /**
   * Run the standard pact-js verifier against the provider's MCP server.
   * Resolves with the verifier output; rejects (with pact-js's own reporting)
   * if any interaction fails.
   */
  async verify(): Promise<string> {
    const { options, env } = this.buildVerifierConfig();
    const saved = applyEnv(env);
    try {
      return await new Verifier(options).verifyProvider();
    } finally {
      restoreEnv(saved);
    }
  }
}
