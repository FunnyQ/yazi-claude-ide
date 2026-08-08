// This module contains shared test helpers for integration tests.
// It's placed in a subdirectory because Cargo treats every top-level .rs file under tests/
// as its own test binary, so shared helpers must be one level down.
#![allow(dead_code)]

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::error::Elapsed;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Error as ConnectError, Message};
use yazi_claude_ide::server::Sidecar;

pub struct Client {
    outgoing: mpsc::UnboundedSender<Message>,
    incoming: mpsc::UnboundedReceiver<Value>,
}

impl Client {
    pub async fn connect(sidecar: &Sidecar, token: &str) -> Result<Client, ConnectError> {
        Self::connect_port(sidecar.port(), token).await
    }

    pub async fn connect_port(port: u16, token: &str) -> Result<Client, ConnectError> {
        let mut request = format!("ws://127.0.0.1:{port}").into_client_request()?;
        let header = token.parse().map_err(|error| {
            ConnectError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
        })?;
        request
            .headers_mut()
            .insert("x-claude-code-ide-authorization", header);
        let (socket, _) = tokio_tungstenite::connect_async(request).await?;
        let (mut writer, mut reader) = socket.split();
        let (outgoing, mut writes) = mpsc::unbounded_channel();
        let (reads, incoming) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(message) = writes.recv() => {
                        if writer.send(message).await.is_err() {
                            break;
                        }
                    }
                    message = reader.next() => {
                        match message {
                            Some(Ok(Message::Text(text))) => {
                                if let Ok(value) = serde_json::from_str(&text) {
                                    let _ = reads.send(value);
                                }
                            }
                            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                            Some(Ok(_)) => {}
                        }
                    }
                }
            }
        });

        Ok(Client { outgoing, incoming })
    }

    pub async fn next(&mut self, wait: Duration) -> Result<Value, Elapsed> {
        timeout(wait, async {
            match self.incoming.recv().await {
                Some(value) => value,
                None => std::future::pending().await,
            }
        })
        .await
    }

    pub async fn silence(&mut self, dur: Duration) -> Option<Value> {
        self.next(dur).await.ok()
    }

    pub async fn call(&mut self, id: i64, method: &str, params: Option<Value>) -> Value {
        let mut request = json!({"jsonrpc": "2.0", "id": id, "method": method});
        if let Some(params) = params {
            request["params"] = params;
        }
        let _ = self.outgoing.send(Message::text(request.to_string()));
        self.next(Duration::from_secs(2))
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {method} response"))
    }

    pub fn raw(&mut self, text: &str) {
        let _ = self.outgoing.send(Message::text(text.to_owned()));
    }

    pub fn close(self) {
        let _ = self.outgoing.send(Message::Close(None));
    }
}
