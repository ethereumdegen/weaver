use axum::extract::ws::{Message as WsMessage, WebSocket};
use axum::extract::{FromRequestParts, Path, WebSocketUpgrade};
use axum::response::Response;
use axum::routing::get;
use axum::{Extension, Router};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use uuid::Uuid;

use crate::chat::hub::WsEvent;
use crate::chat::models::{Attachment, Message};
use crate::WeaverState;
use crate::WeaverUser;

pub fn router<S, U>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    U: WeaverUser + FromRequestParts<S>,
    <U as FromRequestParts<S>>::Rejection: Into<axum::response::Response>,
{
    Router::new().route("/ws/{project_id}", get(ws_handler::<U>))
}

#[derive(serde::Deserialize)]
struct ClientMessage {
    channel_id: Uuid,
    content: String,
    #[serde(default)]
    attachment_ids: Vec<Uuid>,
    reply_to_id: Option<Uuid>,
}

async fn ws_handler<U: WeaverUser>(
    user: U,
    Extension(state): Extension<WeaverState>,
    Path(project_id): Path<Uuid>,
    ws: WebSocketUpgrade,
) -> Response {
    let user_id = user.user_id().to_string();
    let user_email = user.email().to_string();
    ws.on_upgrade(move |socket| handle_socket(socket, state, project_id, user_id, user_email))
}

async fn handle_socket(
    socket: WebSocket,
    state: WeaverState,
    project_id: Uuid,
    user_id: String,
    user_email: String,
) {
    let (mut sink, mut stream) = socket.split();

    // Subscribe to all project channels
    let receivers = state.hub.subscribe_project(&state.pool, project_id).await;

    // Spawn a task to forward broadcast events to the WebSocket
    let (outgoing_tx, mut outgoing_rx) = tokio::sync::mpsc::channel::<Arc<WsEvent>>(256);

    let mut subscribed_channels: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    for (cid, mut rx) in receivers {
        subscribed_channels.insert(cid);
        let tx = outgoing_tx.clone();
        tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                if tx.send(event).await.is_err() {
                    break;
                }
            }
        });
    }

    // Forward outgoing events to WebSocket sink
    let send_task = tokio::spawn(async move {
        while let Some(event) = outgoing_rx.recv().await {
            let json = serde_json::to_string(&*event).unwrap_or_default();
            if sink.send(WsMessage::Text(json.into())).await.is_err() {
                break;
            }
        }
    });

    // Process incoming messages from client
    let hub = state.hub.clone();
    let pool = state.pool.clone();
    let uid = user_id.clone();
    let uemail = user_email.clone();
    let outgoing_tx_for_sub = outgoing_tx.clone();
    let recv_task = tokio::spawn(async move {
        let mut subscribed = subscribed_channels;
        while let Some(Ok(msg)) = stream.next().await {
            match msg {
                WsMessage::Text(text) => {
                    let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) else {
                        continue;
                    };

                    // Validate content length
                    if client_msg.content.is_empty() || client_msg.content.len() > 4000 {
                        continue;
                    }

                    // Dynamically subscribe to channel if not already
                    if !subscribed.contains(&client_msg.channel_id) {
                        let mut rx = hub.subscribe(client_msg.channel_id).await;
                        subscribed.insert(client_msg.channel_id);
                        let tx = outgoing_tx_for_sub.clone();
                        tokio::spawn(async move {
                            while let Ok(event) = rx.recv().await {
                                if tx.send(event).await.is_err() {
                                    break;
                                }
                            }
                        });
                    }

                    // Validate reply_to_id: must exist and not be a reply itself
                    let reply_to_id = if let Some(rto) = client_msg.reply_to_id {
                        let parent = sqlx::query_as::<_, Message>(
                            "SELECT * FROM weaver_messages WHERE id = $1 AND deleted_at IS NULL",
                        )
                        .bind(rto)
                        .fetch_optional(&pool)
                        .await
                        .ok()
                        .flatten();
                        match parent {
                            Some(p) if p.reply_to_id.is_none() => Some(rto),
                            _ => None, // skip invalid reply targets
                        }
                    } else {
                        None
                    };

                    // Persist message
                    let result = sqlx::query_as::<_, Message>(
                        "INSERT INTO weaver_messages (channel_id, user_id, user_email, content, reply_to_id) \
                         VALUES ($1, $2, $3, $4, $5) RETURNING *",
                    )
                    .bind(client_msg.channel_id)
                    .bind(&uid)
                    .bind(&uemail)
                    .bind(&client_msg.content)
                    .bind(reply_to_id)
                    .fetch_one(&pool)
                    .await;

                    let Ok(message) = result else {
                        tracing::error!("[Weaver] Failed to persist message");
                        continue;
                    };

                    // Link any attachments to this message
                    let mut attachment_values = Vec::new();
                    for att_id in &client_msg.attachment_ids {
                        let att = sqlx::query_as::<_, Attachment>(
                            "UPDATE weaver_attachments SET message_id = $1 WHERE id = $2 RETURNING *",
                        )
                        .bind(message.id)
                        .bind(att_id)
                        .fetch_optional(&pool)
                        .await
                        .ok()
                        .flatten();
                        if let Some(a) = att {
                            attachment_values.push(serde_json::to_value(&a).unwrap_or_default());
                        }
                    }

                    // Get reply parent snippet for broadcast
                    let reply_to = if let Some(rto) = message.reply_to_id {
                        sqlx::query_as::<_, Message>(
                            "SELECT * FROM weaver_messages WHERE id = $1",
                        )
                        .bind(rto)
                        .fetch_optional(&pool)
                        .await
                        .ok()
                        .flatten()
                        .map(|m| serde_json::json!({
                            "id": m.id,
                            "user_email": m.user_email,
                            "content": m.content.chars().take(200).collect::<String>(),
                        }))
                    } else {
                        None
                    };

                    // Broadcast
                    let event = WsEvent::Message {
                        id: message.id,
                        channel_id: message.channel_id,
                        user_id: message.user_id,
                        user_email: message.user_email,
                        content: message.content,
                        created_at: message.created_at.to_rfc3339(),
                        attachments: attachment_values,
                        reply_to_id: message.reply_to_id,
                        reply_to,
                    };
                    hub.publish(client_msg.channel_id, event).await;
                }
                WsMessage::Close(_) => break,
                _ => {}
            }
        }
    });

    // Wait for either task to complete
    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }
}
