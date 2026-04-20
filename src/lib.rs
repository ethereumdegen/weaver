pub mod config;
pub mod chat;
pub mod issues;

pub use config::WeaverConfig;

use axum::extract::FromRequestParts;
use axum::Router;
use chat::hub::Hub;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// Trait host app implements to provide user identity.
/// Weaver never touches auth directly.
pub trait WeaverUser: Send + Sync + 'static {
    fn user_id(&self) -> &str;
    fn email(&self) -> &str;
}

/// Shared state accessible to Weaver routes via Extension.
#[derive(Clone)]
pub struct WeaverState {
    pub pool: PgPool,
    pub hub: Arc<Hub>,
    pub s3_bucket: Option<Box<s3::Bucket>>,
    pub s3_prefix: String,
    pub max_file_size: usize,
}

pub struct Weaver {
    pool: PgPool,
    hub: Arc<Hub>,
    s3_bucket: Option<Box<s3::Bucket>>,
    s3_prefix: String,
    max_file_size: usize,
}

impl Weaver {
    pub async fn new(config: WeaverConfig) -> Self {
        Self {
            pool: config.pool,
            hub: Arc::new(Hub::new()),
            s3_bucket: config.s3_bucket,
            s3_prefix: config.s3_prefix,
            max_file_size: config.max_file_size,
        }
    }

    pub async fn migrate(&self) -> Result<(), sqlx::Error> {
        let sql = include_str!("migration.sql");
        sqlx::raw_sql(sql).execute(&self.pool).await?;
        tracing::info!("[Weaver] Migration complete");
        Ok(())
    }

    /// Returns an Axum router with all chat + issues endpoints.
    /// S = your app state, U = your WeaverUser extractor.
    pub fn router<S, U>(&self) -> Router<S>
    where
        S: Clone + Send + Sync + 'static,
        U: WeaverUser + FromRequestParts<S>,
        <U as FromRequestParts<S>>::Rejection: Into<axum::response::Response>,
    {
        let state = WeaverState {
            pool: self.pool.clone(),
            hub: self.hub.clone(),
            s3_bucket: self.s3_bucket.clone(),
            s3_prefix: self.s3_prefix.clone(),
            max_file_size: self.max_file_size,
        };

        let chat_routes = chat::routes::router::<S, U>();
        let ws_routes = chat::ws::router::<S, U>();
        let upload_routes = chat::upload::router::<S, U>();
        let issue_routes = issues::routes::router::<S, U>();

        Router::new()
            .merge(chat_routes)
            .merge(ws_routes)
            .merge(upload_routes)
            .merge(issue_routes)
            .layer(axum::Extension(state))
    }

    /// Create default #general channel for a project.
    pub async fn create_default_channel(
        &self,
        project_id: Uuid,
        user_id: &str,
    ) -> Result<chat::models::Channel, sqlx::Error> {
        let channel = sqlx::query_as::<_, chat::models::Channel>(
            "INSERT INTO weaver_channels (project_id, name, created_by) VALUES ($1, 'general', $2) \
             ON CONFLICT (project_id, name) DO UPDATE SET name = 'general' \
             RETURNING *"
        )
        .bind(project_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(channel)
    }
}
