//! Pagination, sorting and search query parameters shared by every list endpoint.

use serde::Deserialize;
use utoipa::IntoParams;

use super::error::AppError;

pub const DEFAULT_PER_PAGE: u32 = 20;
pub const MAX_PER_PAGE: u32 = 100;

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListQuery {
    /// 1-based page number (default 1).
    pub page: Option<u32>,
    /// Items per page (default 20, max 100).
    pub per_page: Option<u32>,
    /// Sort field; prefix with `-` for descending, e.g. `-created_at`.
    pub sort: Option<String>,
    /// Free-text search.
    pub q: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct Page {
    pub page: u32,
    pub per_page: u32,
}

impl Page {
    pub fn limit(&self) -> i64 {
        i64::from(self.per_page)
    }

    pub fn offset(&self) -> i64 {
        i64::from(self.page - 1) * i64::from(self.per_page)
    }
}

/// A validated sort: `column` is always taken from the caller's whitelist, never from user input,
/// so it is safe to interpolate into SQL.
#[derive(Debug, Clone, Copy)]
pub struct Sort {
    pub column: &'static str,
    pub descending: bool,
}

impl Sort {
    pub fn direction(&self) -> &'static str {
        if self.descending { "DESC" } else { "ASC" }
    }
}

impl ListQuery {
    pub fn page(&self) -> Result<Page, AppError> {
        let page = self.page.unwrap_or(1);
        let per_page = self.per_page.unwrap_or(DEFAULT_PER_PAGE);
        if page == 0 {
            return Err(AppError::BadRequest("page must be >= 1".into()));
        }
        if per_page == 0 || per_page > MAX_PER_PAGE {
            return Err(AppError::BadRequest(format!(
                "per_page must be between 1 and {MAX_PER_PAGE}"
            )));
        }
        Ok(Page { page, per_page })
    }

    /// Resolves `sort` against `allowed` (API field name -> SQL column).
    pub fn sort(&self, allowed: &[(&str, &'static str)], default: Sort) -> Result<Sort, AppError> {
        let Some(raw) = self.sort.as_deref().filter(|s| !s.is_empty()) else {
            return Ok(default);
        };
        let (descending, field) = match raw.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, raw.strip_prefix('+').unwrap_or(raw)),
        };
        allowed
            .iter()
            .find(|(name, _)| *name == field)
            .map(|(_, column)| Sort { column, descending })
            .ok_or_else(|| {
                let names: Vec<&str> = allowed.iter().map(|(n, _)| *n).collect();
                AppError::BadRequest(format!(
                    "cannot sort by {field:?}; allowed: {}",
                    names.join(", ")
                ))
            })
    }

    /// Search term trimmed, with LIKE wildcards escaped; `None` when empty.
    pub fn search_pattern(&self) -> Option<String> {
        let q = self.q.as_deref()?.trim();
        if q.is_empty() {
            return None;
        }
        let escaped = q
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        Some(format!("%{escaped}%"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALLOWED: &[(&str, &str)] = &[("created_at", "created_at"), ("title", "title")];
    const DEFAULT: Sort = Sort {
        column: "created_at",
        descending: true,
    };

    fn query(sort: Option<&str>) -> ListQuery {
        ListQuery {
            page: None,
            per_page: None,
            sort: sort.map(Into::into),
            q: None,
        }
    }

    #[test]
    fn defaults_and_offsets() {
        let p = ListQuery {
            page: Some(3),
            per_page: Some(10),
            sort: None,
            q: None,
        }
        .page()
        .unwrap();
        assert_eq!((p.limit(), p.offset()), (10, 20));
    }

    #[test]
    fn rejects_bad_page_sizes() {
        assert!(
            ListQuery {
                page: Some(0),
                per_page: None,
                sort: None,
                q: None
            }
            .page()
            .is_err()
        );
        assert!(
            ListQuery {
                page: None,
                per_page: Some(101),
                sort: None,
                q: None
            }
            .page()
            .is_err()
        );
    }

    #[test]
    fn sort_is_whitelisted() {
        let s = query(Some("-title")).sort(ALLOWED, DEFAULT).unwrap();
        assert_eq!((s.column, s.descending), ("title", true));
        assert!(
            query(Some("password_hash; DROP TABLE users"))
                .sort(ALLOWED, DEFAULT)
                .is_err()
        );
        assert_eq!(
            query(None).sort(ALLOWED, DEFAULT).unwrap().column,
            "created_at"
        );
    }

    #[test]
    fn search_escapes_wildcards() {
        let q = ListQuery {
            page: None,
            per_page: None,
            sort: None,
            q: Some(" 50%_off ".into()),
        };
        assert_eq!(q.search_pattern().unwrap(), "%50\\%\\_off%");
    }
}
