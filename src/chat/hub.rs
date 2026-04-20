use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

/// A WebSocket event broadcast to channel subscribers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsEvent {
    Message {
        id: Uuid,
        channel_id: Uuid,
        user_id: String,
        user_email: String,
        content: String,
        created_at: String,
        attachments: Vec<serde_json::Value>,
    },
    ChannelCreated {
        id: Uuid,
        project_id: Uuid,
        name: String,
    },
}

const BUFFER_CAPACITY: usize = 256;

pub struct Hub {
    channels: RwLock<HashMap<Uuid, broadcast::Sender<Arc<WsEvent>>>>,
}

impl Hub {
    pub fn new() -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
        }
    }

    pub async fn subscribe(&self, channel_id: Uuid) -> broadcast::Receiver<Arc<WsEvent>> {
        {
            let map = self.channels.read().await;
            if let Some(tx) = map.get(&channel_id) {
                return tx.subscribe();
            }
        }
        let mut map = self.channels.write().await;
        let tx = map.entry(channel_id).or_insert_with(|| {
            let (tx, _) = broadcast::channel(BUFFER_CAPACITY);
            tx
        });
        tx.subscribe()
    }

    pub async fn publish(&self, channel_id: Uuid, event: WsEvent) {
        let map = self.channels.read().await;
        if let Some(tx) = map.get(&channel_id) {
            let _ = tx.send(Arc::new(event));
        }
    }

    /// Subscribe to all channels for a given project (returns list of receivers).
    pub async fn subscribe_project(
        &self,
        pool: &sqlx::PgPool,
        project_id: Uuid,
    ) -> Vec<(Uuid, broadcast::Receiver<Arc<WsEvent>>)> {
        let channel_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM weaver_channels WHERE project_id = $1",
        )
        .bind(project_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();

        let mut receivers = Vec::new();
        for cid in channel_ids {
            let rx = self.subscribe(cid).await;
            receivers.push((cid, rx));
        }
        receivers
    }
}
