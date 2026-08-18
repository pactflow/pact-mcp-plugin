//! ADR 0008 — drive `VerifyInteraction` over real gRPC exactly the way the
//! STANDARD pact verifier does (pact-reference `pact_verifier` 1.3.5 via
//! pact-plugin-driver 0.7.5): the interaction is addressed by its `key` (or the
//! driver's calculated hash — we require adapter-stamped keys, see ADR 0008),
//! and `config` carries only `{host, port?, providerState}`. No `command` —
//! stdio spawn config arrives via `PACT_MCP_SERVER_COMMAND`/`_ARGS` env vars on
//! the plugin process (inherited from the verifier).

use serde::Deserialize;
use serde_json::json;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child as StdChild, Command as StdCommand, Stdio as StdStdio};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader as TokioBufReader};
use tokio::process::{Child, Command};

use pact_mcp_plugin::proto::pact_plugin_client::PactPluginClient;
use pact_mcp_plugin::proto::{verify_interaction_response, VerifyInteractionRequest};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[derive(Deserialize)]
struct Handshake {
    port: u16,
    #[serde(rename = "serverKey")]
    server_key: String,
}

/// Spawn the plugin binary with extra env vars, mirroring how the pact plugin
/// driver launches it as a subprocess of the verifier (env is inherited).
async fn spawn_plugin(env: &[(&str, &str)]) -> (Child, Handshake) {
    let exe = env!("CARGO_BIN_EXE_pact-mcp-plugin");
    let mut cmd = Command::new(exe);
    cmd.stdout(StdStdio::piped()).stderr(StdStdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("failed to spawn pact-mcp-plugin binary");

    let stdout = child.stdout.take().expect("piped stdout");
    let mut lines = TokioBufReader::new(stdout).lines();
    let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
        .await
        .expect("timed out waiting for startup handshake line")
        .expect("reading stdout")
        .expect("plugin produced no stdout line");

    let handshake: Handshake = serde_json::from_str(&line)
        .unwrap_or_else(|e| panic!("startup line was not valid JSON handshake: {line:?}: {e}"));

    (child, handshake)
}

type Client = PactPluginClient<
    tonic::service::interceptor::InterceptedService<
        tonic::transport::Channel,
        Box<dyn FnMut(tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> + Send>,
    >,
>;

async fn connect(handshake: &Handshake) -> Client {
    let endpoint = format!("http://127.0.0.1:{}", handshake.port);
    let channel = tonic::transport::Endpoint::new(endpoint)
        .expect("valid endpoint")
        .connect()
        .await
        .expect("failed to connect to spawned plugin");
    let key = handshake.server_key.clone();
    let interceptor: Box<dyn FnMut(tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> + Send> =
        Box::new(move |mut req: tonic::Request<()>| {
            req.metadata_mut().insert("authorization", key.parse().unwrap());
            Ok(req)
        });
    PactPluginClient::with_interceptor(channel, interceptor)
}

/// Spawn the HTTP fixture server, return (child, port).
fn spawn_http_fixture() -> (StdChild, u64) {
    let server = repo_root().join("examples/fixtures/weather-http-server.mjs");
    let mut child = StdCommand::new("node")
        .arg(&server)
        .stdout(StdStdio::piped())
        .stderr(StdStdio::null())
        .spawn()
        .expect("spawn http fixture server");
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read port line");
    let parsed: serde_json::Value = serde_json::from_str(line.trim()).expect("port json");
    (child, parsed["port"].as_u64().expect("port"))
}

/// A pact in the REAL emitted two-part sync-message shape (ADR 0004), with the
/// adapter-stamped top-level `transport` + `key` (ADR 0008). The expected text
/// is a placeholder that only passes when the response matching rules from the
/// pact are applied (a type matcher on `$.content[0].text`) — this pins the
/// fix for the gRPC verify path previously dropping matching rules.
fn pact_json(transport: &str) -> String {
    json!({
        "consumer": { "name": "weather-agent" },
        "provider": { "name": "weather-mcp" },
        "interactions": [
            {
                "description": "a request for the Melbourne weather",
                "key": "adapter-stamped-key-1",
                "transport": transport,
                "pending": false,
                "pluginConfiguration": {
                    "mcp": { "operation": "tools/call", "server": { "transport": if transport == "mcp-http" { "http" } else { "stdio" } } }
                },
                "request": {
                    "contents": {
                        "content": { "name": "get_weather", "arguments": { "city": "Melbourne" } },
                        "contentType": "application/mcp+json",
                        "contentTypeHint": "TEXT",
                        "encoded": false
                    }
                },
                "response": [
                    {
                        "contents": {
                            "content": {
                                "content": [ { "type": "text", "text": "SOME WEATHER" } ],
                                "isError": false
                            },
                            "contentType": "application/mcp+json",
                            "contentTypeHint": "TEXT",
                            "encoded": false
                        },
                        "matchingRules": {
                            "body": {
                                "$.content[0].text": { "combine": "AND", "matchers": [ { "match": "type" } ] }
                            }
                        }
                    }
                ],
                "type": "Synchronous/Messages"
            }
        ],
        "metadata": { "pactSpecification": { "version": "4.0" } }
    })
    .to_string()
}

fn verify_request(pact: String, key: &str, config: serde_json::Value) -> VerifyInteractionRequest {
    let config_struct = match config {
        serde_json::Value::Object(map) => {
            let mut fields = std::collections::BTreeMap::new();
            for (k, v) in map {
                fields.insert(k, json_to_prost(&v));
            }
            Some(prost_types::Struct { fields })
        }
        _ => None,
    };
    VerifyInteractionRequest {
        interaction_data: None,
        config: config_struct,
        pact,
        interaction_key: key.to_string(),
    }
}

fn json_to_prost(v: &serde_json::Value) -> prost_types::Value {
    use prost_types::value::Kind;
    let kind = match v {
        serde_json::Value::Null => Kind::NullValue(0),
        serde_json::Value::Bool(b) => Kind::BoolValue(*b),
        serde_json::Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Kind::StringValue(s.clone()),
        serde_json::Value::Array(items) => Kind::ListValue(prost_types::ListValue {
            values: items.iter().map(json_to_prost).collect(),
        }),
        serde_json::Value::Object(map) => {
            let mut fields = std::collections::BTreeMap::new();
            for (k, v) in map {
                fields.insert(k.clone(), json_to_prost(v));
            }
            Kind::StructValue(prost_types::Struct { fields })
        }
    };
    prost_types::Value { kind: Some(kind) }
}

fn expect_success(resp: &pact_mcp_plugin::proto::VerifyInteractionResponse) -> bool {
    match resp.response.as_ref().expect("verify response") {
        verify_interaction_response::Response::Result(r) => r.success,
        verify_interaction_response::Response::Error(e) => panic!("verify returned error: {e}"),
    }
}

#[tokio::test]
async fn http_interaction_verifies_with_only_host_and_port_config_like_the_standard_verifier() {
    let (mut fixture, port) = spawn_http_fixture();
    let (mut plugin, handshake) = spawn_plugin(&[]).await;
    let mut client = connect(&handshake).await;

    let req = verify_request(
        pact_json("mcp-http"),
        "adapter-stamped-key-1",
        json!({ "host": "127.0.0.1", "port": port, "providerState": {} }),
    );
    let resp = client.verify_interaction(req).await.expect("verify call").into_inner();
    let ok = expect_success(&resp);

    let _ = plugin.kill().await;
    let _ = fixture.kill();
    assert!(ok, "expected HTTP verification to pass with host/port-only config: {resp:?}");
}

/// ADR 0011 / plan T4: an `oauth` `PACT_MCP_AUTH` flows unchanged through
/// server.rs's existing env-var -> `resolve_config` -> `verify_interaction_http`
/// seam — no signature changes were needed for OAuth. Reuses the same mocked
/// token-endpoint fixture as the T3 transport-level tests.
#[tokio::test]
async fn http_interaction_verifies_with_oauth_client_credentials_via_pact_mcp_auth_env() {
    let (mut fixture, port) = {
        let server = repo_root().join("examples/fixtures/weather-http-server.mjs");
        let mut cmd = StdCommand::new("node");
        cmd.arg(&server)
            .env("REQUIRE_BEARER", "grpc-mock-access-token")
            .env("OAUTH_ACCESS_TOKEN", "grpc-mock-access-token")
            .stdout(StdStdio::piped())
            .stderr(StdStdio::null());
        let mut child = cmd.spawn().expect("spawn http fixture server");
        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        reader.read_line(&mut line).expect("read port line");
        let parsed: serde_json::Value = serde_json::from_str(line.trim()).expect("port json");
        (child, parsed["port"].as_u64().expect("port"))
    };

    let auth_json = json!({
        "type": "oauth",
        "grant": "client_credentials",
        "clientId": "test-client",
        "clientSecret": "test-secret",
        "scopes": ["mcp:verify"]
    })
    .to_string();
    let (mut plugin, handshake) = spawn_plugin(&[("PACT_MCP_AUTH", &auth_json)]).await;
    let mut client = connect(&handshake).await;

    let req = verify_request(
        pact_json("mcp-http"),
        "adapter-stamped-key-1",
        json!({ "host": "127.0.0.1", "port": port, "providerState": {} }),
    );
    let resp = client.verify_interaction(req).await.expect("verify call").into_inner();
    let ok = expect_success(&resp);

    let _ = plugin.kill().await;
    let _ = fixture.kill();
    assert!(ok, "expected HTTP verification to pass via oauth PACT_MCP_AUTH: {resp:?}");
}

/// A failed token exchange (no mocked token endpoint on the fixture here)
/// must surface as a clear error, mirroring how a 401 surfaces today —
/// never a panic or a silently-passed verification.
#[tokio::test]
async fn oauth_token_exchange_failure_surfaces_as_a_clear_verify_error() {
    let (mut fixture, port) = spawn_http_fixture();

    let auth_json = json!({
        "type": "oauth",
        "clientId": "test-client",
        "clientSecret": "test-secret"
    })
    .to_string();
    let (mut plugin, handshake) = spawn_plugin(&[("PACT_MCP_AUTH", &auth_json)]).await;
    let mut client = connect(&handshake).await;

    let req = verify_request(
        pact_json("mcp-http"),
        "adapter-stamped-key-1",
        json!({ "host": "127.0.0.1", "port": port, "providerState": {} }),
    );
    let result = client.verify_interaction(req).await;

    let message = match result {
        Ok(resp) => match resp.into_inner().response {
            Some(verify_interaction_response::Response::Error(e)) => e,
            other => panic!("expected an error response, got: {other:?}"),
        },
        Err(status) => status.message().to_string(),
    };
    let _ = plugin.kill().await;
    let _ = fixture.kill();
    assert!(!message.is_empty(), "expected a non-empty error message for a failed oauth exchange");
}

#[tokio::test]
async fn stdio_interaction_verifies_with_spawn_config_from_env_vars() {
    let server = repo_root().join("examples/fixtures/weather-server.mjs");
    let (mut plugin, handshake) = spawn_plugin(&[
        ("PACT_MCP_SERVER_COMMAND", "node"),
        ("PACT_MCP_SERVER_ARGS", server.to_str().unwrap()),
    ])
    .await;
    let mut client = connect(&handshake).await;

    // Addressed by description (fallback), config carries no spawn info.
    let req = verify_request(
        pact_json("mcp-stdio"),
        "a request for the Melbourne weather",
        json!({ "host": "127.0.0.1", "providerState": {} }),
    );
    let resp = client.verify_interaction(req).await.expect("verify call").into_inner();
    let ok = expect_success(&resp);

    let _ = plugin.kill().await;
    assert!(ok, "expected stdio verification to pass with env-var spawn config: {resp:?}");
}

#[tokio::test]
async fn stdio_interaction_without_any_spawn_config_fails_with_a_clear_error() {
    let (mut plugin, handshake) = spawn_plugin(&[]).await;
    let mut client = connect(&handshake).await;

    let req = verify_request(
        pact_json("mcp-stdio"),
        "adapter-stamped-key-1",
        json!({ "host": "127.0.0.1", "providerState": {} }),
    );
    let result = client.verify_interaction(req).await;

    let message = match result {
        Ok(resp) => match resp.into_inner().response {
            Some(verify_interaction_response::Response::Error(e)) => e,
            other => panic!("expected an error response, got: {other:?}"),
        },
        Err(status) => status.message().to_string(),
    };
    let _ = plugin.kill().await;
    assert!(
        message.contains("PACT_MCP_SERVER_COMMAND"),
        "error should name the env-var mechanism, got: {message}"
    );
}
