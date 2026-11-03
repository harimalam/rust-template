use sqlx::PgPool;
use uuid::Uuid;

use super::dto::FileDto;

#[derive(Debug, Clone)]
pub struct FileRecord {
    pub id: Uuid,
    pub object_key: String,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<FileRecord> for FileDto {
    fn from(f: FileRecord) -> Self {
        Self {
            id: f.id,
            filename: f.filename,
            content_type: f.content_type,
            size_bytes: f.size_bytes,
            created_at: f.created_at,
        }
    }
}

pub async fn insert(
    db: &PgPool,
    id: Uuid,
    owner_id: Uuid,
    object_key: &str,
    filename: &str,
    content_type: &str,
    size_bytes: i64,
) -> Result<FileRecord, sqlx::Error> {
    sqlx::query_as!(
        FileRecord,
        "INSERT INTO files (id, owner_id, object_key, filename, content_type, size_bytes)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, object_key, filename, content_type, size_bytes, created_at",
        id,
        owner_id,
        object_key,
        filename,
        content_type,
        size_bytes,
    )
    .fetch_one(db)
    .await
}

pub async fn find(
    db: &PgPool,
    owner_id: Uuid,
    id: Uuid,
) -> Result<Option<FileRecord>, sqlx::Error> {
    sqlx::query_as!(
        FileRecord,
        "SELECT id, object_key, filename, content_type, size_bytes, created_at
         FROM files WHERE id = $1 AND owner_id = $2 AND deleted_at IS NULL",
        id,
        owner_id,
    )
    .fetch_optional(db)
    .await
}

pub async fn list(
    db: &PgPool,
    owner_id: Uuid,
    limit: i64,
    offset: i64,
) -> Result<(Vec<FileRecord>, i64), sqlx::Error> {
    let total = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "count!" FROM files WHERE owner_id = $1 AND deleted_at IS NULL"#,
        owner_id,
    )
    .fetch_one(db)
    .await?;
    let rows = sqlx::query_as!(
        FileRecord,
        "SELECT id, object_key, filename, content_type, size_bytes, created_at
         FROM files WHERE owner_id = $1 AND deleted_at IS NULL
         ORDER BY created_at DESC, id LIMIT $2 OFFSET $3",
        owner_id,
        limit,
        offset,
    )
    .fetch_all(db)
    .await?;
    Ok((rows, total))
}

pub async fn soft_delete(
    db: &PgPool,
    owner_id: Uuid,
    id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query!(
        "UPDATE files SET deleted_at = now()
         WHERE id = $1 AND owner_id = $2 AND deleted_at IS NULL
         RETURNING object_key",
        id,
        owner_id,
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.object_key))
}
