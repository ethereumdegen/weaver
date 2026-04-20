use axum::extract::{FromRequestParts, Path, Query};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::issues::models::*;
use crate::WeaverState;
use crate::WeaverUser;

pub fn router<S, U>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    U: WeaverUser + FromRequestParts<S>,
    <U as FromRequestParts<S>>::Rejection: Into<axum::response::Response>,
{
    Router::new()
        // Issues
        .route(
            "/projects/{project_id}/issues",
            get(list_issues::<U>).post(create_issue::<U>),
        )
        .route("/projects/{project_id}/board", get(board::<U>))
        .route(
            "/issues/{issue_id}",
            get(get_issue::<U>)
                .patch(update_issue::<U>)
                .delete(delete_issue::<U>),
        )
        // Labels
        .route(
            "/projects/{project_id}/labels",
            get(list_labels::<U>).post(create_label::<U>),
        )
        .route("/labels/{label_id}", delete(delete_label::<U>))
        // Issue labels
        .route(
            "/issues/{issue_id}/labels",
            post(add_issue_label::<U>),
        )
        .route(
            "/issues/{issue_id}/labels/{label_id}",
            delete(remove_issue_label::<U>),
        )
        // Comments
        .route(
            "/issues/{issue_id}/comments",
            get(list_comments::<U>).post(create_comment::<U>),
        )
}

// --- Issues ---

