use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};

use crate::tools::{self, DiffRequest, Position, Selection, SelectionPayload, ToolContext};

pub struct StartOptions {
    pub workspace_folders: Box<dyn Fn() -> Vec<String> + Send + Sync>,
    pub reveal: Box<dyn Fn(&str) + Send + Sync>,
    /// J1-J3. Returns the path whose contents J5 reads back, or `None` when no
    /// viewer is configured or the launch failed (J7). Everything on disk and
    /// everything about yazi lives behind this closure; the server owns only the
    /// token, the held request, and the answer.
    pub open_diff: Box<dyn Fn(DiffLaunch<'_>) -> Option<PathBuf> + Send + Sync>,
    pub auth_token: String,
}

/// What the viewer needs to exist, handed to the closure above.
pub struct DiffLaunch<'a> {
    pub token: &'a str,
    pub old_path: &'a str,
    pub new_contents: &'a str,
}

/// An `openDiff` whose viewer is up, waiting for J4 to name its token.
struct PendingDiff {
    token: String,
    rpc_id: Value,
    connection: u64,
    new_path: PathBuf,
}

/// One open WebSocket, addressed by an unbounded queue.
///
/// Unbounded rather than a bounded broadcast: H5 and H9 require every marked item
/// to reach every connection, in order. A bounded channel drops the oldest frames
/// when a sender outruns a receiver, and `mention` sends a whole marked set in one
/// synchronous loop, so a large set would silently lose its earliest mentions.
struct Connection {
    id: u64,
    frames: mpsc::UnboundedSender<String>,
}

struct State {
    focused: Option<String>,
    hovered: Option<String>,
    last_pushed: Option<String>,
    clients: Vec<Connection>,
    pending_diffs: Vec<PendingDiff>,
}

struct Inner {
    auth_token: String,
    port: u16,
    state: Mutex<State>,
    workspace_folders: Box<dyn Fn() -> Vec<String> + Send + Sync>,
    reveal: Box<dyn Fn(&str) + Send + Sync>,
    open_diff: Box<dyn Fn(DiffLaunch<'_>) -> Option<PathBuf> + Send + Sync>,
    next_connection_id: AtomicU64,
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
            // Gate on the bare stat, not on a full payload: hover fires on every
            // cursor keystroke, and only `selection_frame` needs the payload.
            state.focused = file_path
                .filter(|path| tools::is_file(path))
                .map(str::to_owned);
        }
        self.inner.push();
    }

    pub fn focused_file(&self) -> Option<String> {
        self.inner.focused()
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
            self.inner.broadcast(frame);
        }
    }

    /// I5. `lines` is 1-based and inclusive as the editor counts them; `chars` is
    /// already 0-based with an exclusive end and passes straight through. The two
    /// conventions differ on purpose — I4 says why. `text` is the editor's own
    /// buffer contents; the sidecar never reads a file to fill it (C4).
    pub fn set_editor_selection(
        &self,
        file_path: &str,
        lines: (u32, u32),
        chars: (u32, u32),
        text: &str,
    ) {
        if !tools::is_file(file_path) {
            return;
        }
        // Derived, never asserted (I5). A zero-width range is how an editor says
        // the selection is gone, and claiming `isEmpty: false` there would leave
        // the CLI showing a selection the user has already dismissed.
        let is_empty = lines.0 == lines.1 && chars.0 == chars.1;
        // A selection covering nothing has no contents, whatever the editor sent —
        // and the CLI counts its display from this field.
        let text = if is_empty { "" } else { text };
        // No dedupe (I7): dragging a selection sends range after range for one
        // unchanged path, and D6's path-keyed check would swallow all but the first.
        let frame = json!({
            "jsonrpc": "2.0",
            "method": "selection_changed",
            "params": {
                "text": text,
                "filePath": file_path,
                "fileUrl": format!("file://{file_path}"),
                "selection": Selection {
                    start: Position { line: lines.0.saturating_sub(1), character: chars.0 },
                    end: Position { line: lines.1.saturating_sub(1), character: chars.1 },
                    is_empty,
                },
            },
        })
        .to_string();
        if !self.inner.broadcast(frame) {
            return;
        }
        // I8. The CLI now displays a range, and D6 would keep the next hover onto
        // this same file silent — leaving that range on screen after the user has
        // left it. Forgetting the path is what makes that hover speak again.
        if let Ok(mut state) = self.inner.state.lock() {
            state.last_pushed = None;
        }
    }

    /// Idempotent — stopping twice is not an error.
    /// J5. The viewer for `token` has exited: answer its held `openDiff` with the
    /// file as it now stands, then take the copy off disk. An unknown token is
    /// dropped (J4), and so is a connection that closed while the user read (J7).
    pub fn finish_diff(&self, token: &str) {
        let Ok(mut state) = self.inner.state.lock() else {
            return;
        };
        let Some(index) = state
            .pending_diffs
            .iter()
            .position(|pending| pending.token == token)
        else {
            return;
        };
        let pending = state.pending_diffs.remove(index);
        let frames = state
            .clients
            .iter()
            .find(|client| client.id == pending.connection)
            .map(|client| client.frames.clone());
        drop(state);

        // Whatever the user left behind, including nothing: a viewer that only
        // displays leaves the sidecar's copy exactly as it was written (J1).
        let contents = std::fs::read_to_string(&pending.new_path).unwrap_or_default();
        discard_diff_copy(&pending.new_path);
        let Some(frames) = frames else {
            return;
        };
        let _ = frames.send(
            json!({
                "jsonrpc": "2.0",
                "id": pending.rpc_id,
                "result": {
                    "content": [
                        { "type": "text", "text": "FILE_SAVED" },
                        { "type": "text", "text": contents },
                    ],
                },
            })
            .to_string(),
        );
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
        // Dropping every queue sender is what ends the connection tasks: each one
        // sees its receiver close and answers with a WebSocket close frame. The
        // listener oneshot above only stops new connections being accepted.
        if let Ok(mut state) = self.inner.state.lock() {
            state.clients.clear();
            state.last_pushed = None;
        }
    }
}

