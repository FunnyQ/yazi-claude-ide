mod common;

use common::Client;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Error;
use yazi_claude_ide::server::{Sidecar, StartOptions, start_sidecar};

const TOKEN: &str = "rpc-test-token";

fn fixture(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(name)
        .to_string_lossy()
        .into_owned()
}

async fn sidecar() -> Sidecar {
    start_sidecar(StartOptions {
        workspace_folders: Box::new(|| vec![fixture("Cargo.toml")]),
        reveal: Box::new(|_| {}),
        auth_token: Some(TOKEN.to_owned()),
        port: None,
    })
    .await
    .expect("test sidecar should start")
}

fn assert_unauthorized(error: Error) {
    match error {
        Error::Http(response) => assert_eq!(response.status(), 401),
        other => panic!("expected HTTP rejection, got {other}"),
    }
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
