use axum::extract::{FromRequestParts, Path, Query};
use axum::http::StatusCode;
use axum::routing::{delete, get};
use axum::{Extension, Json, Router};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::chat::models::{Channel, CreateChannel, Message, MessagesQuery, UpdateMessage};
use crate::WeaverState;
use crate::WeaverUser;

pub fn router<S, U>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    U: WeaverUser + FromRequestParts<S>,
    <U as FromRequestParts<S>>::Rejection: Into<axum::response::Response>,
{
    Router::new()
        .route(
            "/projects/{project_id}/channels",
            get(list_channels::<U>).post(create_channel::<U>),
        )
        .route("/channels/{channel_id}", delete(delete_channel::<U>))
        .route(
            "/channels/{channel_id}/messages",
            get(list_messages::<U>),
        )
        .route(
            "/messages/{message_id}",
            axum::routing::patch(update_message::<U>).delete(delete_message::<U>),
        )
}

async fn list_channels<U: WeaverUser>(
    _user: U,
    Extension(state): Extension<WeaverState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let channels = sqlx::query_as::<_, Channel>(
        "SELECT * FROM weaver_channels WHERE project_id = $1 ORDER BY created_at",
    )
    .bind(project_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("[Weaver] list channels error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(json!({ "channels": channels })))
}

async fn create_channel<U: WeaverUser>(
    user: U,
    Extension(state): Extension<WeaverState>,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<CreateChannel>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    if payload.name.is_empty() || payload.name.len() > 100 {
        return Err(StatusCode::BAD_REQUEST);
    }

    let channel = sqlx::query_as::<_, Channel>(
        "INSERT INTO weaver_channels (project_id, name, created_by) VALUES ($1, $2, $3) RETURNING *",
    )
    .bind(project_id)
    .bind(&payload.name)
    .bind(user.user_id())
    .fetch_one(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("[Weaver] create channel error: {e}");
        StatusCode::CONFLICT
    })?;

    // Broadcast channel creation to all project subscribers
    use crate::chat::hub::WsEvent;
    // Publish to all existing project channels so connected clients get notified
    let existing = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM weaver_channels WHERE project_id = $1 AND id != $2",
    )
    .bind(project_id)
    .bind(channel.id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let event = WsEvent::ChannelCreated {
        id: channel.id,
        project_id,
        name: channel.name.clone(),
    };
    for cid in existing {
        state.hub.publish(cid, event.clone()).await;
    }

    Ok((StatusCode::CREATED, Json(json!({ "channel": channel }))))
}

async fn delete_channel<U: WeaverUser>(
    _user: U,
    Extension(state): Extension<WeaverState>,
    Path(channel_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    sqlx::query("DELETE FROM weaver_channels WHERE id = $1")
        .bind(channel_id)
        .execute(&state.pool)
        .await
        .map_err(|e| {
            tracing::error!("[Weaver] delete channel error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(json!({ "success": true })))
}

async fn list_messages<U: WeaverUser>(
    _user: U,
    Extension(state): Extension<WeaverState>,
    Path(channel_id): Path<Uuid>,
    Query(query): Query<MessagesQuery>,
) -> Result<Json<Value>, StatusCode> {
    let limit = query.limit.unwrap_or(50).min(100);

    let messages = if let Some(ref before) = query.before {
        let before_ts: chrono::DateTime<chrono::Utc> = before
            .parse()
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        sqlx::query_as::<_, Message>(
            "SELECT * FROM weaver_messages WHERE channel_id = $1 AND created_at < $2 AND deleted_at IS NULL \
             ORDER BY created_at DESC LIMIT $3",
        )
        .bind(channel_id)
        .bind(before_ts)
        .bind(limit)
        .fetch_all(&state.pool)
        .await
    } else {
        sqlx::query_as::<_, Message>(
            "SELECT * FROM weaver_messages WHERE channel_id = $1 AND deleted_at IS NULL \
             ORDER BY created_at DESC LIMIT $2",
        )
        .bind(channel_id)
        .bind(limit)
        .fetch_all(&state.pool)
        .await
    }
    .map_err(|e| {
        tracing::error!("[Weaver] list messages error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Collect message IDs for batch-loading attachments and reply parents
    let msg_ids: Vec<Uuid> = messages.iter().map(|m| m.id).collect();

    // Load attachments for all messages
    let attachments = if !msg_ids.is_empty() {
        sqlx::query_as::<_, crate::chat::models::Attachment>(
            "SELECT * FROM weaver_attachments WHERE message_id = ANY($1)",
        )
        .bind(&msg_ids)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default()
    } else {
        vec![]
    };

    let mut attachments_map: std::collections::HashMap<Uuid, Vec<serde_json::Value>> =
        std::collections::HashMap::new();
    for att in attachments {
        if let Some(mid) = att.message_id {
            attachments_map
                .entry(mid)
                .or_default()
                .push(serde_json::to_value(&att).unwrap_or_default());
        }
    }

    // Collect reply parent IDs and fetch them
    let reply_ids: Vec<Uuid> = messages
        .iter()
        .filter_map(|m| m.reply_to_id)
        .collect();

    let reply_parents: std::collections::HashMap<Uuid, serde_json::Value> = if !reply_ids.is_empty() {
        let parents = sqlx::query_as::<_, Message>(
            "SELECT * FROM weaver_messages WHERE id = ANY($1)",
        )
        .bind(&reply_ids)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

        parents
            .into_iter()
            .map(|m| (m.id, serde_json::json!({
                "id": m.id,
                "user_email": m.user_email,
                "content": m.content.chars().take(200).collect::<String>(),
            })))
            .collect()
    } else {
        std::collections::HashMap::new()
    };

    // Build response with reply_to and attachments embedded
    let messages_json: Vec<serde_json::Value> = messages
        .into_iter()
        .map(|m| {
            let reply_to = m.reply_to_id.and_then(|rid| reply_parents.get(&rid).cloned());
            let atts = attachments_map.remove(&m.id).unwrap_or_default();
            let mut val = serde_json::to_value(&m).unwrap_or_default();
            if let Some(obj) = val.as_object_mut() {
                obj.insert("reply_to".to_string(), serde_json::json!(reply_to));
                obj.insert("attachments".to_string(), serde_json::json!(atts));
            }
            val
        })
        .collect();

    Ok(Json(json!({ "messages": messages_json })))
}

async fn update_message<U: WeaverUser>(
    user: U,
    Extension(state): Extension<WeaverState>,
    Path(message_id): Path<Uuid>,
    Json(payload): Json<UpdateMessage>,
) -> Result<Json<Value>, StatusCode> {
    if payload.content.is_empty() || payload.content.len() > 4000 {
        return Err(StatusCode::BAD_REQUEST);
    }

    let msg = sqlx::query_as::<_, Message>(
        "SELECT * FROM weaver_messages WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(message_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .ok_or(StatusCode::NOT_FOUND)?;

    if !user.can_edit_message(&msg.user_id) {
        return Err(StatusCode::FORBIDDEN);
    }

    sqlx::query(
        "UPDATE weaver_messages SET content = $1, updated_at = now() WHERE id = $2",
    )
    .bind(&payload.content)
    .bind(message_id)
    .execute(&state.pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({ "success": true })))
}

async fn delete_message<U: WeaverUser>(
    user: U,
    Extension(state): Extension<WeaverState>,
    Path(message_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let msg = sqlx::query_as::<_, Message>(
        "SELECT * FROM weaver_messages WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(message_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .ok_or(StatusCode::NOT_FOUND)?;

    if !user.can_delete_message(&msg.user_id) {
        return Err(StatusCode::FORBIDDEN);
    }

    sqlx::query("UPDATE weaver_messages SET deleted_at = now() WHERE id = $1")
        .bind(message_id)
        .execute(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({ "success": true })))
}
