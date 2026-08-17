# Plan — OAuth2 auth for MCP provider verification (Phase 4, scoped)

**Status:** Ready for implementation (handoff to a Sonnet agent).
**Scope decided:** Primary flow = **OAuth 2.0 client-credentials** (machine-to-machine,
CI-friendly). Tests = **engine + TS DSL wiring with unit/mocked tests**; the full
fixture-authorization-server e2e is **deferred** to a follow-up. Dynamic client
registration (RFC 7591) is **not on the CI critical path** — see §1.3.

This plan supersedes the one-liner in
`docs/plans/pact-mcp-plugin-implementation-plan.md` §11 "Phase 4" and refines ADR
0007's "leave the trait seam for OAuth2" note. Land a new ADR 0011 as task 0.

---

## 1. Impact analysis

### 1.1 The decisive finding — rmcp already implements OAuth2

`rmcp` **2.2.0** (our pinned dep, `rust/pact-mcp-plugin/Cargo.toml:25`) ships a
complete OAuth2 implementation in
`~/.cargo/.../rmcp-2.2.0/src/transport/auth.rs`, gated behind the `auth` cargo
feature (which pulls in the `oauth2` crate). We are **not** hand-rolling token
endpoints, discovery, or RFC 7591. The relevant public surface:

- `OAuthState` — the flow state machine. `OAuthState::new(base_url, Some(reqwest_client))`
  → `Unauthorized`.
- `OAuthState::authenticate_client_credentials(ClientCredentialsConfig)` — SEP-1046:
  discovers metadata (RFC 8414), validates the auth method, configures the client,
  exchanges credentials for a token, transitions to `Authorized`. **One call does
  the whole client-credentials flow.**
- `ClientCredentialsConfig::ClientSecret { client_id, client_secret, scopes, resource }`
  (and an optional `PrivateKeyJwt` variant behind `auth-client-credentials-jwt`).
- `OAuthState::into_authorization_manager() -> Option<AuthorizationManager>`.
- `AuthClient<C>` (`AuthClient::new(http_client, auth_manager)`) — **implements
  `StreamableHttpClient`** (`src/transport/common/auth/streamable_http_client.rs`),
  so it drops straight into
  `StreamableHttpClientTransport::with_client(auth_client, config)`.

That last point is what makes this cheap: our HTTP transport already uses rmcp's
`StreamableHttpClientTransport`; OAuth is a **different client injected into the
same transport**, not a new transport.

**Verified integration path (this is the whole engine mechanism):**

```rust
// resource is REQUIRED by rmcp/MCP auth spec — exchange errors without it.
let mut oauth = OAuthState::new(&base_url, Some(reqwest_client_for_oauth)).await?;
oauth.authenticate_client_credentials(ClientCredentialsConfig::ClientSecret {
    client_id, client_secret, scopes, resource: Some(resource),
}).await?;
let manager = oauth.into_authorization_manager().expect("authorized");
let auth_client = AuthClient::new(reqwest_client_for_transport, manager);
let transport = StreamableHttpClientTransport::with_client(auth_client, config);
```

### 1.2 Blast radius (small, additive)

| Area | File | Change |
|---|---|---|
| Cargo features | `rust/pact-mcp-plugin/Cargo.toml` | Add `"auth"` to the rmcp feature list; adds the `oauth2` crate + reqwest 0.13 (already transitively present per `Cargo.lock`). |
| Auth model | `src/auth.rs` | New `oauth` kind in `from_config`. `ResolvedAuth` must grow an OAuth-carrying variant — see §1.4. This is the one non-trivial refactor. |
| HTTP transport | `src/transport/http.rs` | `HttpClient::connect` branches: static-header path (today) vs OAuth path (build `AuthClient` + `with_client`). |
| Verify wiring | `src/verify.rs` | `verify_interaction_http` already takes `&ResolvedAuth`; no signature change if OAuth is carried inside `ResolvedAuth`. |
| gRPC entry | `src/server.rs:396` | `PACT_MCP_AUTH` JSON already parsed + `resolve_config`'d; an `oauth` type flows through unchanged. |
| CLI | `src/cli.rs` | `--auth <json>` already `resolve_config`s; unchanged. |
| TS DSL | `adapters/ts/pact-mcp/src/provider.ts:13` | Add `oauth` arm to the `HttpAuth` union; it already serializes to `PACT_MCP_AUTH`. Effectively free. |
| Docs | `docs/usage.md`, `README.md` | New auth stanza. |
| ADR | `docs/decisions/0011-*.md` | Record the decision. |

**No change** to: matching engine, mock server, stdio transport, provider-states,
consumer DSL, the persisted-secret invariant (OAuth creds are config-only, same as
today's `${ENV}` secrets — the `secrets_never_land_in_the_persisted_pact_fragment`
test still holds by construction).

### 1.3 Why dynamic client registration is off the CI path (important)

