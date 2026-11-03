use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The note entity. Also the shape stored in the cache.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Note {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub title: String,
    pub body: String,
    pub pinned: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: Uuid,
    pub updated_by: Uuid,
}

/// Partial update: `None` leaves the field unchanged.
#[derive(Debug, Clone, Default)]
pub struct NotePatch {
    pub title: Option<String>,
    pub body: Option<String>,
    pub pinned: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct NoteFilter {
    pub pinned: Option<bool>,
}