impl Inner {
    fn focused(&self) -> Option<String> {
        self.state
            .lock()
            .ok()
            .and_then(|state| state.focused.clone())
    }

    fn mark_pushed(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.last_pushed = state.focused.clone();
        }
    }

    /// D1-D2. `None` when there is nothing to say: nothing focused, or a path that
    /// stopped statting since `set_focus` accepted it.
    fn selection_frame(&self) -> Option<String> {
        selection_frame(self.focused().as_deref())
    }

    /// Queue a frame for every open connection. `false` when nobody is listening.
    fn broadcast(&self, frame: String) -> bool {
        let Ok(state) = self.state.lock() else {
            return false;
        };
        if state.clients.is_empty() {
            return false;
        }
        for client in &state.clients {
            let _ = client.frames.send(frame.clone());
        }
        true
    }

    /// Broadcast a focus change. Silent with nothing new or nobody listening.
    fn push(&self) {
        let Ok(state) = self.state.lock() else {
            return;
        };
        if state.focused == state.last_pushed || state.clients.is_empty() {
            return;
        }
        let focused = state.focused.clone();
        drop(state);

        let Some(frame) = selection_frame(focused.as_deref()) else {
            return;
        };
        if self.broadcast(frame) {
            self.mark_pushed();
        }
    }
}

