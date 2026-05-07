//! Public corpus suggestions / uploads.
//!
//! Anyone can POST a multipart form to `/api/suggestions` with an image file,
//! the page they got it from, license declaration, and (mandatory) email.
//! When the uploader is signed in via the magic-link flow, we read the
//! observer's verified email from the auth row and pre-fill it; the form may
//! still send a different one (we record both).
//!
//! On successful submission we email a notification to
//! `SUGGESTION_NOTIFY_EMAIL` (defaults to `genandlilith@gmail.com`) so the
//! reviewer sees every new submission as it arrives. The send is fire-and-
//! forget — Resend outages don't fail the upload.
//!
//! Bytes live forever under `SQUINTLY_SUGGESTIONS_DIR/{xx}/{yy}/{sha}.{ext}`,
//! keyed by sha256. The DB row records the path. There's no expiry: per the
//! project decision, suggestions are kept indefinitely.
//!
//! Lifecycle:
//!   pending → accepted (promotes the file's sha into curator_candidates)
//!   pending → rejected (status only — file stays on disk)
//!   pending → withdrawn (submitter self-removes via /withdraw endpoint
//!                        — file stays, but the public form cannot resurface it)
//!
//! Reviewer endpoints are gated by `SQUINTLY_SUGGESTION_ADMIN_TOKEN`. When the
//! env var isn't set, the admin endpoints return 503 — same posture as the
//! Resend-not-configured fallbacks.

