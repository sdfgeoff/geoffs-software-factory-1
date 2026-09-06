use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt},
    sync::{Mutex, mpsc, oneshot},
    time::timeout,
};

use crate::error::AppError;

#[derive(Clone)]
pub(super) struct RpcPeer {
    writer: Arc<Mutex<Box<dyn AsyncWrite + Send + Unpin>>>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>,
    next_id: Arc<AtomicI64>,
}

impl RpcPeer {
    pub(super) fn attach<R, W>(reader: R, writer: W) -> (Self, mpsc::Receiver<Value>)
    where
        R: AsyncBufRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let peer = Self {
            writer: Arc::new(Mutex::new(Box::new(writer))),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicI64::new(1)),
        };
        let pending = peer.pending.clone();
        let (events_tx, events_rx) = mpsc::channel(256);
        tokio::spawn(async move {
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let response_id = message.get("id").and_then(Value::as_i64);
                let is_response = message.get("method").is_none();
                if let Some(id) = response_id
                    && is_response
                    && let Some(sender) = pending.lock().await.remove(&id)
                {
                    let _ = sender.send(message);
                    continue;
                }
                if events_tx.send(message).await.is_err() {
                    break;
                }
            }
        });
        (peer, events_rx)
    }

    pub(super) async fn request(&self, method: &str, params: Value) -> Result<Value, AppError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id, sender);
        if let Err(error) = self
            .send(json!({"method": method, "id": id, "params": params}))
            .await
        {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }
        let response = timeout(Duration::from_secs(30), receiver)
            .await
            .map_err(|_| AppError::internal(format!("Codex timed out responding to {method}")))?
            .map_err(|_| AppError::internal("Codex connection closed"))?;
        if let Some(error) = response.get("error") {
            return Err(AppError::internal(format!(
                "Codex {method} failed: {error}"
            )));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| AppError::internal("Codex response did not contain a result"))
    }

    pub(super) async fn notify(&self, method: &str, params: Value) -> Result<(), AppError> {
        self.send(json!({"method": method, "params": params})).await
    }

    pub(super) async fn respond(&self, id: Value, result: Value) -> Result<(), AppError> {
        self.send(json!({"id": id, "result": result})).await
    }

    async fn send(&self, message: Value) -> Result<(), AppError> {
        let mut writer = self.writer.lock().await;
        writer
            .write_all(format!("{message}\n").as_bytes())
            .await
            .map_err(|error| AppError::internal(format!("failed to write to Codex: {error}")))?;
        writer
            .flush()
            .await
            .map_err(|error| AppError::internal(format!("failed to flush Codex input: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, duplex, split};

    use super::*;

    #[tokio::test]
    async fn rpc_peer_talks_to_a_fake_app_server() {
        let (client, server) = duplex(4096);
        let (client_read, client_write) = split(client);
        let (server_read, mut server_write) = split(server);
        let (peer, _events) = RpcPeer::attach(BufReader::new(client_read), client_write);
        tokio::spawn(async move {
            let mut lines = BufReader::new(server_read).lines();
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(request["method"], "initialize");
            server_write
                .write_all(format!("{{\"id\":{},\"result\":{{}}}}\n", request["id"]).as_bytes())
                .await
                .unwrap();
        });

        assert_eq!(
            peer.request("initialize", json!({})).await.unwrap(),
            json!({})
        );
    }
}
