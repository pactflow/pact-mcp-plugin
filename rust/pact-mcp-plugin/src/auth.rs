//! Auth for HTTP transports (Phase 2, +OAuth2 Phase 4). See ADR 0007, ADR 0011.
//!
//! `AuthProvider` resolves an auth config into headers injected on EVERY HTTP
//! request (including `initialize`). Four concrete kinds: `bearer`, `apiKey`,
//! `headers`, `oauth`. All values support `${ENV}` interpolation resolved from
//! the process environment at verification time. Secrets are NEVER persisted
//! to the pact — auth lives only on the verification/transport config.
//!
//! `oauth` (client-credentials, SEP-1046, ADR 0011) is different from the
//! other three: it can't resolve to a static header synchronously — it needs
//! an async token exchange against the live server, and the token must be
//! refreshable by the transport. So `resolve()` stays sync and only carries
//! the (env-interpolated) client-credentials config on `ResolvedAuth.oauth`;
//! the actual token exchange happens in the transport (`transport::http`),
//! where we have the base URL and an async context.

use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("environment variable `{0}` referenced by auth config is not set")]
    MissingEnv(String),
    #[error("invalid auth config: {0}")]
    Invalid(String),
}

/// The resolved headers to inject on HTTP requests.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedAuth {
    /// A bearer token (WITHOUT the `Bearer ` prefix); maps to rmcp's `auth_header`.
    pub auth_header: Option<String>,
    /// Arbitrary custom headers (name, value).
    pub custom_headers: Vec<(String, String)>,
    /// OAuth2 client-credentials config (ADR 0011), carried — not resolved to
    /// a header. `Some` only for `type: "oauth"`; the transport performs the
    /// token exchange.
    pub oauth: Option<OAuthClientCredentials>,
}

/// OAuth 2.0 client-credentials config (SEP-1046), resolved (`${ENV}`
/// interpolated) but not yet exchanged for a token. See ADR 0011.
#[derive(Debug, Clone, PartialEq)]
pub struct OAuthClientCredentials {
    pub client_id: String,
    pub client_secret: String,
    pub scopes: Vec<String>,
    /// The RFC 8707 `resource` indicator. Mandatory to rmcp's token exchange;
    /// when the config omits it, the transport defaults it to the MCP server
    /// base URL (rmcp errors on `None` at exchange time).
    pub resource: Option<String>,
}

/// Resolves auth material (with `${ENV}` interpolation) into headers.
pub trait AuthProvider: std::fmt::Debug {
    fn resolve(&self) -> Result<ResolvedAuth, AuthError>;
}

/// `Authorization: Bearer <token>`.
#[derive(Debug, Clone)]
pub struct BearerAuth {
    pub token: String,
}
impl AuthProvider for BearerAuth {
    fn resolve(&self) -> Result<ResolvedAuth, AuthError> {
        Ok(ResolvedAuth {
            auth_header: Some(interpolate_env(&self.token)?),
            custom_headers: vec![],
            oauth: None,
        })
    }
}

/// A single API-key header, e.g. `X-API-Key: <value>`.
#[derive(Debug, Clone)]
pub struct ApiKeyAuth {
    pub header: String,
    pub value: String,
}
impl AuthProvider for ApiKeyAuth {
    fn resolve(&self) -> Result<ResolvedAuth, AuthError> {
        Ok(ResolvedAuth {
            auth_header: None,
            custom_headers: vec![(self.header.clone(), interpolate_env(&self.value)?)],
            oauth: None,
        })
    }
}

/// Arbitrary custom headers.
#[derive(Debug, Clone)]
pub struct HeadersAuth {
    pub headers: Vec<(String, String)>,
}
impl AuthProvider for HeadersAuth {
    fn resolve(&self) -> Result<ResolvedAuth, AuthError> {
        let mut out = Vec::with_capacity(self.headers.len());
        for (k, v) in &self.headers {
            out.push((k.clone(), interpolate_env(v)?));
        }
        Ok(ResolvedAuth { auth_header: None, custom_headers: out, oauth: None })
    }
}