use anyhow::{Context, Result};
use axum::Json;
use axum::extract::{Multipart, Path as AxumPath, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use sqlx::SqlitePool;

use crate::auth::looks_like_email;
use crate::db::now_ms;
use crate::handlers::{AppError, SharedState};
use crate::licensing;

pub const MAX_UPLOAD_BYTES: usize = 24 * 1024 * 1024; // 24 MB

#[derive(Debug, Serialize)]
pub struct SuggestionRow {
    pub id: i64,
    pub sha256: String,
    pub submitted_at: i64,
    pub submitter_email: String,
    pub submitter_observer_id: Option<String>,
    pub submitter_email_verified: bool,
    pub original_page_url: String,
    pub original_image_url: Option<String>,
    pub license_id: String,
    pub license_text_freeform: Option<String>,
    pub attribution: Option<String>,
    pub why: Option<String>,
    pub file_size_bytes: i64,
    pub mime_type: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub status: String,
    pub status_reason: Option<String>,
    pub reviewed_at: Option<i64>,
    pub reviewer_email: Option<String>,
    pub accepted_candidate_sha256: Option<String>,
    pub notification_sent_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SubmitResp {
    pub id: i64,
    pub sha256: String,
    pub status: &'static str,
    pub notification_attempted: bool,
}

/// `POST /api/suggestions` — accepts multipart/form-data with these fields:
///   - `file` (required): the image bytes. Capped at MAX_UPLOAD_BYTES.
///   - `original_page_url` (required)
///   - `email` (required, unless `observer_id` resolves to a verified one)
///   - `original_image_url`, `license_id`, `license_text_freeform`,
///     `attribution`, `why`, `observer_id`, `mime_type` (optional)
pub async fn submit(
    State(state): State<SharedState>,
    mut form: Multipart,
) -> Result<Json<SubmitResp>, AppError> {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut file_mime: Option<String> = None;
    let mut field_email: Option<String> = None;
    let mut field_observer_id: Option<String> = None;
    let mut original_page_url: Option<String> = None;
    let mut original_image_url: Option<String> = None;
    let mut license_id: Option<String> = None;
    let mut license_text: Option<String> = None;
    let mut attribution: Option<String> = None;
    let mut why: Option<String> = None;
    let mut declared_mime: Option<String> = None;

    while let Some(field) = form
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("malformed multipart: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                file_mime = field.content_type().map(str::to_string);
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("file read failed: {e}")))?;
                if bytes.len() > MAX_UPLOAD_BYTES {
                    return Err(AppError::BadRequest(format!(
                        "file too large: {} bytes (max {})",
                        bytes.len(),
                        MAX_UPLOAD_BYTES
                    )));
                }
                file_bytes = Some(bytes.to_vec());
            }
            "email" => field_email = nonempty(field.text().await.ok()),
            "observer_id" => field_observer_id = nonempty(field.text().await.ok()),
            "original_page_url" => original_page_url = nonempty(field.text().await.ok()),
            "original_image_url" => original_image_url = nonempty(field.text().await.ok()),
            "license_id" => license_id = nonempty(field.text().await.ok()),
            "license_text_freeform" => license_text = nonempty(field.text().await.ok()),
            "attribution" => attribution = nonempty(field.text().await.ok()),
            "why" => why = nonempty(field.text().await.ok()),
            "mime_type" => declared_mime = nonempty(field.text().await.ok()),
            _ => {
                let _ = field.bytes().await; // drain unknown
            }
        }
    }

    let file_bytes = file_bytes.ok_or_else(|| AppError::BadRequest("file is required".into()))?;
    if file_bytes.is_empty() {
        return Err(AppError::BadRequest("file is empty".into()));
    }
    let original_page_url = original_page_url
        .ok_or_else(|| AppError::BadRequest("original_page_url is required".into()))?;
    if !is_http_url(&original_page_url) {
        return Err(AppError::BadRequest(
            "original_page_url must be http(s)://".into(),
        ));
    }
    if let Some(u) = original_image_url.as_deref() {
        if !is_http_url(u) {
            return Err(AppError::BadRequest(
                "original_image_url must be http(s)://".into(),
            ));
        }
    }

    // Resolve email + verified state.
    let (resolved_email, observer_id, verified) = resolve_email(
        &state.pool,
        field_email.as_deref(),
        field_observer_id.as_deref(),
    )
    .await?;

    // Sniff a basic image MIME from magic bytes; fall back to the multipart
    // type or declared one. Reject obviously-non-image content.
    let sniffed = sniff_image_mime(&file_bytes);
    let mime = sniffed
        .map(str::to_string)
        .or(file_mime.clone())
        .or(declared_mime.clone())
        .unwrap_or_else(|| "application/octet-stream".to_string());
    if !mime.starts_with("image/") {
        return Err(AppError::BadRequest(format!(
            "unsupported mime: {mime} (expected image/*)"
        )));
    }

    // Hash + persist.
    let mut h = Sha256::new();
    h.update(&file_bytes);
    let sha = hex::encode(h.finalize());
    let ext = ext_for_mime(&mime).unwrap_or("bin");

    let stored = state
        .suggestions
        .put(&sha, ext, &file_bytes, &mime)
        .await
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("storage put: {e}")))?;

    // Decide license_id. Caller can pass an explicit policy id; otherwise
    // 'other' if license_text_freeform is set, else 'mixed-research'.
    let license_id_resolved = license_id
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if license_text.is_some() {
                "other".to_string()
            } else {
                "mixed-research".to_string()
            }
        });

    let now = now_ms();
    let row = sqlx::query(
        "INSERT INTO suggestions \
            (sha256, submitted_at, submitter_email, submitter_observer_id, \
             submitter_email_verified, original_page_url, original_image_url, \
             license_id, license_text_freeform, attribution, why, file_path, \
             file_size_bytes, mime_type, status) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?, 'pending') \
         RETURNING id",
    )
    .bind(&sha)
    .bind(now)
    .bind(&resolved_email)
    .bind(observer_id.as_deref())
    .bind(if verified { 1i64 } else { 0 })
    .bind(&original_page_url)
    .bind(original_image_url.as_deref())
    .bind(&license_id_resolved)
    .bind(license_text.as_deref())
    .bind(attribution.as_deref())
    .bind(why.as_deref())
    .bind(&stored.locator)
    .bind(file_bytes.len() as i64)
    .bind(&mime)
    .fetch_one(&state.pool)
    .await?;
    let id: i64 = row.get(0);

    // Best-effort notification.
    let notification_attempted = notify_submission(
        &state.pool,
        id,
        &SubmissionForEmail {
            id,
            sha256: &sha,
            email: &resolved_email,
            email_verified: verified,
            page_url: &original_page_url,
            image_url: original_image_url.as_deref(),
            license_id: &license_id_resolved,
            license_text: license_text.as_deref(),
            attribution: attribution.as_deref(),
            why: why.as_deref(),
            mime: &mime,
            size_bytes: file_bytes.len(),
        },
    )
    .await;

    Ok(Json(SubmitResp {
        id,
        sha256: sha,
        status: "pending",
        notification_attempted,
    }))
}

