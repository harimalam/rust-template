use sqlx::{PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use super::domain::{Note, NoteFilter, NotePatch};
use crate::http::pagination::{Page, Sort};

pub const SORTABLE: &[(&str, &str)] = &[
    ("created_at", "created_at"),
    ("updated_at", "updated_at"),
    ("title", "title"),
];

pub async fn insert(
    db: &PgPool,
    id: Uuid,
    owner_id: Uuid,
    title: &str,
    body: &str,
    pinned: bool,
) -> Result<Note, sqlx::Error> {
    sqlx::query_as!(
        Note,
        "INSERT INTO notes (id, owner_id, title, body, pinned, created_by, updated_by)
         VALUES ($1, $2, $3, $4, $5, $2, $2)
         RETURNING id, owner_id, title, body, pinned, created_at, updated_at, created_by, updated_by",
        id,
        owner_id,
        title,
        body,
        pinned,
    )
    .fetch_one(db)
    .await
}

/// Scoped by owner in SQL: other users' notes are indistinguishable from missing ones.
pub async fn find(db: &PgPool, owner_id: Uuid, id: Uuid) -> Result<Option<Note>, sqlx::Error> {
    sqlx::query_as!(
        Note,
        "SELECT id, owner_id, title, body, pinned, created_at, updated_at, created_by, updated_by
         FROM notes WHERE id = $1 AND owner_id = $2 AND deleted_at IS NULL",
        id,
        owner_id,
    )
    .fetch_optional(db)
    .await
}

pub async fn update(
    db: &PgPool,
    owner_id: Uuid,
    id: Uuid,
    patch: &NotePatch,
) -> Result<Option<Note>, sqlx::Error> {
    sqlx::query_as!(
        Note,
        "UPDATE notes SET
            title = COALESCE($3, title),
            body = COALESCE($4, body),
            pinned = COALESCE($5, pinned),
            updated_at = now(),
            updated_by = $2
         WHERE id = $1 AND owner_id = $2 AND deleted_at IS NULL
         RETURNING id, owner_id, title, body, pinned, created_at, updated_at, created_by, updated_by",
        id,
        owner_id,
        patch.title,
        patch.body,
        patch.pinned,
    )
    .fetch_optional(db)
    .await
}

pub async fn soft_delete(db: &PgPool, owner_id: Uuid, id: Uuid) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE notes SET deleted_at = now(), updated_at = now(), updated_by = $2
         WHERE id = $1 AND owner_id = $2 AND deleted_at IS NULL",
        id,
        owner_id,
    )
    .execute(db)
    .await?;
    Ok(result.rows_affected() > 0)
}

fn push_filters(
    query: &mut QueryBuilder<Postgres>,
    owner_id: Uuid,
    filter: &NoteFilter,
    search: Option<&str>,
) {
    query.push(" WHERE owner_id = ").push_bind(owner_id);
    query.push(" AND deleted_at IS NULL");
    if let Some(pinned) = filter.pinned {
        query.push(" AND pinned = ").push_bind(pinned);
    }
    if let Some(pattern) = search {
        query
            .push(" AND (title ILIKE ")
            .push_bind(pattern.to_owned());
        query.push(" OR body ILIKE ").push_bind(pattern.to_owned());
        query.push(")");
    }
}

pub async fn list(
    db: &PgPool,
    owner_id: Uuid,
    filter: &NoteFilter,
    search: Option<&str>,
    page: Page,
    sort: Sort,
) -> Result<(Vec<Note>, i64), sqlx::Error> {
    let mut count = QueryBuilder::new("SELECT COUNT(*) FROM notes");
    push_filters(&mut count, owner_id, filter, search);
    let total: i64 = count.build_query_scalar().fetch_one(db).await?;

    let mut query = QueryBuilder::new(
        "SELECT id, owner_id, title, body, pinned, created_at, updated_at, created_by, updated_by FROM notes",
    );
    push_filters(&mut query, owner_id, filter, search);
    // `sort.column` comes from the static SORTABLE whitelist, never from user input.
    query.push(format!(
        " ORDER BY {} {}, id",
        sort.column,
        sort.direction()
    ));
    query.push(" LIMIT ").push_bind(page.limit());
    query.push(" OFFSET ").push_bind(page.offset());

    let notes = query.build_query_as::<Note>().fetch_all(db).await?;
    Ok((notes, total))
}

/// Permanently removes notes soft-deleted more than `retention_days` ago (cron job).
pub async fn purge_deleted(db: &PgPool, retention_days: i32) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM notes WHERE deleted_at IS NOT NULL AND deleted_at < now() - make_interval(days => $1)",
        retention_days,
    )
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}
