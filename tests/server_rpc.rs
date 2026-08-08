mod common;

use common::{Client, assert_unauthorized, fixture};
use serde_json::{Value, json};
use std::time::Duration;
use yazi_claude_ide::server::Sidecar;

const TOKEN: &str = "rpc-test-token";

async fn sidecar() -> Sidecar {
    common::sidecar(TOKEN, vec![fixture("Cargo.toml")]).await
}

#[tokio::test]
async fn a5_server_binds_loopback_only() {
    let server = sidecar().await;
    assert_eq!(server.hostname(), "127.0.0.1");
    assert_ne!(server.port(), 0);
}

#[tokio::test]
async fn e1_missing_token_is_refused_with_401() {
    let server = sidecar().await;
    let error = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}", server.port()))
        .await
        .expect_err("missing token must not upgrade");
    assert_unauthorized(error);
}

#[tokio::test]
async fn e1_wrong_token_is_refused_with_401() {
    let server = sidecar().await;
    let error = Client::connect(&server, "wrong-token")
        .await
        .err()
        .expect("wrong token must not upgrade");
    assert_unauthorized(error);
}

#[tokio::test]
async fn e1_correct_token_connects() {
    let server = sidecar().await;
    Client::connect(&server, TOKEN)
        .await
        .expect("correct token should connect")
        .close();
}

/// Claude Code asks for the `mcp` subprotocol and hangs up on a `101` that does
/// not name it back, so this asserts on the handshake response rather than on
/// whether the socket opened — tungstenite opens it either way.
#[tokio::test]
async fn e6_requested_subprotocol_is_echoed_in_the_handshake() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let server = sidecar().await;
    let mut request = format!("ws://127.0.0.1:{}", server.port())
        .into_client_request()
        .expect("url should parse");
    request
        .headers_mut()
        .insert("x-claude-code-ide-authorization", TOKEN.parse().unwrap());
    request
        .headers_mut()
        .insert("sec-websocket-protocol", "mcp".parse().unwrap());

    let (_socket, response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("correct token should connect");

    assert_eq!(
        response
            .headers()
            .get("sec-websocket-protocol")
            .map(|value| value.as_bytes()),
        Some("mcp".as_bytes()),
    );
}

/// The header is optional. A client that asks for no subprotocol MUST NOT be
/// told it got one.
#[tokio::test]
async fn e6_no_subprotocol_is_offered_when_none_was_requested() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let server = sidecar().await;
    let mut request = format!("ws://127.0.0.1:{}", server.port())
        .into_client_request()
        .expect("url should parse");
    request
        .headers_mut()
        .insert("x-claude-code-ide-authorization", TOKEN.parse().unwrap());

    let (_socket, response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("correct token should connect");

    assert!(response.headers().get("sec-websocket-protocol").is_none());
}

#[tokio::test]
async fn e2_unknown_method_returns_minus_32601() {
    let server = sidecar().await;
    let mut client = Client::connect(&server, TOKEN).await.expect("connect");
    let response = client.call(1, "missing/method", None).await;
    assert_eq!(response["error"]["code"], -32601);
    assert_eq!(
        response["error"]["message"],
        "Method not found: missing/method"
    );
}

#[tokio::test]
async fn e2_unimplemented_tool_returns_minus_32601() {
    let server = sidecar().await;
    let mut client = Client::connect(&server, TOKEN).await.expect("connect");
    let response = client
        .call(2, "tools/call", Some(json!({"name": "missingTool"})))
        .await;
    assert_eq!(response["error"]["code"], -32601);
    assert_eq!(response["error"]["message"], "Tool not found: missingTool");
}

#[tokio::test]
async fn e3_e5_non_json_frame_is_dropped_and_sidecar_survives() {
    let server = sidecar().await;
    let mut client = Client::connect(&server, TOKEN).await.expect("connect");
    client.raw("{not json");
    assert_eq!(client.silence(Duration::from_millis(400)).await, None);
    assert!(client.call(3, "tools/list", None).await["result"]["tools"].is_array());
}

#[tokio::test]
async fn e3_e5_non_object_json_frames_are_dropped() {
    let server = sidecar().await;
    let mut client = Client::connect(&server, TOKEN).await.expect("connect");
    for frame in ["null", "5", "\"x\"", "false", "[]"] {
        client.raw(frame);
        assert_eq!(client.silence(Duration::from_millis(400)).await, None);
    }
    assert!(client.call(4, "tools/list", None).await["result"]["tools"].is_array());
}

#[tokio::test]
async fn e4_notification_is_never_answered() {
    let server = sidecar().await;
    let mut client = Client::connect(&server, TOKEN).await.expect("connect");
    client.raw(r#"{"jsonrpc":"2.0","method":"tools/list"}"#);
    assert_eq!(client.silence(Duration::from_millis(400)).await, None);
}

#[tokio::test]
async fn initialize_echoes_client_version() {
    let server = sidecar().await;
    let mut client = Client::connect(&server, TOKEN).await.expect("connect");
    let response = client
        .call(
            5,
            "initialize",
            Some(json!({"protocolVersion": "2099-01-01"})),
        )
        .await;
    assert_eq!(response["result"]["protocolVersion"], "2099-01-01");
    assert_eq!(response["result"]["serverInfo"]["name"], "yazi");
}

#[tokio::test]
async fn initialize_falls_back_to_2025_11_25() {
    let server = sidecar().await;
    let mut client = Client::connect(&server, TOKEN).await.expect("connect");
    let response = client.call(6, "initialize", Some(json!({}))).await;
    assert_eq!(response["result"]["protocolVersion"], "2025-11-25");
}

#[tokio::test]
async fn tools_list_advertises_four_tools() {
    let server = sidecar().await;
    let mut client = Client::connect(&server, TOKEN).await.expect("connect");
    let response = client.call(7, "tools/list", None).await;
    let names: Vec<_> = response["result"]["tools"]
        .as_array()
        .expect("tools should be an array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect();
    assert_eq!(
        names,
        [
            "getCurrentSelection",
            "getLatestSelection",
            "getWorkspaceFolders",
            "getOpenEditors",
        ]
    );
}

#[tokio::test]
async fn tools_call_routes_to_tool_layer() {
    let server = sidecar().await;
    let mut client = Client::connect(&server, TOKEN).await.expect("connect");
    let response = client
        .call(
            8,
            "tools/call",
            Some(json!({"name": "getWorkspaceFolders", "arguments": {}})),
        )
        .await;
    let payload: Value = serde_json::from_str(
        response["result"]["content"][0]["text"]
            .as_str()
            .expect("tool result text"),
    )
    .expect("tool result JSON");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["folders"][0]["path"], fixture("Cargo.toml"));
}

#[tokio::test]
async fn d8_multiple_clients_can_connect_and_receive_responses() {
    let server = sidecar().await;
    let mut first = Client::connect(&server, TOKEN)
        .await
        .expect("first connect");
    let mut second = Client::connect(&server, TOKEN)
        .await
        .expect("second connect");
    assert!(first.call(9, "tools/list", None).await["result"].is_object());
    assert!(second.call(10, "tools/list", None).await["result"].is_object());
}

#[tokio::test]
async fn stop_closes_an_already_open_connection() {
    let server = sidecar().await;
    let mut client = Client::connect(&server, TOKEN).await.expect("connect");
    assert!(client.call(11, "tools/list", None).await["result"].is_object());

    server.stop();
    // Idempotent: the second stop must not panic on an already-taken shutdown.
    server.stop();

    assert!(
        client.closed(Duration::from_secs(2)).await,
        "stop() must close connections already accepted, not just the listener"
    );
}
