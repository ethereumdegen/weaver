# Weaver

Embeddable **project chat + issue tracking** for Rust/Axum applications.

Weaver gives each project in your app a Slack-like chat experience (channels, real-time WebSocket messaging, file attachments) and a Linear-like issue tracker (kanban board, labels, comments) — all backed by Postgres.

## Features

**Chat**
- Multi-channel messaging scoped per project
- Real-time via WebSocket with per-channel broadcast
- File attachments via S3-compatible storage
- Message history with cursor-based pagination

**Issues**
- Kanban board with configurable statuses: `backlog`, `todo`, `in_progress`, `done`, `cancelled`
- Priority levels: `urgent`, `high`, `medium`, `low`
- Labels with custom colors
- Comments on issues
- Auto-incrementing issue numbers per project
- Assignee tracking

## Design Philosophy

- **Your auth, your rules** — Weaver never touches authentication. You implement the `WeaverUser` trait to provide user identity.
- **Your database** — You pass in a `PgPool`. Weaver runs its own migrations (all tables prefixed `weaver_`) and never creates foreign keys to your tables.
- **Your app state** — Weaver's router is generic over your app state. Just nest it into your existing Axum app.

## Quick Start

### 1. Add the dependency

```toml
[dependencies]
weaver = { git = "https://github.com/ethereumdegen/weaver.git", branch = "main" }
```

### 2. Implement `WeaverUser`

Weaver needs to know who the current user is. Create an extractor that implements `WeaverUser`:

```rust
use axum::extract::FromRequestParts;
use axum::http::{request::Parts, StatusCode};
use axum::response::{IntoResponse, Response};

pub struct MyUser {
    pub user_id: String,
    pub email: String,
}

impl weaver::WeaverUser for MyUser {
    fn user_id(&self) -> &str { &self.user_id }
    fn email(&self) -> &str { &self.email }
}

// Your rejection type must implement Into<Response>
pub struct AuthRejection(StatusCode);
impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response { self.0.into_response() }
}
impl From<AuthRejection> for Response {
    fn from(r: AuthRejection) -> Response { r.into_response() }
}

impl<S> FromRequestParts<S> for MyUser
where
    S: Send + Sync,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        // Extract the user from your auth system here
        // e.g., read a session cookie, validate a JWT, etc.
        todo!()
    }
}
```

### 3. Initialize and mount

```rust
use weaver::{Weaver, WeaverConfig};

#[tokio::main]
async fn main() {
    let pool = sqlx::PgPool::connect("postgres://...").await.unwrap();

    let weaver = Weaver::new(WeaverConfig {
        pool: pool.clone(),
        s3_bucket: None,           // Optional: for file attachments
        s3_prefix: "weaver/".into(),
        max_file_size: 10 * 1024 * 1024,
    }).await;

    // Run migrations (creates weaver_* tables)
    weaver.migrate().await.expect("Weaver migration failed");

    // Mount Weaver routes into your app
    let app = axum::Router::new()
        .nest("/api/weaver", weaver.router::<AppState, MyUser>())
        .with_state(app_state);

    // ...
}
```

### 4. Create a default channel for new projects

When you create a project in your app, call:

```rust
weaver.create_default_channel(project_id, &user_id).await?;
```

This creates a `#general` channel for the project.

## API Endpoints

All endpoints require an authenticated user (your `WeaverUser` extractor).

### Chat

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/projects/{project_id}/channels` | List channels |
| `POST` | `/projects/{project_id}/channels` | Create channel `{ "name": "..." }` |
| `DELETE` | `/channels/{channel_id}` | Delete channel |
| `GET` | `/channels/{channel_id}/messages?before=&limit=` | Message history |
| `POST` | `/channels/{channel_id}/upload` | Upload file (multipart) |
| `GET` | `/ws/{project_id}` | WebSocket connection |

**WebSocket Protocol:**

Client → Server:
```json
{ "channel_id": "uuid", "content": "Hello!", "attachment_ids": [] }
```

Server → Client:
```json
{ "type": "message", "id": "uuid", "channel_id": "uuid", "user_id": "...", "user_email": "...", "content": "Hello!", "created_at": "...", "attachments": [] }
```
```json
{ "type": "channel_created", "id": "uuid", "project_id": "uuid", "name": "new-channel" }
```

### Issues

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/projects/{project_id}/issues?status=&assignee=` | List issues |
| `POST` | `/projects/{project_id}/issues` | Create issue |
| `GET` | `/projects/{project_id}/board` | Kanban board (grouped by status) |
| `GET` | `/issues/{issue_id}` | Issue detail with labels + comments |
| `PATCH` | `/issues/{issue_id}` | Update issue |
| `DELETE` | `/issues/{issue_id}` | Delete issue |

### Labels

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/projects/{project_id}/labels` | List labels |
| `POST` | `/projects/{project_id}/labels` | Create label `{ "name": "...", "color": "#FF0000" }` |
| `DELETE` | `/labels/{label_id}` | Delete label |
| `POST` | `/issues/{issue_id}/labels` | Add label to issue `{ "label_id": "uuid" }` |
| `DELETE` | `/issues/{issue_id}/labels/{label_id}` | Remove label from issue |

### Comments

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/issues/{issue_id}/comments` | List comments |
| `POST` | `/issues/{issue_id}/comments` | Create comment `{ "content": "..." }` |

## Database Schema

All tables are prefixed with `weaver_` to avoid conflicts. Migrations are idempotent (`CREATE TABLE IF NOT EXISTS`).

**Tables:** `weaver_channels`, `weaver_messages`, `weaver_attachments`, `weaver_labels`, `weaver_issues`, `weaver_issue_labels`, `weaver_comments`

No foreign keys point to your application's tables — Weaver stores user IDs and emails as plain `TEXT` columns.

## S3 Configuration

File attachments require an S3-compatible bucket. Pass it via `WeaverConfig`:

```rust
use s3::{Bucket, Region};
use s3::creds::Credentials;

let region = Region::Custom {
    region: "us-east-1".into(),
    endpoint: "https://s3.example.com".into(),
};
let creds = Credentials::new(Some("key"), Some("secret"), None, None, None)?;
let bucket = Bucket::new("my-bucket", region, creds)?.with_path_style();

let config = WeaverConfig {
    pool,
    s3_bucket: Some(bucket),
    s3_prefix: "weaver/".into(),
    max_file_size: 10 * 1024 * 1024, // 10MB
};
```

If `s3_bucket` is `None`, file upload endpoints return `503 Service Unavailable`.

## Crate Structure

```
weaver/
├── src/
│   ├── lib.rs          # Public API: Weaver, WeaverConfig, WeaverUser trait
│   ├── config.rs       # Configuration struct
│   ├── migration.sql   # Embedded DDL for all tables
│   ├── chat/
│   │   ├── hub.rs      # Per-channel broadcast hub
│   │   ├── models.rs   # Channel, Message, Attachment types
│   │   ├── routes.rs   # REST endpoints
│   │   ├── ws.rs       # WebSocket handler
│   │   └── upload.rs   # File upload via S3
│   └── issues/
│       ├── models.rs   # Issue, Label, Comment types
│       └── routes.rs   # REST endpoints (CRUD, board, labels, comments)
```

## License

MIT