fn selection_frame(focused: Option<&str>) -> Option<String> {
    let SelectionPayload::Success {
        file_path,
        text,
        selection,
        ..
    } = tools::selection_payload(focused)
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

impl ToolContext for Inner {
    fn focused_file(&self) -> Option<String> {
        self.focused()
    }

    fn workspace_folders(&self) -> Vec<String> {
        (self.workspace_folders)()
    }

    fn reveal(&self, file_path: &str) {
        (self.reveal)(file_path);
    }

    fn open_diff(&self, connection: u64, rpc_id: &Value, request: DiffRequest<'_>) -> bool {
        // J4's completion rides a machine-wide broadcast, so the token has to be
        // unguessable: anyone who can name it can finish someone else's diff with
        // bytes of their choosing.
        let token = crate::lock::new_auth_token();
        let Some(new_path) = (self.open_diff)(DiffLaunch {
            token: &token,
            old_path: request.old_path,
            new_contents: request.new_contents,
        }) else {
            return false;
        };
        // J8. The path and the tab, never the contents — this log lands in /tmp.
        eprintln!(
            "yazi-claude-ide: diff {} ({})",
            request.old_path, request.tab_name
        );
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        state.pending_diffs.push(PendingDiff {
            token,
            rpc_id: rpc_id.clone(),
            connection,
            new_path,
        });
        true
    }
}

// tungstenite requires the auth callback to return a full HTTP rejection response.
#[allow(clippy::result_large_err)]
pub async fn start_sidecar(opts: StartOptions) -> io::Result<Sidecar> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let (shutdown_sender, mut shutdown_receiver) = oneshot::channel();
    let inner = Arc::new(Inner {
        auth_token: opts.auth_token,
        port,
        state: Mutex::new(State {
            focused: None,
            hovered: None,
            last_pushed: None,
            clients: Vec::new(),
            pending_diffs: Vec::new(),
        }),
        workspace_folders: opts.workspace_folders,
        reveal: opts.reveal,
        open_diff: opts.open_diff,
        next_connection_id: AtomicU64::new(0),
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
                                let socket = accept_hdr_async(stream, move |request: &tokio_tungstenite::tungstenite::http::Request<()>, mut response: Response| {
                                    let authorized = request
                                        .headers()
                                        .get("x-claude-code-ide-authorization")
                                        .is_some_and(|value| value.as_bytes() == expected_token.as_bytes());
                                    if !authorized {
                                        let mut rejection = ErrorResponse::new(Some("Unauthorized".to_owned()));
                                        *rejection.status_mut() = StatusCode::UNAUTHORIZED;
                                        return Err(rejection);
                                    }
                                    // E6. Claude Code asks for `mcp` and hangs up on a 101 that does
                                    // not name it back, which the CLI reports only as a failure to
                                    // connect. tungstenite does not echo it for us.
                                    if let Some(protocol) = request.headers().get("sec-websocket-protocol") {
                                        response
                                            .headers_mut()
                                            .insert("sec-websocket-protocol", protocol.clone());
                                    }
                                    Ok(response)
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

async fn serve_connection<S>(socket: tokio_tungstenite::WebSocketStream<S>, inner: Arc<Inner>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let id = inner.next_connection_id.fetch_add(1, Ordering::Relaxed);
    let (frames_sender, mut frames) = mpsc::unbounded_channel();
    if let Ok(mut state) = inner.state.lock() {
        state.clients.push(Connection {
            id,
            frames: frames_sender,
        });
    }
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
            close_connection(&inner, id);
            return;
        }
        inner.mark_pushed();
    }

    loop {
        tokio::select! {
            incoming = reader.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Some(response) = handle_json_rpc(text.as_str(), inner.as_ref(), id)
                            && writer.send(Message::Text(response.into())).await.is_err()
                        {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                }
            }
            frame = frames.recv() => {
                match frame {
                    Some(frame) => {
                        if writer.send(Message::Text(Utf8Bytes::from(frame))).await.is_err() {
                            break;
                        }
                    }
                    // `Sidecar::stop` dropped this connection's sender.
                    None => {
                        let _ = writer.send(Message::Close(None)).await;
                        break;
                    }
                }
            }
        }
    }

    close_connection(&inner, id);
}

/// J5. The launcher makes one directory per request and puts nothing in it but
/// this copy and the script yazi ran, so the directory goes with the copy.
/// Failures are ignored: a file left in the temp directory is not worth an error
/// path (J7).
fn discard_diff_copy(new_path: &std::path::Path) {
    match new_path.parent() {
        Some(dir) => {
            let _ = std::fs::remove_dir_all(dir);
        }
        None => {
            let _ = std::fs::remove_file(new_path);
        }
    }
}

fn close_connection(inner: &Inner, id: u64) {
    if let Ok(mut state) = inner.state.lock() {
        state.clients.retain(|client| client.id != id);
        // J7. Nobody is left to answer, and the viewer may well still be up; the
        // publish that follows it will find no pending entry and stop there.
        let orphaned: Vec<_> = state
            .pending_diffs
            .extract_if(.., |pending| pending.connection == id)
            .collect();
        for pending in &orphaned {
            discard_diff_copy(&pending.new_path);
        }
        if state.clients.is_empty() {
            // The next connection is owed D3 again — one push of the then-current
            // file. Clearing `last_pushed` ensures the next connection sees its D3
            // push even if the focused file never changes.
            state.last_pushed = None;
        }
    }
}

fn handle_json_rpc(frame: &str, ctx: &dyn ToolContext, connection: u64) -> Option<String> {
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
                    "serverInfo": { "name": "yazi", "version": env!("CARGO_PKG_VERSION") },
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
            // J1. openDiff is the one tool answered out of band, so it gets first
            // refusal: a launched viewer holds the request until J5, and anything
            // else falls through to call_tool's -32601 (F5, J7).
            if name == "openDiff" {
                let argument = |key| {
                    arguments
                        .get(key)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                };
                let old_path = argument("old_file_path");
                if !old_path.is_empty()
                    && ctx.open_diff(
                        connection,
                        &id,
                        DiffRequest {
                            old_path,
                            new_contents: argument("new_file_contents"),
                            tab_name: argument("tab_name"),
                        },
                    )
                {
                    return None;
                }
            }
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
