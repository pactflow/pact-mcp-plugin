# ADR 0011: OAuth 2.0 client-credentials auth for MCP provider verification

## Status
Accepted

## Context
Phase 4 of the master plan calls for OAuth2 auth on the HTTP transport, noted in
ADR 0007 as a clean trait seam left for later. `docs/plans/oauth-client-credentials-plan.md`
is the detailed impact analysis and task breakdown behind this ADR; this record
captures the decisions that plan reached.

## Investigation (rmcp 2.2.0)
`rmcp` ships a complete OAuth2 implementation behind the `auth` cargo feature
(`src/transport/auth.rs`), pulling in the `oauth2` crate and an optional
`reqwest` client. We are not hand-rolling token endpoints, RFC 8414 discovery,
or RFC 7591 dynamic registration. The relevant public surface:

- `OAuthState::new(base_url, Some(reqwest_client))` → `Unauthorized`.
- `OAuthState::authenticate_client_credentials(ClientCredentialsConfig)`
  (SEP-1046) — discovers metadata, validates the auth method, configures the
  client, exchanges credentials for a token, transitions to `Authorized`. One
  call performs the whole client-credentials flow, including falling back to
  "legacy" `/token` + `/authorize` endpoints derived from the base URL when no
  `/.well-known/*` metadata is published.
- `OAuthState::into_authorization_manager() -> Option<AuthorizationManager>`.
- `AuthClient::new(http_client, auth_manager)` implements `StreamableHttpClient`
  (`src/transport/common/auth/streamable_http_client.rs`), so it drops straight
  into `StreamableHttpClientTransport::with_client(auth_client, config)` — the
  same transport we already use, just handed a different inner client.

This means OAuth is **a different client injected into the same transport**,
not a new transport, and rmcp's `AuthClient` already refreshes the token within
a session.

## Decision — client-credentials first
Ship OAuth 2.0 **client-credentials** (machine-to-machine, pre-registered
`client_id`/`client_secret`) as the only supported grant in this phase. Add
`"auth"` to the rmcp feature list, a new `oauth` kind to `auth.rs`'s
`from_config`, and an OAuth branch in `transport/http.rs`'s `HttpClient::connect`
that builds an `OAuthState`, runs `authenticate_client_credentials`, and wraps
the resulting `AuthorizationManager` in an `AuthClient` before connecting the
transport. `server.rs`, `verify.rs`, and the TS `PACT_MCP_AUTH` wiring need no
signature changes — an `oauth` `type` flows through the existing
`resolve_config` seam unchanged.

Tests are unit/mocked: a small local HTTP stub for the token endpoint (relying
on rmcp's legacy-metadata fallback so no `/.well-known/*` responses are needed),
asserting the transport sends `Authorization: Bearer …` on the MCP request. The
full fixture-authorization-server end-to-end (real discovery + protected MCP
server) is **deferred** to a follow-up.

## Why dynamic client registration is off the CI path
rmcp's `AuthorizationManager::register_client` (RFC 7591) hardcodes
`grant_types: ["authorization_code", "refresh_token"]`,
`token_endpoint_auth_method: "none"`, `response_types: ["code"]` — that is the
**interactive** authorization-code flow, requiring a browser redirect and human
consent, which cannot run unattended in CI. Client-credentials (SEP-1046) uses a
pre-registered `client_id`/`client_secret`; there is no registration step at
verify time. "Dynamic client registration" and "runs headless in CI" are
mutually exclusive with rmcp as shipped, so this phase does **not** implement
DCR. The `AuthProvider` seam and rmcp wiring added here make an interactive-DCR
path (consumer/agent side, or a future attended provider verify) a purely
additive follow-up — it reuses the same `OAuthState`/`AuthClient` plumbing with
a different terminal grant.

## `ResolvedAuth` shape
Today `ResolvedAuth { auth_header, custom_headers }` is a resolved *static*
header set, and `AuthProvider::resolve()` is sync. OAuth cannot resolve to a
static header synchronously — it needs an async token exchange against the live
server, and the token must be refreshable by the transport (rmcp's `AuthClient`
owns that). So the token is **not** pre-resolved into a header; it is carried as
config and exchanged inside the transport, where we have the base URL and an
async context:

```rust
pub struct ResolvedAuth {
    pub auth_header: Option<String>,
    pub custom_headers: Vec<(String, String)>,
    pub oauth: Option<OAuthClientCredentials>, // carried, not resolved to a header
}

pub struct OAuthClientCredentials {
    pub client_id: String,
    pub client_secret: String,    // ${ENV}-interpolated at resolve() time
    pub scopes: Vec<String>,
    pub resource: Option<String>, // defaults to the server base URL if omitted
}
```

`resolve()` stays sync — it interpolates `${ENV}` on `client_id`/`client_secret`
(reusing `interpolate_env`) and populates `oauth: Some(...)`. `apiKey`/`headers`/
`bearer` are unchanged. `resource` is mandatory to rmcp's token exchange (it
errors without it); we default it to the MCP server base URL when the user
omits it.

## Consequences
- **No change** to the matching engine, mock server, stdio transport,
  provider-states, consumer DSL, or the persisted-secret invariant — OAuth
  creds are config-only, same as today's `${ENV}` secrets, and the
  `secrets_never_land_in_the_persisted_pact_fragment` test is extended to cover
  the `oauth` kind rather than needing a new mechanism.
- The `auth` cargo feature adds the `oauth2` crate and (if not already present)
  a `reqwest` client; the transitive dependency footprint is small since
  `reqwest` is already pulled by the streamable-HTTP-client-reqwest feature.
- **Deferred** (follow-up issues, not this phase): the fixture authorization
  server + protected MCP fixture end-to-end; interactive authorization-code +
  RFC 7591 dynamic registration; `private_key_jwt` client-credentials
  (`auth-client-credentials-jwt` feature); cross-spawn token caching (rmcp's
  `AuthClient` already refreshes within a session).
