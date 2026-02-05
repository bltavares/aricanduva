use chrono::NaiveDateTime;
use futures::TryFutureExt;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;
use std::{fs, time::Duration};
use thiserror::Error;
use tracing::Instrument;
use typed_path::{UnixPath, UnixPathBuf};

use crate::cli;

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("SQL error: {0}")]
    SqlxError(#[from] sqlx::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Database initialization failed")]
    InitializationFailed(#[from] sqlx::migrate::MigrateError),
}

#[derive(Clone)]
pub struct Database {
    pub pool: SqlitePool,
}

pub struct MetadataResponse {
    pub cid: String,
    pub size: i64,
    pub content_type: String,
    pub key: String,
    pub bucket: String,
    pub updated_at: NaiveDateTime,
}

impl Database {
    async fn new_with_config(
        database_url: &str,
        config: &cli::SqliteConfig,
    ) -> Result<Self, sqlx::Error> {
        let options = SqliteConnectOptions::from_str(database_url)?
            .create_if_missing(true)
            .auto_vacuum(config.auto_vacuum.unwrap_or_default())
            .journal_mode(config.journal_mode.unwrap_or_default())
            .synchronous(config.synchronous.unwrap_or_default())
            .busy_timeout(Duration::from_secs(30))
            .optimize_on_close(true, None);
        let pool = SqlitePoolOptions::new()
            .acquire_timeout(Duration::from_secs(1))
            .max_connections(8)
            .connect_with(options)
            .await?;
        Ok(Self { pool })
    }

    /// Initialize the database by ensuring it exists and running migrations
    pub async fn initialize(
        db_path: &Path,
        config: &cli::SqliteConfig,
    ) -> Result<Self, DatabaseError> {
        // Ensure the database file exists
        if !db_path.exists() {
            tracing::info!(db = ?db_path, "Database file not found at and will be created");
            // Create parent directories if they don't exist
            if let Some(parent) = db_path.parent() {
                fs::create_dir_all(parent)?;
            }
        }

        let database_url = format!("sqlite:{}", db_path.display());
        tracing::info!(db = ?db_path, "Initializing database");

        let db = Self::new_with_config(&database_url, config)
            .inspect_ok(|_| tracing::trace!("connected to database"))
            .await?;

        // Run migrations
        tracing::info!("Running migrations...");
        sqlx::migrate!("./migrations")
            .run(&db.pool)
            .inspect_ok(|()| tracing::trace!("applied migrations"))
            .await?;

        tracing::info!("Database initialized successfully");
        Ok(db)
    }

    pub async fn ping(&self) -> bool {
        let result = sqlx::query_scalar!("select 1 from _sqlx_migrations limit 1")
            .fetch_optional(&self.pool)
            .await;
        result.is_ok()
    }

    /// Store metadata for an S3 object
    pub async fn store_object_metadata(
        &self,
        bucket: &str,
        key: &str,
        cid: &str,
        size: i64,
        content_type: &str,
    ) -> Result<(), DatabaseError> {
        sqlx::query!(
            "INSERT INTO metadata (cid, bucket, object_key, content_type, size) VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT DO UPDATE SET cid = excluded.cid, size = excluded.size, content_type = excluded.content_type, updated_at = excluded.updated_at",
            cid,
            bucket,
            key,
            content_type,
            size
        )
        .execute(&self.pool)
        .inspect_ok(|_| tracing::trace!("stored metadata"))
        .instrument(tracing::debug_span!("store metadata", key))
        .await
        ?;

        Ok(())
    }

    /// Retrieve metadata for an S3 object
    pub async fn get_object_metadata(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<MetadataResponse>, DatabaseError> {
        let record = sqlx::query_as!(
            MetadataResponse,
            r#"SELECT cid, size, content_type, bucket, object_key as key, updated_at FROM metadata WHERE bucket = ? AND object_key = ?"#,
            bucket,
            key
        )
        .fetch_optional(&self.pool)
        .inspect_ok(|_| tracing::trace!("retrieved"))
        .instrument(tracing::debug_span!("get object", key))
        .await?;

        Ok(record)
    }

    /// Delete metadata for an S3 object
    pub async fn delete_object(&self, metadata: &MetadataResponse) -> Result<(), DatabaseError> {
        sqlx::query!(
            "DELETE FROM metadata WHERE bucket = ? AND object_key = ?",
            metadata.bucket,
            metadata.key
        )
        .execute(&self.pool)
        .inspect_ok(|_| tracing::trace!("deleted"))
        .instrument(tracing::debug_span!("delete object", key = metadata.key))
        .await?;

        Ok(())
    }

    /// Count how many objects reference a CID
    pub async fn cid_count(&self, cid: &str) -> Result<i64, DatabaseError> {
        let count = sqlx::query_scalar!("SELECT COUNT(1) FROM metadata WHERE cid = ?", cid)
            .fetch_one(&self.pool)
            .inspect_ok(|total| tracing::debug!(total, "Stored CID count"))
            .instrument(tracing::debug_span!("counting", cid))
            .await?;

        Ok(count)
    }

    /// Find the shallowest removable directory path from a deleted object's path.
    /// Returns the shallowest directory that can be safely removed (i.e., no other objects exist in it).
    pub async fn find_shallowest_removable_directory(
        &self,
        bucket: &str,
        path: &str,
    ) -> Result<Option<UnixPathBuf>, DatabaseError> {
        let mut shallow = None;

        // TODO figure out how to do it all in SQLLite SQL to avoid N+1 queries on deep removals
        // But Sqlite has lots of missing features (CTE, split_part, reverse) that makes it hard
        // Maybe something with json_each?
        for ancestor in UnixPath::new(path)
            .ancestors()
            .filter(|&f| !f.to_string_lossy().is_empty())
        {
            let like = format!("{ancestor}/%");
            let result = sqlx::query_scalar!(
                r#"
SELECT count(1) FROM metadata where bucket = ? and object_key LIKE ?;
        "#,
                bucket,
                like,
            )
            .fetch_one(&self.pool)
            .inspect_ok(|i| tracing::debug!(total = i, "Found entries"))
            .instrument(tracing::debug_span!(
                "searching for pattern",
                bucket,
                pattern = like,
            ))
            .await?;

            if result == 0 {
                shallow = Some(ancestor);
            } else {
                break;
            }
        }

        Ok(shallow.map(UnixPath::to_owned))
    }
}
