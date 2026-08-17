//! Phase 2 — HTTP provider verification against a REAL Streamable HTTP MCP
//! server (Node @modelcontextprotocol/sdk StreamableHTTPServerTransport):
//! open, bearer, apiKey, custom headers, and a negative (missing/invalid auth).

use pact_mcp_plugin::auth::resolve_config;
use pact_mcp_plugin::mcp::model::{McpInteraction, Operation};
use pact_mcp_plugin::verify::verify_interaction_http;
use serde_json::json;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Spawn the HTTP fixture server with the given env, return (child, base URL).
fn spawn_http_server(env: &[(&str, &str)]) -> (Child, String) {
    let server = repo_root().join("examples/fixtures/weather-http-server.mjs");
    let mut cmd = Command::new("node");
    cmd.arg(&server).stdout(Stdio::piped()).stderr(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn http fixture server");
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read port line");
    let parsed: serde_json::Value = serde_json::from_str(line.trim()).expect("port json");
    let port = parsed["port"].as_u64().expect("port");
    (child, format!("http://127.0.0.1:{port}/"))
}

fn weather_interaction() -> McpInteraction {
    McpInteraction::new(
        Operation::ToolsCall,
        json!({ "name": "get_weather", "arguments": { "city": "Melbourne" } }),
        json!({ "content": [ { "type": "text", "text": "Sunny, 22C" } ], "isError": false }),
    )
}

#[tokio::test]
async fn verifies_over_http_against_an_open_server() {
    let (mut child, url) = spawn_http_server(&[]);
    let auth = resolve_config(None).unwrap();
    let result = verify_interaction_http(&weather_interaction(), &url, &auth, None).await;
    let _ = child.kill();
    let result = result.expect("verification should run");
    assert!(result.is_match(), "mismatches: {:?}", result.mismatches);
}

#[tokio::test]
async fn verifies_over_http_with_bearer_auth() {
    std::env::set_var("PACT_MCP_HTTP_BEARER", "s3cret-bearer");
    let (mut child, url) = spawn_http_server(&[("REQUIRE_BEARER", "s3cret-bearer")]);
    let auth = resolve_config(Some(&json!({ "type": "bearer", "token": "${PACT_MCP_HTTP_BEARER}" }))).unwrap();
    let result = verify_interaction_http(&weather_interaction(), &url, &auth, None).await;
    let _ = child.kill();
    assert!(result.expect("ran").is_match());
}

#[tokio::test]
async fn verifies_over_http_with_api_key_auth() {
    let (mut child, url) = spawn_http_server(&[
        ("REQUIRE_API_KEY_HEADER", "X-API-Key"),
        ("REQUIRE_API_KEY_VALUE", "key-123"),
    ]);
    let auth = resolve_config(Some(&json!({ "type": "apiKey", "header": "X-API-Key", "value": "key-123" }))).unwrap();
    let result = verify_interaction_http(&weather_interaction(), &url, &auth, None).await;
    let _ = child.kill();
    assert!(result.expect("ran").is_match());
}

#[tokio::test]
async fn verifies_over_http_with_custom_headers_auth() {
    let (mut child, url) = spawn_http_server(&[
        ("REQUIRE_API_KEY_HEADER", "X-Custom"),
        ("REQUIRE_API_KEY_VALUE", "abc"),
    ]);
    let auth = resolve_config(Some(&json!({ "type": "headers", "headers": { "X-Custom": "abc" } }))).unwrap();
    let result = verify_interaction_http(&weather_interaction(), &url, &auth, None).await;
    let _ = child.kill();
    assert!(result.expect("ran").is_match());
}

#[tokio::test]
async fn auth_protected_server_genuinely_rejects_missing_auth() {
    // The server requires a bearer; we send NONE -> the handshake must fail.
    let (mut child, url) = spawn_http_server(&[("REQUIRE_BEARER", "s3cret-bearer")]);
    let no_auth = resolve_config(None).unwrap();
    let result = verify_interaction_http(&weather_interaction(), &url, &no_auth, None).await;
    let _ = child.kill();
    assert!(result.is_err(), "expected a transport/handshake failure without auth, got {result:?}");
}

#[tokio::test]
async fn auth_protected_server_rejects_wrong_token() {
    let (mut child, url) = spawn_http_server(&[("REQUIRE_BEARER", "the-right-token")]);
    let wrong = resolve_config(Some(&json!({ "type": "bearer", "token": "the-wrong-token" }))).unwrap();
    let result = verify_interaction_http(&weather_interaction(), &url, &wrong, None).await;
    let _ = child.kill();
    assert!(result.is_err(), "expected rejection with a wrong token, got {result:?}");
}

// --- OAuth2 client-credentials (ADR 0011, T3) --------------------------------
//
// Unit/mocked only, per the plan: the fixture server's `/token` route is a
// stub (returns a fixed access_token, doesn't validate client_id/secret or
// discovery metadata — rmcp falls back to legacy `<origin>/token` derivation
// when no `/.well-known/*` metadata is published, which this stub relies on).
// The full fixture-authorization-server end-to-end is deferred.

#[tokio::test]
async fn verifies_over_http_with_oauth_client_credentials() {
    let (mut child, url) = spawn_http_server(&[
        ("REQUIRE_BEARER", "mock-exchanged-access-token"),
        ("OAUTH_ACCESS_TOKEN", "mock-exchanged-access-token"),
    ]);
    let auth = resolve_config(Some(&json!({
        "type": "oauth",
        "grant": "client_credentials",
        "clientId": "test-client",
        "clientSecret": "test-secret",
        "scopes": ["mcp:verify"]
    })))
    .unwrap();
    let result = verify_interaction_http(&weather_interaction(), &url, &auth, None).await;
    let _ = child.kill();
    assert!(result.expect("ran").is_match());
}

#[tokio::test]
async fn oauth_token_exchange_failure_surfaces_as_a_transport_error() {
    // No OAUTH_ACCESS_TOKEN env -> the fixture's mocked /token route is never
    // registered, so the (legacy-fallback) token exchange 404s.
    let (mut child, url) = spawn_http_server(&[]);
    let auth = resolve_config(Some(&json!({
        "type": "oauth",
        "clientId": "test-client",
        "clientSecret": "test-secret"
    })))
    .unwrap();
    let result = verify_interaction_http(&weather_interaction(), &url, &auth, None).await;
    let _ = child.kill();
    assert!(result.is_err(), "expected a transport error when the token exchange fails, got {result:?}");
}

#[tokio::test]
async fn oauth_resource_defaults_to_the_server_base_url_when_omitted() {
    // No explicit `resource` in the config — connect_oauth must default it to
    // the MCP server's own base URL (rmcp errors without a resource param).
    let (mut child, url) = spawn_http_server(&[
        ("REQUIRE_BEARER", "another-mock-token"),
        ("OAUTH_ACCESS_TOKEN", "another-mock-token"),
    ]);
    let auth = resolve_config(Some(&json!({
        "type": "oauth",
        "clientId": "test-client",
        "clientSecret": "test-secret"
    })))
    .unwrap();
    assert_eq!(auth.oauth.as_ref().unwrap().resource, None);
    let result = verify_interaction_http(&weather_interaction(), &url, &auth, None).await;
    let _ = child.kill();
    assert!(result.expect("ran").is_match());
}
