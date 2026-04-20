use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Channel {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Message {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub user_id: String,
    pub user_email: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub reply_to_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMessage {
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Attachment {
    pub id: Uuid,
    pub message_id: Option<Uuid>,
    pub storage_key: String,
    pub url: String,
    pub filename: String,
    pub file_type: String,
    pub file_size: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageWithAttachments {
    #[serde(flatten)]
    pub message: Message,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Deserialize)]
pub struct CreateChannel {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct MessagesQuery {
    pub before: Option<String>,
    pub limit: Option<i64>,
}