/// `POST /api/suggestions/{id}/withdraw` — submitter self-removes a pending
/// suggestion. Authorization is by matching `submitter_email` (caller posts
/// the email as JSON body); no token needed because the email itself is the
/// secret-ish handle they used to submit. Withdrawn rows stay; their status
/// flips so they can no longer be promoted.
#[derive(Debug, Deserialize)]
pub struct WithdrawReq {
    pub email: String,
    pub reason: Option<String>,
}

pub async fn withdraw(
    State(state): State<SharedState>,
    AxumPath(id): AxumPath<i64>,
    Json(req): Json<WithdrawReq>,
) -> Result<Json<serde_json::Value>, AppError> {
    let row = sqlx::query("SELECT submitter_email, status FROM suggestions WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("suggestion {id}")))?;
    let email: String = row.get(0);
    let status: String = row.get(1);
    if !email.eq_ignore_ascii_case(req.email.trim()) {
        return Err(AppError::BadRequest(
            "email does not match submitter".into(),
        ));
    }
    if status == "accepted" {
        return Err(AppError::Conflict(
            "already accepted; cannot withdraw".into(),
        ));
    }
    sqlx::query(
        "UPDATE suggestions SET status = 'withdrawn', status_reason = ?, reviewed_at = ? WHERE id = ?",
    )
    .bind(req.reason.as_deref())
    .bind(now_ms())
    .bind(id)
    .execute(&state.pool)
    .await?;
    Ok(Json(
        serde_json::json!({"ok": true, "id": id, "status": "withdrawn"}),
    ))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
    pub admin_token: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// `GET /api/suggestions` — admin listing. Requires
/// `SQUINTLY_SUGGESTION_ADMIN_TOKEN` env to match `admin_token` query.
pub async fn list(
    State(state): State<SharedState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<SuggestionRow>>, AppError> {
    require_admin(&q.admin_token)?;
    let status = q.status.as_deref().unwrap_or("pending");
    if !matches!(
        status,
        "pending" | "accepted" | "rejected" | "withdrawn" | "all"
    ) {
        return Err(AppError::BadRequest("invalid status filter".into()));
    }
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let offset = q.offset.unwrap_or(0).max(0);

    let mut sql = String::from(
        "SELECT id, sha256, submitted_at, submitter_email, submitter_observer_id, \
                submitter_email_verified, original_page_url, original_image_url, \
                license_id, license_text_freeform, attribution, why, file_size_bytes, \
                mime_type, width, height, status, status_reason, reviewed_at, \
                reviewer_email, accepted_candidate_sha256, notification_sent_at \
         FROM suggestions ",
    );
    if status != "all" {
        sql.push_str("WHERE status = ? ");
    }
    sql.push_str("ORDER BY submitted_at DESC LIMIT ? OFFSET ?");
    let mut query = sqlx::query(&sql);
    if status != "all" {
        query = query.bind(status);
    }
    query = query.bind(limit).bind(offset);
    let rows = query.fetch_all(&state.pool).await?;
    let out = rows.into_iter().map(row_to_suggestion).collect();
    Ok(Json(out))
}

#[derive(Debug, Deserialize)]
pub struct ReviewReq {
    pub admin_token: String,
    pub reviewer_email: Option<String>,
    pub reason: Option<String>,
}

pub async fn accept(
    State(state): State<SharedState>,
    AxumPath(id): AxumPath<i64>,
    Json(req): Json<ReviewReq>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&Some(req.admin_token.clone()))?;
    let row = sqlx::query(
        "SELECT sha256, file_path, file_size_bytes, mime_type, width, height, \
                license_id, license_text_freeform, original_page_url, original_image_url, \
                attribution, status \
         FROM suggestions WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("suggestion {id}")))?;
    let status: String = row.get(11);
    if status == "accepted" {
        return Err(AppError::Conflict("already accepted".into()));
    }
    let sha: String = row.get(0);
    let file_path: String = row.get(1);
    let file_size: i64 = row.get(2);
    let mime: Option<String> = row.try_get(3).ok();
    let width: Option<i64> = row.try_get(4).ok();
    let height: Option<i64> = row.try_get(5).ok();
    let license_id: String = row.try_get(6).unwrap_or_else(|_| "other".to_string());
    let license_text: Option<String> = row.try_get(7).ok();
    let page_url: String = row.try_get(8).unwrap_or_default();
    let image_url: Option<String> = row.try_get(9).ok();
    let _attribution: Option<String> = row.try_get(10).ok();

    let format = mime.as_deref().and_then(super_image_format);
    sqlx::query(
        "INSERT INTO curator_candidates \
            (sha256, corpus, relative_path, width, height, size_bytes, format, \
             suspected_category, has_alpha, has_animation, license_id, license_url, \
             blob_url, order_hint) \
         VALUES (?, 'public-suggestions', ?, ?, ?, ?, ?, NULL, 0, 0, ?, ?, ?, 100000) \
         ON CONFLICT(sha256) DO UPDATE SET \
            license_id = excluded.license_id, \
            license_url = excluded.license_url",
    )
    .bind(&sha)
    .bind(&file_path)
    .bind(width)
    .bind(height)
    .bind(file_size)
    .bind(format)
    .bind(if license_id.is_empty() {
        "other".to_string()
    } else {
        license_id.clone()
    })
    .bind(image_url.as_deref().unwrap_or(page_url.as_str()))
    .bind(format!("/api/suggestions/{}/file", id))
    .execute(&state.pool)
    .await?;

    sqlx::query(
        "UPDATE suggestions SET status = 'accepted', reviewed_at = ?, reviewer_email = ?, \
            accepted_candidate_sha256 = ?, status_reason = ? WHERE id = ?",
    )
    .bind(now_ms())
    .bind(req.reviewer_email.as_deref())
    .bind(&sha)
    .bind(req.reason.as_deref())
    .bind(id)
    .execute(&state.pool)
    .await?;

    let _ = license_text; // recorded on the suggestions row already

    Ok(Json(serde_json::json!({
        "ok": true,
        "id": id,
        "status": "accepted",
        "candidate_sha256": sha
    })))
}

pub async fn reject(
    State(state): State<SharedState>,
    AxumPath(id): AxumPath<i64>,
    Json(req): Json<ReviewReq>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_admin(&Some(req.admin_token.clone()))?;
    let res = sqlx::query(
        "UPDATE suggestions SET status = 'rejected', reviewed_at = ?, reviewer_email = ?, \
            status_reason = ? WHERE id = ? AND status IN ('pending', 'withdrawn')",
    )
    .bind(now_ms())
    .bind(req.reviewer_email.as_deref())
    .bind(req.reason.as_deref())
    .bind(id)
    .execute(&state.pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "suggestion {id} not pending/withdrawn"
        )));
    }
    Ok(Json(
        serde_json::json!({"ok": true, "id": id, "status": "rejected"}),
    ))
}

