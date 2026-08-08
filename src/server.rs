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

use crate::tools::{self, SelectionPayload, ToolContext};

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

    pub fn set_focus(&self, file_path: Option<&str>) {
        if let Ok(mut state) = self.inner.state.lock() {
            // Why `hovered` and `focused` are two variables. C5 makes the focused
            // file file-only, so a directory can never reach `selection_changed`,
            // where `filePath` claims an open editor. But the empty-marked-set
            // fallback has to work when the user is standing on a folder — that is
            // exactly when it is most useful. One variable serving both purposes
            // would break one of them.
            state.hovered = file_path.map(str::to_owned);
            state.focused = match tools::selection_payload(file_path) {
                SelectionPayload::Success { file_path, .. } => Some(file_path),
                SelectionPayload::Failure { .. } => None,
            };
        }
        self.inner.push();
    }

    pub fn focused_file(&self) -> Option<String> {
        self.inner
            .state
            .lock()
            .ok()
            .and_then(|state| state.focused.clone())
    }

    pub fn mention(&self, file_paths: &[String]) {
        let paths = if file_paths.is_empty() {
            self.inner
                .state
                .lock()
                .ok()
                .and_then(|state| state.hovered.clone())
                .into_iter()
                .collect::<Vec<_>>()
        } else {
            file_paths.to_vec()
        };

        for file_path in paths {
            if !tools::exists(&file_path) {
                continue;
            }
            // Measured: omitting them renders `@<file>`, while sending `0` for both
            // renders `@<file>#L1`, a line anchor a marked file never meant (H4).
            let frame = json!({
                "jsonrpc": "2.0",
                "method": "at_mentioned",
                "params": { "filePath": file_path },
            })
            .to_string();
            let _ = self.inner.broadcasts.send(frame);
        }
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

impl Inner {
    /// D1-D2. `None` when there is nothing to say: nothing focused, or a path that
    /// stopped statting since `set_focus` accepted it.
    fn selection_frame(&self) -> Option<String> {
        let focused = self
            .state
            .lock()
            .ok()
            .and_then(|state| state.focused.clone());
        let SelectionPayload::Success {
            file_path,
            text,
            selection,
            ..
        } = tools::selection_payload(focused.as_deref())
        else {
            return None;
        };
        Some(
            json!({
                "jsonrpc": "2.0",
                "method": "selection_changed",
                "params": {
                    "text": text,
                    "filePath": file_path,
                    "fileUrl": format!("file://{file_path}"),
                    "selection": selection,
                },
            })
            .to_string(),
        )
    }

    /// Broadcast a focus change. Silent with nothing new or nobody listening.
    fn push(&self) {
        let should_push = self
            .state
            .lock()
            .ok()
            .is_some_and(|state| state.focused != state.last_pushed && state.clients > 0);
        if !should_push {
            return;
        }
        let Some(frame) = self.selection_frame() else {
            return;
        };
        if self.broadcasts.send(frame).is_err() {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.last_pushed = state.focused.clone();
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

    // D3 is owed to each connection separately. A new client must be pushed the
    // current file, even if another socket already holds it. Going through `push()`
    // would let `last_pushed` swallow the frame, leaving the new client in the dark.
    if let Some(frame) = inner.selection_frame() {
        if writer
            .send(Message::Text(Utf8Bytes::from(frame)))
            .await
            .is_err()
        {
            close_connection(&inner);
            return;
        }
        if let Ok(mut state) = inner.state.lock() {
            state.last_pushed = state.focused.clone();
        }
    }

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

    close_connection(&inner);
}

fn close_connection(inner: &Inner) {
    if let Ok(mut state) = inner.state.lock() {
        state.clients = state.clients.saturating_sub(1);
        if state.clients == 0 {
            // The next connection is owed D3 again — one push of the then-current
            // file. Clearing `last_pushed` ensures the next connection sees its D3
            // push even if the focused file never changes.
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
