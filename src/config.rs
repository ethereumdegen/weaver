use sqlx::PgPool;

pub struct WeaverConfig {
    pub pool: PgPool,
    pub s3_bucket: Option<Box<s3::Bucket>>,
    pub s3_prefix: String,
    pub max_file_size: usize,
}

impl WeaverConfig {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            s3_bucket: None,
            s3_prefix: "weaver/".into(),
            max_file_size: 10 * 1024 * 1024,
        }
    }
}
