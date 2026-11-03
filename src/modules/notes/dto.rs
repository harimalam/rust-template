use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::domain::{Note, NotePatch};

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateNoteRequest {
    #[validate(length(min = 1, max = 200, message = "must be between 1 and 200 characters"))]
    pub title: String,
    #[serde(default)]
    #[validate(length(max = 20000, message = "must be at most 20000 characters"))]
    pub body: String,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateNoteRequest {
    #[validate(length(min = 1, max = 200, message = "must be between 1 and 200 characters"))]
    pub title: Option<String>,
    #[validate(length(max = 20000, message = "must be at most 20000 characters"))]
    pub body: Option<String>,
    pub pinned: Option<bool>,
}

impl From<UpdateNoteRequest> for NotePatch {
    fn from(req: UpdateNoteRequest) -> Self {
        Self {
            title: req.title,
            body: req.body,
            pinned: req.pinned,
        }
    }
}

/// Extra list filters (pagination / sort / search come from the shared `ListQuery`).
#[derive(Debug, Deserialize, IntoParams)]
pub struct NoteFilterQuery {
    /// Only pinned (`true`) or unpinned (`false`) notes.
    pub pinned: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NoteDto {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    pub pinned: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Note> for NoteDto {
    fn from(n: Note) -> Self {
        Self {
            id: n.id,
            title: n.title,
            body: n.body,
            pinned: n.pinned,
            created_at: n.created_at,
            updated_at: n.updated_at,
        }
    }
}
