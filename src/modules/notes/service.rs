//! Note business rules: ownership, cache-aside reads, cache invalidation on writes.

use uuid::Uuid;

use super::{
    domain::{Note, NoteFilter, NotePatch},
    dto::CreateNoteRequest,
    repository,
};
use crate::{
    bootstrap::state::AppState,
    http::{
        AppError, AppResult,
        pagination::{ListQuery, Sort},
        response::PageMeta,
    },
};

/// Cache key includes the owner, so one user's cached entry can never serve another user.
fn cache_key(owner_id: Uuid, id: Uuid) -> String {
    format!("notes:{owner_id}:{id}")
}

pub async fn create(state: &AppState, owner_id: Uuid, req: CreateNoteRequest) -> AppResult<Note> {
    Ok(repository::insert(
        &state.db,
        Uuid::now_v7(),
        owner_id,
        req.title.trim(),
        &req.body,
        req.pinned,
    )
    .await?)
}

/// Cache-aside read. The owner is re-checked after a cache hit as defence in depth.
pub async fn get(state: &AppState, owner_id: Uuid, id: Uuid) -> AppResult<Note> {
    let key = cache_key(owner_id, id);
    if let Some(note) = state.cache.get_json::<Note>(&key).await
        && note.owner_id == owner_id
        && note.id == id
    {
        return Ok(note);
    }
    let note = repository::find(&state.db, owner_id, id)
        .await?
        .ok_or(AppError::NotFound("note"))?;
    state.cache.set_json(&key, &note, None).await;
    Ok(note)
}

pub async fn update(
    state: &AppState,
    owner_id: Uuid,
    id: Uuid,
    mut patch: NotePatch,
) -> AppResult<Note> {
    patch.title = patch.title.map(|t| t.trim().to_owned());
    let note = repository::update(&state.db, owner_id, id, &patch)
        .await?
        .ok_or(AppError::NotFound("note"))?;
    state.cache.delete(&cache_key(owner_id, id)).await;
    Ok(note)
}

pub async fn delete(state: &AppState, owner_id: Uuid, id: Uuid) -> AppResult<()> {
    if !repository::soft_delete(&state.db, owner_id, id).await? {
        return Err(AppError::NotFound("note"));
    }
    state.cache.delete(&cache_key(owner_id, id)).await;
    Ok(())
}

pub async fn list(
    state: &AppState,
    owner_id: Uuid,
    query: &ListQuery,
    filter: &NoteFilter,
) -> AppResult<(Vec<Note>, PageMeta)> {
    let page = query.page()?;
    let sort = query.sort(
        repository::SORTABLE,
        Sort {
            column: "created_at",
            descending: true,
        },
    )?;
    let search = query.search_pattern();
    let (notes, total) =
        repository::list(&state.db, owner_id, filter, search.as_deref(), page, sort).await?;
    Ok((notes, PageMeta::new(page.page, page.per_page, total)))
}