/// `GET /api/suggestions/{id}/file` — serve the stored bytes. Public read so
/// accepted suggestions can be rendered in the curator stream like any other
/// candidate. For pending/rejected/withdrawn rows we 404 to keep them out of
/// public view.
pub async fn file(
    State(state): State<SharedState>,
    AxumPath(id): AxumPath<i64>,
) -> Result<Response, AppError> {
    let row = sqlx::query("SELECT file_path, mime_type, status FROM suggestions WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("suggestion {id}")))?;
    let locator: String = row.get(0);
    let mime: Option<String> = row.try_get(1).ok();
    let status: String = row.try_get(2).unwrap_or_default();
    if status != "accepted" {
        return Err(AppError::NotFound(
            "suggestion not in accepted state".into(),
        ));
    }
    if let Some(url) = state.suggestions.public_url(&locator) {
        return Ok((StatusCode::FOUND, [(header::LOCATION, url)]).into_response());
    }
    let bytes = state
        .suggestions
        .read(&locator)
        .await
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("read {locator}: {e}")))?;
    let mime = mime.unwrap_or_else(|| "application/octet-stream".to_string());
    Ok((StatusCode::OK, [(header::CONTENT_TYPE, mime)], bytes).into_response())
}

// ---------- helpers ----------

fn require_admin(provided: &Option<String>) -> Result<(), AppError> {
    let expected = std::env::var("SQUINTLY_SUGGESTION_ADMIN_TOKEN")
        .ok()
        .filter(|s| !s.is_empty());
    let expected = expected.ok_or_else(|| {
        AppError::ServiceUnavailable(
            "Suggestion review is not configured (SQUINTLY_SUGGESTION_ADMIN_TOKEN missing).".into(),
        )
    })?;
    let provided = provided.as_deref().unwrap_or("");
    if !ct_eq(expected.as_bytes(), provided.as_bytes()) {
        return Err(AppError::BadRequest("admin_token mismatch".into()));
    }
    Ok(())
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn nonempty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn is_http_url(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

fn sniff_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\xff\xd8\xff") {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.len() >= 12
        && &bytes[4..8] == b"ftyp"
        && (&bytes[8..12] == b"avif" || &bytes[8..12] == b"avis" || &bytes[8..12] == b"heic")
    {
        if &bytes[8..12] == b"heic" {
            return Some("image/heic");
        }
        return Some("image/avif");
    }
    if bytes.starts_with(b"\xff\x0a") || bytes.starts_with(b"\x00\x00\x00\x0cJXL ") {
        return Some("image/jxl");
    }
    None
}

fn ext_for_mime(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/avif" => "avif",
        "image/jxl" => "jxl",
        "image/heic" => "heic",
        _ => return None,
    })
}