async fn list_issues<U: WeaverUser>(
    _user: U,
    Extension(state): Extension<WeaverState>,
    Path(project_id): Path<Uuid>,
    Query(query): Query<IssuesQuery>,
) -> Result<Json<Value>, StatusCode> {
    let mut issues = sqlx::query_as::<_, Issue>(
        "SELECT * FROM weaver_issues WHERE project_id = $1 ORDER BY number DESC",
    )
    .bind(project_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("[Weaver] list issues error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Apply filters in memory (simple approach for MVP)
    if let Some(ref status) = query.status {
        issues.retain(|i| &i.status == status);
    }
    if let Some(ref assignee) = query.assignee {
        issues.retain(|i| i.assignee_id.as_deref() == Some(assignee.as_str()));
    }

    Ok(Json(json!({ "issues": issues })))
}

async fn board<U: WeaverUser>(
    _user: U,
    Extension(state): Extension<WeaverState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let issues = sqlx::query_as::<_, Issue>(
        "SELECT * FROM weaver_issues WHERE project_id = $1 ORDER BY number",
    )
    .bind(project_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("[Weaver] board error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Group by status
    let statuses = ["backlog", "todo", "in_progress", "done", "cancelled"];
    let mut board = serde_json::Map::new();
    for status in statuses {
        let column: Vec<&Issue> = issues.iter().filter(|i| i.status == status).collect();
        board.insert(status.to_string(), json!(column));
    }

    Ok(Json(json!({ "board": board })))
}

async fn create_issue<U: WeaverUser>(
    user: U,
    Extension(state): Extension<WeaverState>,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<CreateIssue>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    if payload.title.is_empty() || payload.title.len() > 300 {
        return Err(StatusCode::BAD_REQUEST);
    }

    let priority = payload.priority.as_deref().unwrap_or("medium");
    if !["urgent", "high", "medium", "low"].contains(&priority) {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Auto-increment issue number
    let number: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(number), 0) + 1 FROM weaver_issues WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("[Weaver] issue number error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let issue = sqlx::query_as::<_, Issue>(
        "INSERT INTO weaver_issues (project_id, number, title, description, priority, assignee_id, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING *",
    )
    .bind(project_id)
    .bind(number)
    .bind(&payload.title)
    .bind(payload.description.as_deref().unwrap_or(""))
    .bind(priority)
    .bind(payload.assignee_id.as_deref())
    .bind(user.user_id())
    .fetch_one(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("[Weaver] create issue error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Add labels if provided
    if let Some(ref label_ids) = payload.label_ids {
        for lid in label_ids {
            let _ = sqlx::query(
                "INSERT INTO weaver_issue_labels (issue_id, label_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(issue.id)
            .bind(lid)
            .execute(&state.pool)
            .await;
        }
    }

    Ok((StatusCode::CREATED, Json(json!({ "issue": issue }))))
}

async fn get_issue<U: WeaverUser>(
    _user: U,
    Extension(state): Extension<WeaverState>,
    Path(issue_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let issue = sqlx::query_as::<_, Issue>(
        "SELECT * FROM weaver_issues WHERE id = $1",
    )
    .bind(issue_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("[Weaver] get issue error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .ok_or(StatusCode::NOT_FOUND)?;

    let labels = sqlx::query_as::<_, Label>(
        "SELECT l.* FROM weaver_labels l \
         JOIN weaver_issue_labels il ON il.label_id = l.id \
         WHERE il.issue_id = $1",
    )
    .bind(issue_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let comments = sqlx::query_as::<_, Comment>(
        "SELECT * FROM weaver_comments WHERE issue_id = $1 ORDER BY created_at",
    )
    .bind(issue_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let detail = IssueDetail {
        issue,
        labels,
        comments,
    };

    Ok(Json(json!({ "issue": detail })))
}

async fn update_issue<U: WeaverUser>(
    _user: U,
    Extension(state): Extension<WeaverState>,
    Path(issue_id): Path<Uuid>,
    Json(payload): Json<UpdateIssue>,
) -> Result<Json<Value>, StatusCode> {
    if let Some(ref title) = payload.title {
        if title.is_empty() || title.len() > 300 {
            return Err(StatusCode::BAD_REQUEST);
        }
        sqlx::query("UPDATE weaver_issues SET title = $1, updated_at = now() WHERE id = $2")
            .bind(title)
            .bind(issue_id)
            .execute(&state.pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if let Some(ref description) = payload.description {
        sqlx::query("UPDATE weaver_issues SET description = $1, updated_at = now() WHERE id = $2")
            .bind(description)
            .bind(issue_id)
            .execute(&state.pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if let Some(ref status) = payload.status {
        if !["backlog", "todo", "in_progress", "done", "cancelled"].contains(&status.as_str()) {
            return Err(StatusCode::BAD_REQUEST);
        }
        sqlx::query("UPDATE weaver_issues SET status = $1, updated_at = now() WHERE id = $2")
            .bind(status)
            .bind(issue_id)
            .execute(&state.pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if let Some(ref priority) = payload.priority {
        if !["urgent", "high", "medium", "low"].contains(&priority.as_str()) {
            return Err(StatusCode::BAD_REQUEST);
        }
        sqlx::query("UPDATE weaver_issues SET priority = $1, updated_at = now() WHERE id = $2")
            .bind(priority)
            .bind(issue_id)
            .execute(&state.pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if let Some(ref assignee_id) = payload.assignee_id {
        let assignee = if assignee_id.is_empty() {
            None
        } else {
            Some(assignee_id.as_str())
        };
        sqlx::query("UPDATE weaver_issues SET assignee_id = $1, updated_at = now() WHERE id = $2")
            .bind(assignee)
            .bind(issue_id)
            .execute(&state.pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    Ok(Json(json!({ "success": true })))
}

async fn delete_issue<U: WeaverUser>(
    _user: U,
    Extension(state): Extension<WeaverState>,
    Path(issue_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    sqlx::query("DELETE FROM weaver_issues WHERE id = $1")
        .bind(issue_id)
        .execute(&state.pool)
        .await
        .map_err(|e| {
            tracing::error!("[Weaver] delete issue error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(json!({ "success": true })))
}

// --- Labels ---

async fn list_labels<U: WeaverUser>(
    _user: U,
    Extension(state): Extension<WeaverState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let labels = sqlx::query_as::<_, Label>(
        "SELECT * FROM weaver_labels WHERE project_id = $1 ORDER BY name",
    )
    .bind(project_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("[Weaver] list labels error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(json!({ "labels": labels })))
}

async fn create_label<U: WeaverUser>(
    _user: U,
    Extension(state): Extension<WeaverState>,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<CreateLabel>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    if payload.name.is_empty() || payload.name.len() > 50 {
        return Err(StatusCode::BAD_REQUEST);
    }

    let color = payload.color.as_deref().unwrap_or("#6B7280");

    let label = sqlx::query_as::<_, Label>(
        "INSERT INTO weaver_labels (project_id, name, color) VALUES ($1, $2, $3) RETURNING *",
    )
    .bind(project_id)
    .bind(&payload.name)
    .bind(color)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("[Weaver] create label error: {e}");
        StatusCode::CONFLICT
    })?;

    Ok((StatusCode::CREATED, Json(json!({ "label": label }))))
}

async fn delete_label<U: WeaverUser>(
    _user: U,
    Extension(state): Extension<WeaverState>,
    Path(label_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    sqlx::query("DELETE FROM weaver_labels WHERE id = $1")
        .bind(label_id)
        .execute(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({ "success": true })))
}

// --- Issue Labels ---

async fn add_issue_label<U: WeaverUser>(
    _user: U,
    Extension(state): Extension<WeaverState>,
    Path(issue_id): Path<Uuid>,
    Json(payload): Json<AddLabel>,
) -> Result<Json<Value>, StatusCode> {
    sqlx::query(
        "INSERT INTO weaver_issue_labels (issue_id, label_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(issue_id)
    .bind(payload.label_id)
    .execute(&state.pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({ "success": true })))
}

async fn remove_issue_label<U: WeaverUser>(
    _user: U,
    Extension(state): Extension<WeaverState>,
    Path((issue_id, label_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, StatusCode> {
    sqlx::query("DELETE FROM weaver_issue_labels WHERE issue_id = $1 AND label_id = $2")
        .bind(issue_id)
        .bind(label_id)
        .execute(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({ "success": true })))
}

// --- Comments ---

async fn list_comments<U: WeaverUser>(
    _user: U,
    Extension(state): Extension<WeaverState>,
    Path(issue_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let comments = sqlx::query_as::<_, Comment>(
        "SELECT * FROM weaver_comments WHERE issue_id = $1 ORDER BY created_at",
    )
    .bind(issue_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("[Weaver] list comments error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(json!({ "comments": comments })))
}

async fn create_comment<U: WeaverUser>(
    user: U,
    Extension(state): Extension<WeaverState>,
    Path(issue_id): Path<Uuid>,
    Json(payload): Json<CreateComment>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    if payload.content.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let comment = sqlx::query_as::<_, Comment>(
        "INSERT INTO weaver_comments (issue_id, user_id, user_email, content) VALUES ($1, $2, $3, $4) RETURNING *",
    )
    .bind(issue_id)
    .bind(user.user_id())
    .bind(user.email())
    .bind(&payload.content)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("[Weaver] create comment error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok((StatusCode::CREATED, Json(json!({ "comment": comment }))))
}