/// OAuth 2.0 client-credentials (SEP-1046, ADR 0011). Only `grant:
/// "client_credentials"` is supported in this phase — see ADR 0011 for why
/// dynamic client registration / interactive flows are out of scope.
#[derive(Debug, Clone)]
pub struct OAuthAuth {
    pub client_id: String,
    pub client_secret: String,
    pub scopes: Vec<String>,
    pub resource: Option<String>,
}
impl AuthProvider for OAuthAuth {
    fn resolve(&self) -> Result<ResolvedAuth, AuthError> {
        Ok(ResolvedAuth {
            auth_header: None,
            custom_headers: vec![],
            oauth: Some(OAuthClientCredentials {
                client_id: interpolate_env(&self.client_id)?,
                client_secret: interpolate_env(&self.client_secret)?,
                scopes: self.scopes.clone(),
                resource: self.resource.clone(),
            }),
        })
    }
}

/// Parse an auth config JSON object into an `AuthProvider`.
///
/// ```jsonc
/// { "type": "bearer",  "token": "${MCP_TOKEN}" }
/// { "type": "apiKey",  "header": "X-API-Key", "value": "${MCP_KEY}" }
/// { "type": "headers", "headers": { "X-A": "1", "X-B": "${B}" } }
/// { "type": "oauth", "grant": "client_credentials",
///   "clientId": "${MCP_OAUTH_CLIENT_ID}", "clientSecret": "${MCP_OAUTH_CLIENT_SECRET}",
///   "scopes": ["mcp:verify"], "resource": "https://mcp.example.com/mcp" }
/// ```
pub fn from_config(config: &Value) -> Result<Box<dyn AuthProvider>, AuthError> {
    let kind = config
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| AuthError::Invalid("missing `type`".to_string()))?;
    match kind {
        "bearer" => {
            let token = config
                .get("token")
                .and_then(Value::as_str)
                .ok_or_else(|| AuthError::Invalid("bearer auth requires `token`".to_string()))?;
            Ok(Box::new(BearerAuth { token: token.to_string() }))
        }
        "apiKey" => {
            let header = config
                .get("header")
                .and_then(Value::as_str)
                .ok_or_else(|| AuthError::Invalid("apiKey auth requires `header`".to_string()))?;
            let value = config
                .get("value")
                .and_then(Value::as_str)
                .ok_or_else(|| AuthError::Invalid("apiKey auth requires `value`".to_string()))?;
            Ok(Box::new(ApiKeyAuth { header: header.to_string(), value: value.to_string() }))
        }
        "headers" => {
            let map = config
                .get("headers")
                .and_then(Value::as_object)
                .ok_or_else(|| AuthError::Invalid("headers auth requires `headers` object".to_string()))?;
            let headers = map
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect();
            Ok(Box::new(HeadersAuth { headers }))
        }
        "oauth" => {
            let grant = config.get("grant").and_then(Value::as_str).unwrap_or("client_credentials");
            if grant != "client_credentials" {
                return Err(AuthError::Invalid(format!(
                    "unsupported oauth grant `{grant}` — only `client_credentials` is supported (see ADR 0011)"
                )));
            }
            let client_id = config
                .get("clientId")
                .and_then(Value::as_str)
                .ok_or_else(|| AuthError::Invalid("oauth auth requires `clientId`".to_string()))?;
            let client_secret = config
                .get("clientSecret")
                .and_then(Value::as_str)
                .ok_or_else(|| AuthError::Invalid("oauth auth requires `clientSecret`".to_string()))?;
            let scopes = config
                .get("scopes")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            let resource = config.get("resource").and_then(Value::as_str).map(str::to_string);
            Ok(Box::new(OAuthAuth {
                client_id: client_id.to_string(),
                client_secret: client_secret.to_string(),
                scopes,
                resource,
            }))
        }
        other => Err(AuthError::Invalid(format!("unknown auth type `{other}`"))),
    }
}

/// Resolve an optional auth config to headers. `None`/`Null` config = no auth.
pub fn resolve_config(config: Option<&Value>) -> Result<ResolvedAuth, AuthError> {
    match config {
        None => Ok(ResolvedAuth::default()),
        Some(v) if v.is_null() => Ok(ResolvedAuth::default()),
        Some(v) => from_config(v)?.resolve(),
    }
}