fn super_image_format(mime: &str) -> Option<&'static str> {
    match mime {
        "image/jpeg" => Some("jpeg"),
        "image/png" => Some("png"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/avif" => Some("avif"),
        "image/jxl" => Some("jxl"),
        "image/heic" => Some("heic"),
        _ => None,
    }
}

async fn resolve_email(
    pool: &SqlitePool,
    field_email: Option<&str>,
    observer_id: Option<&str>,
) -> Result<(String, Option<String>, bool), AppError> {
    // If observer_id is provided, look up its verified email + use that as
    // the canonical address. Field email still wins if explicitly different —
    // we record both and trust the form.
    let mut verified = false;
    let mut observer_email: Option<String> = None;
    let mut canonical_observer: Option<String> = None;
    if let Some(oid) = observer_id {
        let row = sqlx::query(
            "SELECT COALESCE(o.email, ''), \
                    CASE WHEN o.email IS NOT NULL AND o.email_verified_at IS NOT NULL THEN 1 ELSE 0 END \
             FROM observers o WHERE o.id = ?",
        )
        .bind(oid)
        .fetch_optional(pool)
        .await?;
        if let Some(r) = row {
            let e: String = r.get(0);
            let v: i64 = r.get(1);
            if !e.is_empty() {
                observer_email = Some(e);
                verified = v != 0;
                canonical_observer = Some(oid.to_string());
            }
        }
    }
    let email = field_email
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| observer_email.clone());
    let email = match email {
        Some(e) if looks_like_email(&e) => e,
        Some(_) => {
            return Err(AppError::BadRequest("invalid email format".into()));
        }
        None => {
            return Err(AppError::BadRequest(
                "email is required (sign in or pass `email` field)".into(),
            ));
        }
    };
    // verified iff the observer's stored email matches the submitted one.
    let verified = verified
        && observer_email
            .as_deref()
            .map(|e| e.eq_ignore_ascii_case(&email))
            .unwrap_or(false);
    Ok((email, canonical_observer, verified))
}

