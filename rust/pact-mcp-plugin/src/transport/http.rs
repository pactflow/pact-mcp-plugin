//! Streamable HTTP transport (Phase 2). Connects to a real HTTP MCP server,
//! injecting resolved auth headers on every request (incl. `initialize`).
//! Uses rmcp's Streamable HTTP client (reqwest) — see docs/decisions/0007.
//! The transport handles `Mcp-Session-Id`, `Accept: application/json,
//! text/event-stream`, and both JSON-body and SSE response modes.

use super::{perform_on_service, McpResponse, TransportError};
use crate::auth::ResolvedAuth;
use crate::mcp::model::Operation;
use http::{HeaderName, HeaderValue};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::auth::{AuthClient, ClientCredentialsConfig, OAuthState};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use serde_json::Value;
use std::collections::HashMap;

/// A connected Streamable HTTP MCP client.
pub struct HttpClient {
    service: RunningService<RoleClient, ()>,
}

/// rustls (pulled in transitively via reqwest, and directly for the OAuth
/// client-credentials exchange — ADR 0011) does not auto-install a default
/// crypto provider even when a single backend is compiled in; building a
/// `reqwest::Client` panics until one is installed. Do it once, lazily, right
/// before the first client is built — cheap and idempotent.
fn ensure_default_crypto_provider() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        // `install_default` fails only if a provider is already installed
        // (e.g. by another crate in the process); either way one is present
        // afterwards, so a failed install is not an error for us.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

impl HttpClient {
    /// Connect to `url` and complete the initialize handshake, injecting the
    /// resolved auth on every request: bearer -> auth_header, apiKey/headers
    /// -> custom_headers (static, today's path), or oauth -> a client-
    /// credentials token exchange whose access token rmcp's `AuthClient`
    /// injects (and refreshes) on every request (ADR 0011).
    pub async fn connect(url: &str, auth: &ResolvedAuth) -> Result<Self, TransportError> {
        ensure_default_crypto_provider();

        if let Some(oauth) = &auth.oauth {
            return Self::connect_oauth(url, oauth).await;
        }

        let mut custom_headers: HashMap<HeaderName, HeaderValue> = HashMap::new();
        for (name, value) in &auth.custom_headers {
            let hn = HeaderName::from_bytes(name.as_bytes())
                .map_err(|e| TransportError::Connect(format!("invalid header name `{name}`: {e}")))?;
            let hv = HeaderValue::from_str(value)
                .map_err(|e| TransportError::Connect(format!("invalid header value for `{name}`: {e}")))?;
            custom_headers.insert(hn, hv);
        }

        let mut config = StreamableHttpClientTransportConfig::with_uri(url).custom_headers(custom_headers);
        if let Some(token) = &auth.auth_header {
            config = config.auth_header(token.clone());
        }

        let transport = StreamableHttpClientTransport::from_config(config);
        let service = ()
            .serve(transport)
            .await
            .map_err(|e| TransportError::Handshake(e.to_string()))?;

        Ok(Self { service })
    }

    /// OAuth 2.0 client-credentials connect path (SEP-1046, ADR 0011): run
    /// the whole discover -> configure -> exchange flow via rmcp's
    /// `OAuthState`, then hand the resulting `AuthorizationManager` to
    /// `AuthClient` — a different inner client for the SAME
    /// `StreamableHttpClientTransport`, not a new transport. `AuthClient`
    /// injects (and refreshes) `Authorization: Bearer <token>` on every
    /// request itself, so no static `auth_header`/`custom_headers` are set on
    /// the transport config here.
    async fn connect_oauth(url: &str, oauth: &crate::auth::OAuthClientCredentials) -> Result<Self, TransportError> {
        // `resource` is REQUIRED by the MCP auth spec / rmcp's exchange —
        // default to the MCP server's own base URL when the config omits it.
        let resource = oauth.resource.clone().unwrap_or_else(|| url.to_string());

        let mut state = OAuthState::new(url, Some(reqwest::Client::new()))
            .await
            .map_err(|e| TransportError::Connect(format!("oauth: failed to initialize OAuth state: {e}")))?;

        state
            .authenticate_client_credentials(ClientCredentialsConfig::ClientSecret {
                client_id: oauth.client_id.clone(),
                client_secret: oauth.client_secret.clone(),
                scopes: oauth.scopes.clone(),
                resource: Some(resource),
            })
            .await
            .map_err(|e| TransportError::Handshake(format!("oauth client-credentials exchange failed: {e}")))?;

        let manager = state.into_authorization_manager().ok_or_else(|| {
            TransportError::Handshake("oauth: state was not Authorized after a successful exchange".to_string())
        })?;

        let auth_client = AuthClient::new(reqwest::Client::new(), manager);
        let config = StreamableHttpClientTransportConfig::with_uri(url);
        let transport = StreamableHttpClientTransport::with_client(auth_client, config);
        let service = ()
            .serve(transport)
            .await
            .map_err(|e| TransportError::Handshake(e.to_string()))?;

        Ok(Self { service })
    }

    pub async fn perform(&self, operation: Operation, request: &Value) -> Result<McpResponse, TransportError> {
        perform_on_service(&self.service, operation, request).await
    }

    pub async fn close(self) -> Result<(), TransportError> {
        self.service
            .cancel()
            .await
            .map_err(|e| TransportError::Request(e.to_string()))?;
        Ok(())
    }
}