rmcp's `AuthorizationManager::register_client` (RFC 7591) hardcodes
`grant_types: ["authorization_code", "refresh_token"]`,
`token_endpoint_auth_method: "none"` (public client), `response_types: ["code"]`.
That is the **interactive** flow — it requires a browser redirect and human consent,
which cannot run unattended in CI. Client-credentials (SEP-1046) uses a
**pre-registered** `client_id`/`client_secret`; there is no registration step at
verify time. So "dynamic client registration" and "runs headless in CI" are
mutually exclusive with rmcp as-is.

**Consequence for this plan:** we ship **client-credentials**. The `AuthProvider`
seam and rmcp wiring we add make an interactive-DCR path (for the consumer/agent
side, or a future attended provider verify) a purely additive follow-up — it reuses
the same `OAuthState`/`AuthClient` plumbing with a different terminal grant. Note
this explicitly in ADR 0011 so the "dynamic registration" language in the master
plan isn't read as delivered.

### 1.4 The one real design decision — carrying OAuth through `ResolvedAuth`

Today `ResolvedAuth { auth_header, custom_headers }` is a resolved *static* header
set, and `AuthProvider::resolve()` is **sync**. OAuth can't resolve to a static
header synchronously — it needs an async token exchange against the live server,
and the token must be *refreshable* by the transport (rmcp's `AuthClient` owns that).
So the token is **not** pre-resolved into a header. Recommended shape:

```rust
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedAuth {
    pub auth_header: Option<String>,
    pub custom_headers: Vec<(String, String)>,
    pub oauth: Option<OAuthClientCredentials>, // NEW: carried, not resolved to a header
}

#[derive(Debug, Clone, PartialEq)]
pub struct OAuthClientCredentials {
    pub client_id: String,
    pub client_secret: String,   // ${ENV}-interpolated at resolve() time
    pub scopes: Vec<String>,
    pub resource: Option<String>, // defaults to the server base URL if omitted (rmcp requires it)
}
```

`resolve()` interpolates `${ENV}` on `client_secret`/`client_id` (reuse
`interpolate_env`) and populates `oauth: Some(...)`. The **async token exchange
happens in the transport** (`HttpClient::connect`), where we have the base URL and
an async context — not in `resolve()`. Keep `resolve()` sync; it just carries the
config. `apiKey`/`headers`/`bearer` are unchanged.

### 1.5 Risks / gotchas to hand to the implementer

- **`resource` is mandatory.** `exchange_client_credentials` returns
  `ClientCredentialsError("resource parameter is required by the MCP auth spec")`
  if `resource` is `None`. Default it to the MCP server base URL when the user
  omits it.
- **Feature bloat / build time.** The `auth` feature adds `oauth2` + its deps.
  Confirm CI build time and the release binary size are acceptable (they should be;
  reqwest 0.13 is already pulled).
- **Two reqwest clients.** rmcp's OAuth path and the transport path each want a
  `reqwest::Client`. Building two is fine; don't try to share unless trivial.
- **Secret hygiene.** `client_secret` is `${ENV}`-interpolated and lives only on
  the transport config; assert it never lands in the pact (extend the existing
  invariant test to the `oauth` kind).
- **Error surfacing.** A failed token exchange (bad creds, unreachable AS,
  missing `resource`) must surface as a clear verification error, mirroring how a
  401 surfaces today (`verify.rs` maps transport errors → `VerifyError::Transport`).

---

## 2. Config contract (the wire + DSL surface)

`PACT_MCP_AUTH` / `--auth` / DSL `auth` all accept the same JSON:

```jsonc
{
  "type": "oauth",
  "grant": "client_credentials",       // only supported grant in this phase
  "clientId": "${MCP_OAUTH_CLIENT_ID}",
  "clientSecret": "${MCP_OAUTH_CLIENT_SECRET}",
  "scopes": ["mcp:verify"],            // optional
  "resource": "https://mcp.example.com/mcp" // optional; defaults to server base URL
}
```

TS DSL addition (`provider.ts` `HttpAuth` union):

```ts
| { type: "oauth"; grant?: "client_credentials"; clientId: string;
    clientSecret: string; scopes?: string[]; resource?: string }
```

Usage stays identical to the existing http auth path — the wrapper already
`JSON.stringify`s `auth` into `PACT_MCP_AUTH` (`provider.ts:177`):

```ts
await new McpProviderVerifier({ provider: "weather-mcp", pactUrls: [...] })
  .withServerTransport({ type: "http", url: "https://mcp.example.com/mcp",
    auth: { type: "oauth", clientId: "${MCP_OAUTH_CLIENT_ID}",
            clientSecret: "${MCP_OAUTH_CLIENT_SECRET}", scopes: ["mcp:verify"] } })
  .verify();
```

---

## 3. Task breakdown (TDD, bite-sized, commit per task)

Follow the repo convention: failing test first, `refactor:`/`feat:` conventional
commits, keep the persisted-secret invariant green throughout.

- [ ] **T0 — ADR 0011.** `docs/decisions/0011-oauth-client-credentials.md`: record
  client-credentials-first, the rmcp reuse, the DCR/CI incompatibility (§1.3), and
  the `ResolvedAuth.oauth` shape (§1.4). *(No code.)*

- [ ] **T1 — Enable rmcp `auth` feature.** Add `"auth"` to the rmcp features in
  `Cargo.toml`. `cargo build` + `cargo test` still green; note build-time delta.

- [ ] **T2 — `auth.rs` parse + carry (unit).** Add `OAuthClientCredentials` +
  `ResolvedAuth.oauth`; parse `type:"oauth"` in `from_config` (reject grants other
  than `client_credentials` with a clear error); `${ENV}`-interpolate secrets in
  `resolve()`. Tests: parse ok; unknown grant errors; missing `clientId`/`clientSecret`
  errors; `${ENV}` resolves; missing env errors (mirror existing bearer tests);
  extend `secrets_never_land_in_the_persisted_pact_fragment` to the oauth kind.

- [ ] **T3 — Transport OAuth path (unit + mocked token endpoint).**
  In `http.rs`, when `auth.oauth.is_some()`: build `OAuthState::new(base_url, Some(client))`,
  `authenticate_client_credentials(...)` (default `resource` to base URL),
  `into_authorization_manager()`, wrap in `AuthClient::new(...)`, and connect via
  `StreamableHttpClientTransport::with_client(auth_client, config)`. Otherwise keep
  today's static-header path unchanged. Test with a **mocked token endpoint**
  (small hyper/axum stub, or rmcp's own test doubles — see the `#[cfg(test)]` block
  at the bottom of rmcp's `auth.rs` for the `OAuthHttpClient` seam it uses) that
  returns an `access_token`; assert the transport sends `Authorization: Bearer …`
  on the MCP request. **Do not** stand up a full AS or protected MCP server here
  (deferred).

- [ ] **T4 — End-to-end wiring through `server.rs`/`verify.rs` (unit).** Confirm an
  `oauth` `PACT_MCP_AUTH` flows `server.rs:396` → `resolve_config` → 
  `verify_interaction_http` unchanged. Add a focused test asserting the oauth config
  reaches the transport (can reuse the T3 mock). Ensure token-exchange failure maps
  to a clear `VerifyError`.

- [ ] **T5 — TS DSL.** Add the `oauth` arm to `HttpAuth` in `provider.ts`; a unit
  test on `buildVerifierConfig` asserting `PACT_MCP_AUTH` carries the oauth JSON.
  No engine round-trip needed (that's the deferred e2e).

- [ ] **T6 — Docs.** `docs/usage.md` (new "OAuth2 (client credentials)" subsection
  under auth) + a `README.md` capabilities-table row. Note the `resource` default
  and that secrets use `${ENV}` and never hit the pact.

- [ ] **T7 — Plan checkboxes.** Tick the relevant Phase 4 line in
  `pact-mcp-plugin-implementation-plan.md` and note client-credentials-only + DCR
  deferred; add a "Deferred" bullet for the fixture-AS e2e.

**Deferred (explicitly NOT this task, create as follow-up issues):**
- Fixture authorization server (`/.well-known/*`, `/token`, optional `/register`) +
  protected MCP fixture + full discover→token→verify round-trip e2e.
- Interactive authorization-code + RFC 7591 dynamic registration (consumer/agent
  side or attended provider verify).
- `private_key_jwt` client-credentials (`auth-client-credentials-jwt` feature).
- Token caching / refresh across interactions (rmcp's `AuthClient` already refreshes
  within a session; cross-spawn caching is a separate optimization).

---

## 4. Definition of done

- `cargo test` (engine) + `npm test` (TS adapter) green, including the new oauth
  unit/mocked tests and the extended secret-never-persisted invariant.
- `type:"oauth"` config parses, resolves `${ENV}`, and drives a mocked
  client-credentials exchange whose token is injected as `Authorization: Bearer …`
  on MCP requests.
- TS `HttpAuth` accepts `oauth` and serializes it to `PACT_MCP_AUTH`.
- ADR 0011 + usage/README docs landed; master-plan checkboxes updated.
- CI green on the branch.

---

## 5. Key source references (verified in rmcp 2.2.0)

- `src/transport/auth.rs`: `OAuthState` (2910), `authenticate_client_credentials`
  (3236), `ClientCredentialsConfig` (699), `exchange_client_credentials` (2440,
  `resource` required), `AuthClient` (430/446), `register_client` (1227, authz-code
  only — see §1.3), `into_authorization_manager` (3240).
- `src/transport/common/auth/streamable_http_client.rs:9`: `impl StreamableHttpClient
  for AuthClient<C>`.
- `src/transport/streamable_http_client.rs:1248`: `with_client(client, config)`.
- Our seams: `auth.rs` (`from_config`, `resolve_config`, `ResolvedAuth`,
  `interpolate_env`), `transport/http.rs` (`HttpClient::connect`), `server.rs:396`
  (`PACT_MCP_AUTH`), `verify.rs:57` (`verify_interaction_http`),
  `adapters/ts/pact-mcp/src/provider.ts:13/177` (`HttpAuth`, `PACT_MCP_AUTH`).