/// Interpolate `${VAR}` occurrences from the process environment. A literal
/// value with no `${...}` is returned unchanged.
pub fn interpolate_env(input: &str) -> Result<String, AuthError> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| AuthError::Invalid(format!("unterminated ${{ in `{input}`")))?;
        let var = &after[..end];
        let val = std::env::var(var).map_err(|_| AuthError::MissingEnv(var.to_string()))?;
        out.push_str(&val);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_resolves_to_auth_header() {
        let a = from_config(&serde_json::json!({ "type": "bearer", "token": "abc123" })).unwrap();
        let r = a.resolve().unwrap();
        assert_eq!(r.auth_header.as_deref(), Some("abc123"));
        assert!(r.custom_headers.is_empty());
    }

    #[test]
    fn api_key_resolves_to_a_custom_header() {
        let a = from_config(&serde_json::json!({ "type": "apiKey", "header": "X-API-Key", "value": "sekret" })).unwrap();
        let r = a.resolve().unwrap();
        assert_eq!(r.auth_header, None);
        assert_eq!(r.custom_headers, vec![("X-API-Key".to_string(), "sekret".to_string())]);
    }

    #[test]
    fn headers_resolve_verbatim() {
        let a = from_config(&serde_json::json!({ "type": "headers", "headers": { "X-A": "1" } })).unwrap();
        let r = a.resolve().unwrap();
        assert_eq!(r.custom_headers, vec![("X-A".to_string(), "1".to_string())]);
    }

    #[test]
    fn env_interpolation_resolves_from_the_environment() {
        std::env::set_var("PACT_MCP_TEST_TOKEN", "resolved-secret");
        let a = from_config(&serde_json::json!({ "type": "bearer", "token": "${PACT_MCP_TEST_TOKEN}" })).unwrap();
        let r = a.resolve().unwrap();
        assert_eq!(r.auth_header.as_deref(), Some("resolved-secret"));
    }

    #[test]
    fn missing_env_is_an_error_not_a_silent_empty() {
        let a = from_config(&serde_json::json!({ "type": "bearer", "token": "${PACT_MCP_DEFINITELY_UNSET}" })).unwrap();
        assert!(matches!(a.resolve(), Err(AuthError::MissingEnv(_))));
    }

    #[test]
    fn interpolation_mixes_literal_and_env() {
        std::env::set_var("PACT_MCP_TEST_KEY", "XYZ");
        assert_eq!(interpolate_env("key-${PACT_MCP_TEST_KEY}-end").unwrap(), "key-XYZ-end");
    }

    #[test]
    fn no_auth_config_yields_empty() {
        assert_eq!(resolve_config(None).unwrap(), ResolvedAuth::default());
        assert_eq!(resolve_config(Some(&Value::Null)).unwrap(), ResolvedAuth::default());
    }

    /// Invariant: auth config / secrets never enter the persisted pact fragment.
    /// `config::configure_interaction` has no auth input, so a configured
    /// interaction's serialized body cannot contain the secret.
    #[test]
    fn secrets_never_land_in_the_persisted_pact_fragment() {
        std::env::set_var("PACT_MCP_SECRET", "TOPSECRET");
        let _auth = resolve_config(Some(&serde_json::json!({ "type": "bearer", "token": "${PACT_MCP_SECRET}" }))).unwrap();

        let configured = crate::config::configure_interaction(&serde_json::json!({
            "operation": "tools/call",
            "request": { "name": "get_weather", "arguments": { "city": "Melbourne" } },
            "response": { "content": [ { "type": "text", "text": "Sunny" } ], "isError": false }
        }))
        .unwrap();

        let request_json = String::from_utf8(configured.request.body_bytes).unwrap();
        let response_json = String::from_utf8(configured.response.body_bytes).unwrap();
        assert!(!request_json.contains("TOPSECRET"));
        assert!(!response_json.contains("TOPSECRET"));
        assert!(!serde_json::to_string(&configured.fragment).unwrap().contains("TOPSECRET"));
    }

    /// Same invariant as above, extended to the `oauth` kind (ADR 0011,
    /// T2): a resolved oauth client_secret must never reach the persisted
    /// pact fragment either.
    #[test]
    fn oauth_secrets_never_land_in_the_persisted_pact_fragment() {
        std::env::set_var("PACT_MCP_OAUTH_SECRET", "TOPSECRET_OAUTH");
        let auth = resolve_config(Some(&serde_json::json!({
            "type": "oauth",
            "grant": "client_credentials",
            "clientId": "my-client",
            "clientSecret": "${PACT_MCP_OAUTH_SECRET}",
            "scopes": ["mcp:verify"]
        })))
        .unwrap();
        assert_eq!(auth.oauth.as_ref().unwrap().client_secret, "TOPSECRET_OAUTH");

        let configured = crate::config::configure_interaction(&serde_json::json!({
            "operation": "tools/call",
            "request": { "name": "get_weather", "arguments": { "city": "Melbourne" } },
            "response": { "content": [ { "type": "text", "text": "Sunny" } ], "isError": false }
        }))
        .unwrap();

        let request_json = String::from_utf8(configured.request.body_bytes).unwrap();
        let response_json = String::from_utf8(configured.response.body_bytes).unwrap();
        assert!(!request_json.contains("TOPSECRET_OAUTH"));
        assert!(!response_json.contains("TOPSECRET_OAUTH"));
        assert!(!serde_json::to_string(&configured.fragment).unwrap().contains("TOPSECRET_OAUTH"));
    }

    #[test]
    fn oauth_parses_and_resolves_client_credentials() {
        let a = from_config(&serde_json::json!({
            "type": "oauth",
            "grant": "client_credentials",
            "clientId": "my-client",
            "clientSecret": "my-secret",
            "scopes": ["mcp:verify", "mcp:admin"],
            "resource": "https://mcp.example.com/mcp"
        }))
        .unwrap();
        let r = a.resolve().unwrap();
        assert_eq!(r.auth_header, None);
        assert!(r.custom_headers.is_empty());
        let oauth = r.oauth.expect("oauth config should be carried");
        assert_eq!(oauth.client_id, "my-client");
        assert_eq!(oauth.client_secret, "my-secret");
        assert_eq!(oauth.scopes, vec!["mcp:verify".to_string(), "mcp:admin".to_string()]);
        assert_eq!(oauth.resource.as_deref(), Some("https://mcp.example.com/mcp"));
    }

    #[test]
    fn oauth_grant_defaults_to_client_credentials_when_omitted() {
        let a = from_config(&serde_json::json!({
            "type": "oauth", "clientId": "c", "clientSecret": "s"
        }))
        .unwrap();
        let r = a.resolve().unwrap();
        assert!(r.oauth.is_some());
    }

    #[test]
    fn oauth_rejects_unsupported_grants() {
        let err = from_config(&serde_json::json!({
            "type": "oauth",
            "grant": "authorization_code",
            "clientId": "c",
            "clientSecret": "s"
        }))
        .unwrap_err();
        assert!(matches!(err, AuthError::Invalid(_)));
        assert!(err.to_string().contains("client_credentials"));
    }

    #[test]
    fn oauth_requires_client_id() {
        let err = from_config(&serde_json::json!({ "type": "oauth", "clientSecret": "s" })).unwrap_err();
        assert!(matches!(err, AuthError::Invalid(_)));
    }

    #[test]
    fn oauth_requires_client_secret() {
        let err = from_config(&serde_json::json!({ "type": "oauth", "clientId": "c" })).unwrap_err();
        assert!(matches!(err, AuthError::Invalid(_)));
    }

    #[test]
    fn oauth_env_interpolates_client_id_and_secret() {
        std::env::set_var("PACT_MCP_TEST_OAUTH_ID", "resolved-id");
        std::env::set_var("PACT_MCP_TEST_OAUTH_SECRET", "resolved-secret");
        let a = from_config(&serde_json::json!({
            "type": "oauth",
            "clientId": "${PACT_MCP_TEST_OAUTH_ID}",
            "clientSecret": "${PACT_MCP_TEST_OAUTH_SECRET}"
        }))
        .unwrap();
        let r = a.resolve().unwrap();
        let oauth = r.oauth.unwrap();
        assert_eq!(oauth.client_id, "resolved-id");
        assert_eq!(oauth.client_secret, "resolved-secret");
    }

    #[test]
    fn oauth_missing_env_is_an_error() {
        let a = from_config(&serde_json::json!({
            "type": "oauth",
            "clientId": "c",
            "clientSecret": "${PACT_MCP_OAUTH_DEFINITELY_UNSET}"
        }))
        .unwrap();
        assert!(matches!(a.resolve(), Err(AuthError::MissingEnv(_))));
    }
}
