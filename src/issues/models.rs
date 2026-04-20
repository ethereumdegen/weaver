use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Issue {
    pub id: Uuid,
    pub project_id: Uuid,
    pub number: i32,
    pub title: String,
    pub description: String,
    pub status: String,
    pub priority: String,
    pub assignee_id: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Label {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub color: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Comment {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub user_id: String,
    pub user_email: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct IssueWithLabels {
    #[serde(flatten)]
    pub issue: Issue,
    pub labels: Vec<Label>,
}

#[derive(Debug, Serialize)]
pub struct IssueDetail {
    #[serde(flatten)]
    pub issue: Issue,
    pub labels: Vec<Label>,
    pub comments: Vec<Comment>,
}

#[derive(Debug, Deserialize)]
pub struct CreateIssue {
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<String>,
    pub assignee_id: Option<String>,
    pub label_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateIssue {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub priority: Option<String>,
    pub assignee_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateLabel {
    pub name: String,
    pub color: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateComment {
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct AddLabel {
    pub label_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct IssuesQuery {
    pub status: Option<String>,
    pub assignee: Option<String>,
    pub label: Option<String>,
}