fn row_to_suggestion(row: sqlx::sqlite::SqliteRow) -> SuggestionRow {
    SuggestionRow {
        id: row.get(0),
        sha256: row.get(1),
        submitted_at: row.get(2),
        submitter_email: row.get(3),
        submitter_observer_id: row.try_get(4).ok(),
        submitter_email_verified: row.try_get::<i64, _>(5).unwrap_or(0) != 0,
        original_page_url: row.get(6),
        original_image_url: row.try_get(7).ok(),
        license_id: row.get(8),
        license_text_freeform: row.try_get(9).ok(),
        attribution: row.try_get(10).ok(),
        why: row.try_get(11).ok(),
        file_size_bytes: row.get(12),
        mime_type: row.try_get(13).ok(),
        width: row.try_get(14).ok(),
        height: row.try_get(15).ok(),
        status: row.get(16),
        status_reason: row.try_get(17).ok(),
        reviewed_at: row.try_get(18).ok(),
        reviewer_email: row.try_get(19).ok(),
        accepted_candidate_sha256: row.try_get(20).ok(),
        notification_sent_at: row.try_get(21).ok(),
    }
}

// ---------- email notification ----------

struct SubmissionForEmail<'a> {
    id: i64,
    sha256: &'a str,
    email: &'a str,
    email_verified: bool,
    page_url: &'a str,
    image_url: Option<&'a str>,
    license_id: &'a str,
    license_text: Option<&'a str>,
    attribution: Option<&'a str>,
    why: Option<&'a str>,
    mime: &'a str,
    size_bytes: usize,
}

/// Postmark configuration for suggestion-notification emails.
///
/// Reads `POSTMARK_SERVER_TOKEN` (required) and `POSTMARK_FROM_EMAIL`
/// (required — Postmark requires a verified sender). Returns `None` when
/// the token isn't set, in which case notification is a no-op and we log
/// the reason at info level.
struct PostmarkConfig {
    server_token: String,
    from: String,
    /// Optional Postmark message stream — defaults to "outbound" (transactional).
    message_stream: String,
}

impl PostmarkConfig {
    fn from_env() -> Option<Self> {
        let server_token = std::env::var("POSTMARK_SERVER_TOKEN").ok()?;
        if server_token.is_empty() {
            return None;
        }
        let from = std::env::var("POSTMARK_FROM_EMAIL")
            .ok()
            .filter(|s| !s.is_empty())?;
        let message_stream = std::env::var("POSTMARK_MESSAGE_STREAM")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "outbound".to_string());
        Some(Self {
            server_token,
            from,
            message_stream,
        })
    }
}

