use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use std::io;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, oneshot};
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::handshake::server::ErrorResponse;
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};

use crate::tools::{self, ToolContext};

pub struct StartOptions {
    pub workspace_folders: Box<dyn Fn() -> Vec<String> + Send + Sync>,
    pub reveal: Box<dyn Fn(&str) + Send + Sync>,
    pub auth_token: Option<String>,
    pub port: Option<u16>,
}

struct State {
    focused: Option<String>,
    // PORT-05 push-stream state is declared early so the shared state shape stays stable.
    #[allow(dead_code)]
    hovered: Option<String>,
    // PORT-05 push-stream state is declared early so the shared state shape stays stable.
    #[allow(dead_code)]
    last_pushed: Option<String>,
    clients: usize,
}

struct Inner {
    auth_token: String,
    port: u16,
    state: Mutex<State>,
    workspace_folders: Box<dyn Fn() -> Vec<String> + Send + Sync>,
    reveal: Box<dyn Fn(&str) + Send + Sync>,
    broadcasts: broadcast::Sender<String>,
    shutdown: Mutex<Option<oneshot::Sender<()>>>,
}

pub struct Sidecar {
    inner: Arc<Inner>,
}

impl Sidecar {
    pub fn port(&self) -> u16 {
        self.inner.port
    }

    pub fn hostname(&self) -> &str {
        "127.0.0.1"
    }

    pub fn auth_token(&self) -> &str {
        &self.inner.auth_token
    }

    pub fn set_focus(&self, _file_path: Option<&str>) {
        todo!()
    }

    pub fn focused_file(&self) -> Option<String> {
        self.inner
            .state
            .lock()
            .ok()
            .and_then(|state| state.focused.clone())
    }

    pub fn mention(&self, _file_paths: &[String]) {
        todo!()
    }

    pub fn stop(&self) {
        let sender = self
            .inner
            .shutdown
            .lock()
            .ok()
            .and_then(|mut shutdown| shutdown.take());
        if let Some(sender) = sender {
            let _ = sender.send(());
        }
    }
}

impl ToolContext for Inner {
    fn focused_file(&self) -> Option<String> {
        self.state
            .lock()
            .ok()
            .and_then(|state| state.focused.clone())
    }

    fn workspace_folders(&self) -> Vec<String> {
        (self.workspace_folders)()
    }

    fn reveal(&self, file_path: &str) {
        (self.reveal)(file_path);
    }
}

// tungstenite requires the auth callback to return a full HTTP rejection response.
#[allow(clippy::result_large_err)]
pub async fn start_sidecar(opts: StartOptions) -> io::Result<Sidecar> {
    let auth_token = match opts.auth_token {
        Some(token) => token,
        None => generate_auth_token()?,
    };
    let listener = TcpListener::bind(("127.0.0.1", opts.port.unwrap_or(0))).await?;
    let port = listener.local_addr()?.port();
    let (broadcasts, _) = broadcast::channel(32);
    let (shutdown_sender, mut shutdown_receiver) = oneshot::channel();
    let inner = Arc::new(Inner {
        auth_token,
        port,
        state: Mutex::new(State {
            focused: None,
            hovered: None,
            last_pushed: None,
            clients: 0,
        }),
        workspace_folders: opts.workspace_folders,
        reveal: opts.reveal,
        broadcasts,
        shutdown: Mutex::new(Some(shutdown_sender)),
    });
    let listener_inner = Arc::clone(&inner);

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_receiver => break,
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _)) => {
                            let connection_inner = Arc::clone(&listener_inner);
                            tokio::spawn(async move {
                                let expected_token = connection_inner.auth_token.clone();
                                let socket = accept_hdr_async(stream, move |request: &tokio_tungstenite::tungstenite::http::Request<()>, response| {
                                    let authorized = request
                                        .headers()
                                        .get("x-claude-code-ide-authorization")
                                        .is_some_and(|value| value.as_bytes() == expected_token.as_bytes());
                                    if authorized {
                                        Ok(response)
                                    } else {
                                        let mut rejection = ErrorResponse::new(Some("Unauthorized".to_owned()));
                                        *rejection.status_mut() = StatusCode::UNAUTHORIZED;
                                        Err(rejection)
                                    }
                                })
                                .await;

                                if let Ok(socket) = socket {
                                    serve_connection(socket, connection_inner).await;
                                }
                            });
                        }
                        Err(error) => {
                            eprintln!("WebSocket listener error: {error}");
                            break;
                        }
                    }
                }
            }
        }
    });

    Ok(Sidecar { inner })
}

fn generate_auth_token() -> io::Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

async fn serve_connection<S>(socket: tokio_tungstenite::WebSocketStream<S>, inner: Arc<Inner>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if let Ok(mut state) = inner.state.lock() {
        state.clients += 1;
    }
    let mut broadcasts = inner.broadcasts.subscribe();
    let (mut writer, mut reader) = socket.split();

    loop {
        tokio::select! {
            incoming = reader.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Some(response) = handle_json_rpc(text.as_str(), inner.as_ref())
                            && writer.send(Message::Text(response.into())).await.is_err()
                        {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                }
            }
            broadcast = broadcasts.recv() => {
                match broadcast {
                    Ok(frame) => {
                        if writer.send(Message::Text(Utf8Bytes::from(frame))).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }

    if let Ok(mut state) = inner.state.lock() {
        state.clients = state.clients.saturating_sub(1);
        if state.clients == 0 {
            state.last_pushed = None;
        }
    }
}

fn handle_json_rpc(frame: &str, ctx: &dyn ToolContext) -> Option<String> {
    let parsed: Value = match serde_json::from_str(frame) {
        Ok(value) => value,
        Err(_) => {
            log_dropped_frame(frame);
            return None;
        }
    };
    let Some(request) = parsed.as_object() else {
        log_dropped_frame(frame);
        return None;
    };
    let id = request.get("id")?.clone();
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").and_then(Value::as_object);

    let response = match method {
        "initialize" => {
            let protocol_version = params
                .and_then(|params| params.get("protocolVersion"))
                .and_then(Value::as_str)
                .unwrap_or("2025-11-25");
            success(
                id,
                json!({
                    "protocolVersion": protocol_version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "yazi", "version": "0.1.0" },
                }),
            )
        }
        "tools/list" => success(id, json!({ "tools": tools::advertised_json() })),
        "tools/call" => {
            let name = params
                .and_then(|params| params.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let arguments = params
                .and_then(|params| params.get("arguments"))
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_else(Map::new);
            match tools::call_tool(name, &arguments, ctx) {
                Some(result) => success(id, json!(result)),
                None => failure(id, format!("Tool not found: {name}")),
            }
        }
        _ => failure(id, format!("Method not found: {method}")),
    };

    Some(response.to_string())
}

fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn failure(id: Value, message: String) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32601, "message": message },
    })
}

fn log_dropped_frame(frame: &str) {
    let truncated: String = frame.chars().take(200).collect();
    eprintln!("Dropped malformed JSON-RPC frame: {truncated}");
}
