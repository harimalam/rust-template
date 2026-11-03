use std::collections::HashMap;

use sqlx::{PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use crate::{
    http::pagination::{Page, Sort},
    modules::auth::repository::UserRecord,
};

pub const SORTABLE: &[(&str, &str)] = &[
    ("created_at", "created_at"),
    ("email", "email"),
    ("full_name", "full_name"),
];

fn push_filters(query: &mut QueryBuilder<Postgres>, search: Option<&str>) {
    query.push(" WHERE deleted_at IS NULL");
    if let Some(pattern) = search {
        query
            .push(" AND (email::text ILIKE ")
            .push_bind(pattern.to_owned());
        query
            .push(" OR full_name ILIKE ")
            .push_bind(pattern.to_owned());
        query.push(")");
    }
}

pub async fn list(
    db: &PgPool,
    page: Page,
    sort: Sort,
    search: Option<&str>,
) -> Result<(Vec<UserRecord>, i64), sqlx::Error> {
    let mut count = QueryBuilder::new("SELECT COUNT(*) FROM users");
    push_filters(&mut count, search);
    let total: i64 = count.build_query_scalar().fetch_one(db).await?;

    let mut query = QueryBuilder::new(
        "SELECT id, email::text AS email, password_hash, full_name, email_verified_at, is_active, created_at FROM users",
    );
    push_filters(&mut query, search);
    // `sort.column` comes from a static whitelist, never from user input.
    query.push(format!(
        " ORDER BY {} {}, id",
        sort.column,
        sort.direction()
    ));
    query.push(" LIMIT ").push_bind(page.limit());
    query.push(" OFFSET ").push_bind(page.offset());

    let rows = query
        .build()
        .try_map(|row: sqlx::postgres::PgRow| {
            use sqlx::Row;
            Ok(UserRecord {
                id: row.try_get("id")?,
                email: row.try_get("email")?,
                password_hash: row.try_get("password_hash")?,
                full_name: row.try_get("full_name")?,
                email_verified_at: row.try_get("email_verified_at")?,
                is_active: row.try_get("is_active")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .fetch_all(db)
        .await?;
    Ok((rows, total))
}

pub async fn roles_for_users(
    db: &PgPool,
    user_ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<String>>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT ur.user_id AS "user_id!", r.name AS "name!"
           FROM user_roles ur JOIN roles r ON r.id = ur.role_id
           WHERE ur.user_id = ANY($1) ORDER BY r.name"#,
        user_ids,
    )
    .fetch_all(db)
    .await?;
    let mut map: HashMap<Uuid, Vec<String>> = HashMap::new();
    for row in rows {
        map.entry(row.user_id).or_default().push(row.name);
    }
    Ok(map)
}

pub async fn update_full_name(
    db: &PgPool,
    user_id: Uuid,
    full_name: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE users SET full_name = $2, updated_at = now() WHERE id = $1 AND deleted_at IS NULL",
        user_id,
        full_name,
    )
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

/// Replaces the user's roles. Returns the number of roles applied (fewer than requested means
/// at least one name didn't exist).
pub async fn replace_roles(
    tx: &mut sqlx::PgConnection,
    user_id: Uuid,
    role_names: &[String],
) -> Result<u64, sqlx::Error> {
    sqlx::query!("DELETE FROM user_roles WHERE user_id = $1", user_id)
        .execute(&mut *tx)
        .await?;
    let result = sqlx::query!(
        "INSERT INTO user_roles (user_id, role_id) SELECT $1, id FROM roles WHERE name = ANY($2)",
        user_id,
        role_names,
    )
    .execute(&mut *tx)
    .await?;
    Ok(result.rows_affected())
}