async fn notify_submission(
    pool: &SqlitePool,
    suggestion_id: i64,
    msg: &SubmissionForEmail<'_>,
) -> bool {
    let cfg = match PostmarkConfig::from_env() {
        Some(c) => c,
        None => {
            tracing::info!(
                suggestion_id,
                "POSTMARK_SERVER_TOKEN/FROM_EMAIL missing — suggestion stored but no email sent"
            );
            return false;
        }
    };
    let to = std::env::var("SQUINTLY_SUGGESTION_NOTIFY_EMAIL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "genandlilith@gmail.com".to_string());
    let policy = licensing::by_id(msg.license_id);
    if let Err(e) = send_notification(&cfg, &to, msg, policy).await {
        tracing::warn!(suggestion_id, error = %e, "suggestion notify failed");
        return false;
    }
    let _ = sqlx::query("UPDATE suggestions SET notification_sent_at = ? WHERE id = ?")
        .bind(now_ms())
        .bind(suggestion_id)
        .execute(pool)
        .await;
    true
}

async fn send_notification(
    cfg: &PostmarkConfig,
    to: &str,
    m: &SubmissionForEmail<'_>,
    policy: &licensing::LicensePolicy,
) -> Result<()> {
    let text = format!(
        "New corpus suggestion #{id}\n\n\
         Submitter: {email} (verified: {verified})\n\
         Page URL:  {page}\n\
         Image URL: {image}\n\
         License id: {lid} ({label})\n\
         License notes: {ltext}\n\
         Attribution: {attr}\n\
         Why: {why}\n\n\
         File: sha256={sha} mime={mime} size={size} bytes\n\
         Review: /api/suggestions?status=pending&admin_token=…\n",
        id = m.id,
        email = m.email,
        verified = m.email_verified,
        page = m.page_url,
        image = m.image_url.unwrap_or("(not provided)"),
        lid = m.license_id,
        label = policy.label,
        ltext = m.license_text.unwrap_or("(none)"),
        attr = m.attribution.unwrap_or("(none)"),
        why = m.why.unwrap_or("(none)"),
        sha = m.sha256,
        mime = m.mime,
        size = m.size_bytes,
    );
    let body = serde_json::json!({
        "From": cfg.from,
        "To": to,
        "Subject": format!("Squintly suggestion #{} — {}", m.id, m.email),
        "TextBody": text,
        "MessageStream": cfg.message_stream,
    });
    let resp = reqwest::Client::new()
        .post("https://api.postmarkapp.com/email")
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("X-Postmark-Server-Token", &cfg.server_token)
        .json(&body)
        .send()
        .await
        .context("calling Postmark")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Postmark rejected ({status}): {text}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_mimes() {
        assert_eq!(sniff_image_mime(b"\xff\xd8\xff\xe0..."), Some("image/jpeg"));
        assert_eq!(sniff_image_mime(b"\x89PNG\r\n\x1a\n_"), Some("image/png"));
        assert_eq!(sniff_image_mime(b"GIF89a..."), Some("image/gif"));
        let mut webp = Vec::from(*b"RIFF\x00\x00\x00\x00WEBP");
        webp.extend_from_slice(&[0; 4]);
        assert_eq!(sniff_image_mime(&webp), Some("image/webp"));
        assert_eq!(sniff_image_mime(b"hello there"), None);
    }

    #[test]
    fn require_admin_rejects_when_unset() {
        // safe to test in serial — we set + unset within this scope
        let prev = std::env::var("SQUINTLY_SUGGESTION_ADMIN_TOKEN").ok();
        // SAFETY: tests run serially via `cargo test -- --test-threads=1` if
        // env-mutation matters; in our tree most tests don't depend on this var.
        unsafe {
            std::env::remove_var("SQUINTLY_SUGGESTION_ADMIN_TOKEN");
        }
        assert!(matches!(
            require_admin(&Some("anything".into())),
            Err(AppError::ServiceUnavailable(_))
        ));
        if let Some(p) = prev {
            unsafe {
                std::env::set_var("SQUINTLY_SUGGESTION_ADMIN_TOKEN", p);
            }
        }
    }

    #[test]
    fn ct_eq_works() {
        assert!(ct_eq(b"abcd", b"abcd"));
        assert!(!ct_eq(b"abcd", b"abce"));
        assert!(!ct_eq(b"abcd", b"abc"));
    }
}
