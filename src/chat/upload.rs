use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Extension, Json, Router};
use axum_extra::extract::Multipart;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::chat::models::Attachment;
use crate::WeaverState;
use crate::WeaverUser;

pub fn router<S, U>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    U: WeaverUser + FromRequestParts<S>,
    <U as FromRequestParts<S>>::Rejection: Into<axum::response::Response>,
{
    Router::new().route("/channels/{channel_id}/upload", post(upload_file::<U>))
}

async fn upload_file<U: WeaverUser>(
    _user: U,
    Extension(state): Extension<WeaverState>,
    axum::extract::Path(channel_id): axum::extract::Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Json<Value>, StatusCode> {
    let bucket = state.s3_bucket.as_ref().ok_or_else(|| {
        tracing::error!("[Weaver] S3 not configured");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        tracing::error!("[Weaver] multipart error: {e}");
        StatusCode::BAD_REQUEST
    })? {
        let filename = field.file_name().unwrap_or("unknown").to_string();
        let content_type = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_string();

        let data = field.bytes().await.map_err(|e| {
            tracing::error!("[Weaver] read field error: {e}");
            StatusCode::BAD_REQUEST
        })?;

        if data.len() > state.max_file_size {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }

        let file_id = Uuid::new_v4();
        let storage_key = format!("{}{}/{}", state.s3_prefix, channel_id, file_id);

        bucket
            .put_object_with_content_type(&storage_key, &data, &content_type)
            .await
            .map_err(|e| {
                tracing::error!("[Weaver] S3 upload error: {e}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        let url = format!(
            "{}/{}/{}",
            bucket.url(),
            bucket.name(),
            storage_key
        );

        // Create a placeholder message_id — will be updated when message is sent
        let placeholder_msg_id = Uuid::nil();

        let attachment = sqlx::query_as::<_, Attachment>(
            "INSERT INTO weaver_attachments (message_id, storage_key, url, filename, file_type, file_size) \
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
        )
        .bind(placeholder_msg_id)
        .bind(&storage_key)
        .bind(&url)
        .bind(&filename)
        .bind(&content_type)
        .bind(data.len() as i32)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| {
            tracing::error!("[Weaver] save attachment error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        return Ok(Json(json!({ "attachment": attachment })));
    }

    Err(StatusCode::BAD_REQUEST)
}
