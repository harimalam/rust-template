use uuid::Uuid;

use super::{
    dto::{DownloadUrlDto, FileDto},
    repository, sniff,
};
use crate::{
    bootstrap::state::AppState,
    http::{AppError, AppResult, pagination::ListQuery, response::PageMeta},
};

/// Last path component only, no control characters, bounded length.
pub fn sanitize_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().filter(|c| !c.is_control()).take(255).collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_owned();
    if cleaned.is_empty() {
        "file".to_owned()
    } else {
        cleaned
    }
}

pub async fn upload(
    state: &AppState,
    owner_id: Uuid,
    filename: &str,
    declared_type: &str,
    data: Vec<u8>,
) -> AppResult<FileDto> {
    let s3 = &state.settings.s3;
    if data.is_empty() {
        return Err(AppError::BadRequest("uploaded file is empty".into()));
    }
    if data.len() > s3.max_upload_bytes {
        return Err(AppError::PayloadTooLarge);
    }

    // Drop parameters like "; charset=utf-8" before comparing.
    let declared = declared_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if !s3
        .allowed_mime_types
        .iter()
        .any(|t| t.eq_ignore_ascii_case(&declared))
    {
        return Err(AppError::UnsupportedMediaType(format!(
            "content type {declared:?} is not allowed; allowed: {}",
            s3.allowed_mime_types.join(", ")
        )));
    }
    if sniff::detect(&data) != Some(declared.as_str()) {
        return Err(AppError::UnsupportedMediaType(
            "file contents do not match the declared content type".into(),
        ));
    }

    let id = Uuid::now_v7();
    // The key never contains user-supplied text, so it can't traverse or collide.
    let object_key = format!("{owner_id}/{id}");
    let size = data.len() as i64;
    state
        .storage
        .put(&object_key, &declared, data)
        .await
        .map_err(AppError::internal)?;

    match repository::insert(
        &state.db,
        id,
        owner_id,
        &object_key,
        &sanitize_filename(filename),
        &declared,
        size,
    )
    .await
    {
        Ok(record) => Ok(record.into()),
        Err(err) => {
            // Don't leave an orphaned object behind.
            if let Err(cleanup) = state.storage.delete(&object_key).await {
                tracing::warn!(error = ?cleanup, %object_key, "failed to clean up orphaned object");
            }
            Err(err.into())
        }
    }
}

pub async fn list(
    state: &AppState,
    owner_id: Uuid,
    query: &ListQuery,
) -> AppResult<(Vec<FileDto>, PageMeta)> {
    let page = query.page()?;
    let (rows, total) = repository::list(&state.db, owner_id, page.limit(), page.offset()).await?;
    Ok((
        rows.into_iter().map(Into::into).collect(),
        PageMeta::new(page.page, page.per_page, total),
    ))
}

pub async fn download_url(state: &AppState, owner_id: Uuid, id: Uuid) -> AppResult<DownloadUrlDto> {
    let record = repository::find(&state.db, owner_id, id)
        .await?
        .ok_or(AppError::NotFound("file"))?;
    let ttl = state.settings.s3.presign_ttl;
    let url = state
        .storage
        .presigned_get_url(&record.object_key, &record.filename, ttl)
        .await
        .map_err(AppError::internal)?;
    Ok(DownloadUrlDto {
        url,
        expires_in: ttl.as_secs(),
    })
}

pub async fn delete(state: &AppState, owner_id: Uuid, id: Uuid) -> AppResult<()> {
    let key = repository::soft_delete(&state.db, owner_id, id)
        .await?
        .ok_or(AppError::NotFound("file"))?;
    // Metadata is already gone from the user's view; a failed object delete is only a leak,
    // so log it rather than failing the request.
    if let Err(err) = state.storage.delete(&key).await {
        tracing::warn!(error = ?err, object_key = %key, "failed to delete object from storage");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::sanitize_filename;

    #[test]
    fn strips_paths_and_dots() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename(r"C:\Users\me\photo.png"), "photo.png");
        assert_eq!(sanitize_filename(".hidden"), "hidden");
        assert_eq!(sanitize_filename("   "), "file");
        assert_eq!(sanitize_filename("a\nb.txt"), "ab.txt");
    }
}
