//! HTTP handlers. Thin glue around `coefficient`, `sampling`, and `db`.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::SqlitePool;
use uuid::Uuid;

use chrono::NaiveDate;

use crate::auth::{
    EmailMessage, ResendConfig, TOKEN_TTL_MS, generate_token, hash_token, looks_like_email,
    send_magic_link,
};
use crate::coefficient::{CoefficientSource, Manifest};
use crate::db::now_ms;
use crate::grading::{InlineGradeInput, compute_response_flags, grade_session};
use crate::sampling::{SamplerConfig, TrialPlan, pick_trial};
use crate::streaks::{
    StreakState, advance_streak, crossed_streak_milestone, crossed_trial_milestone,
};

pub struct AppState {
    pub pool: SqlitePool,
    pub coefficient: CoefficientSource,
    pub manifest: tokio::sync::RwLock<Manifest>,
}

pub type SharedState = Arc<AppState>;

// ---------- session ----------

#[derive(Debug, Deserialize)]
pub struct CreateSessionReq {
    pub observer_id: Option<String>,
    pub user_agent: Option<String>,
    pub age_bracket: Option<String>,
    pub vision_corrected: Option<String>,

    pub device_pixel_ratio: f64,
    pub screen_width_css: i64,
    pub screen_height_css: i64,
    pub color_gamut: Option<String>,
    pub dynamic_range_high: Option<bool>,
    pub prefers_dark: Option<bool>,
    pub pointer_type: Option<String>,
    pub timezone: Option<String>,

    pub viewing_distance_cm: Option<i64>,
    pub ambient_light: Option<String>,
    pub css_px_per_mm: Option<f64>,
    pub notes: Option<String>,

    /// Observer's local calendar date (ISO YYYY-MM-DD) for streak math. The client
    /// always knows its local date; sending it explicitly avoids the server needing
    /// chrono-tz and a timezone database.
    pub local_date: Option<String>,

    /// Theme picked for this session. Optional; falls back to corpus default.
    pub theme_slug: Option<String>,

    /// Codecs the browser natively decodes, captured by the client-side probe.
    /// e.g. ["jpeg", "png", "webp", "avif"]. The sampler filters trials to this
    /// set so we never serve a codec the observer can't natively render.
    pub supported_codecs: Option<Vec<String>>,
    pub codec_probe_cached: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct CreateSessionResp {
    pub observer_id: String,
    pub session_id: String,
    pub streak_days: u32,
    pub streak_outcome: &'static str, // "advanced" | "frozen" | "reset" | "same_day" | "skipped"
    pub freezes_remaining: u32,
    pub total_trials: u32,
}

pub async fn create_session(
    State(state): State<SharedState>,
    Json(req): Json<CreateSessionReq>,
) -> Result<Json<CreateSessionResp>, AppError> {
    let observer_id = match req.observer_id {
        Some(id) if Uuid::parse_str(&id).is_ok() => id,
        _ => Uuid::new_v4().to_string(),
    };
    let session_id = Uuid::new_v4().to_string();
    let now = now_ms();

    sqlx::query(
        "INSERT OR IGNORE INTO observers (id, created_at, user_agent, age_bracket, vision_corrected) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&observer_id)
    .bind(now)
    .bind(req.user_agent.as_deref())
    .bind(req.age_bracket.as_deref())
    .bind(req.vision_corrected.as_deref())
    .execute(&state.pool)
    .await?;

    // Streak advance, if the client supplied its local date. Lenient v0.1 rule:
    // streak advances on session creation, not on first response. Stricter rule
    // (Duolingo-style "complete a lesson") is a v0.2 backlog item.
    let (streak_days, streak_outcome, freezes_remaining) = if let Some(date_str) =
        req.local_date.as_deref()
    {
        if let Ok(today) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
            let row: Option<(i64, i64, Option<String>)> = sqlx::query_as(
                "SELECT streak_days, freezes_remaining, streak_last_date FROM observers WHERE id = ?",
            )
            .bind(&observer_id)
            .fetch_optional(&state.pool)
            .await?;
            let prev = match row {
                Some((sd, fr, last)) => StreakState {
                    streak_days: sd as u32,
                    freezes_remaining: fr as u32,
                    last_date: last.and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()),
                },
                None => StreakState {
                    streak_days: 0,
                    freezes_remaining: 1,
                    last_date: None,
                },
            };
            let (next, outcome) = advance_streak(prev, today);
            sqlx::query(
                "UPDATE observers SET streak_days = ?, streak_last_date = ?, freezes_remaining = ? \
                 WHERE id = ?",
            )
            .bind(next.streak_days as i64)
            .bind(next.last_date.map(|d| d.format("%Y-%m-%d").to_string()))
            .bind(next.freezes_remaining as i64)
            .bind(&observer_id)
            .execute(&state.pool)
            .await?;
            // Award streak milestone badge if crossed.
            if let Some(slug) = crossed_streak_milestone(prev.streak_days, next.streak_days) {
                award_badge(&state.pool, &observer_id, slug).await?;
            }
            (
                next.streak_days,
                match outcome {
                    crate::streaks::StreakOutcome::Advanced => "advanced",
                    crate::streaks::StreakOutcome::Frozen => "frozen",
                    crate::streaks::StreakOutcome::Reset => "reset",
                    crate::streaks::StreakOutcome::SameDay => "same_day",
                },
                next.freezes_remaining,
            )
        } else {
            (0, "skipped", 0)
        }
    } else {
        (0, "skipped", 0)
    };

    let supported_codecs_csv = req.supported_codecs.as_ref().map(|v| {
        v.iter()
            .map(|s| s.to_lowercase())
            .collect::<Vec<_>>()
            .join(",")
    });

    sqlx::query(
        "INSERT INTO sessions (id, observer_id, started_at, device_pixel_ratio, \
         screen_width_css, screen_height_css, color_gamut, dynamic_range_high, prefers_dark, \
         pointer_type, timezone, viewing_distance_cm, ambient_light, css_px_per_mm, notes, \
         theme_slug, supported_codecs, codec_probe_cached) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&session_id)
    .bind(&observer_id)
    .bind(now)
    .bind(req.device_pixel_ratio)
    .bind(req.screen_width_css)
    .bind(req.screen_height_css)
    .bind(req.color_gamut.as_deref())
    .bind(req.dynamic_range_high.map(|b| b as i64))
    .bind(req.prefers_dark.map(|b| b as i64))
    .bind(req.pointer_type.as_deref())
    .bind(req.timezone.as_deref())
    .bind(req.viewing_distance_cm)
    .bind(req.ambient_light.as_deref())
    .bind(req.css_px_per_mm)
    .bind(req.notes.as_deref())
    .bind(req.theme_slug.as_deref())
    .bind(supported_codecs_csv.as_deref())
    .bind(req.codec_probe_cached.unwrap_or(false) as i64)
    .execute(&state.pool)
    .await?;

    let total_trials: (i64,) = sqlx::query_as("SELECT total_trials FROM observers WHERE id = ?")
        .bind(&observer_id)
        .fetch_one(&state.pool)
        .await?;

    Ok(Json(CreateSessionResp {
        observer_id,
        session_id,
        streak_days,
        streak_outcome,
        freezes_remaining,
        total_trials: total_trials.0 as u32,
    }))
}

async fn award_badge(pool: &SqlitePool, observer_id: &str, slug: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO observer_badges (observer_id, badge_slug, awarded_at) \
         VALUES (?, ?, ?)",
    )
    .bind(observer_id)
    .bind(slug)
    .bind(now_ms())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn end_session(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    sqlx::query("UPDATE sessions SET ended_at = ? WHERE id = ?")
        .bind(now_ms())
        .bind(&id)
        .execute(&state.pool)
        .await?;
    // Compute the session grade. Failures here shouldn't fail the request — the
    // observer doesn't care, and we'd rather have an ungraded session than block
    // /session/end.
    if let Err(e) = grade_session(&state.pool, &id).await {
        tracing::warn!(?e, %id, "grade_session failed");
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------- trial ----------

#[derive(Debug, Deserialize)]
pub struct NextTrialQuery {
    pub session_id: String,
}

#[derive(Debug, Serialize)]
pub struct TrialPayload {
    pub trial_id: String,
    pub kind: &'static str, // "single" | "pair"
    pub source_hash: String,
    pub source_url: String,
    pub source_w: u32,
    pub source_h: u32,
    pub a: TrialEncoding,
    pub b: Option<TrialEncoding>,
    pub staircase_target: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TrialEncoding {
    pub encoding_id: String,
    pub url: String,
    pub codec: String,
    pub quality: Option<f32>,
    pub bytes: u64,
}

pub async fn next_trial(
    State(state): State<SharedState>,
    Query(q): Query<NextTrialQuery>,
) -> Result<Json<TrialPayload>, AppError> {
    // Read the session's supported_codecs and filter the sampler accordingly.
    let codecs_csv: Option<(Option<String>,)> =
        sqlx::query_as("SELECT supported_codecs FROM sessions WHERE id = ?")
            .bind(&q.session_id)
            .fetch_optional(&state.pool)
            .await?;
    let allowed: Option<std::collections::HashSet<String>> = codecs_csv
        .and_then(|(s,)| s)
        .map(|s| s.split(',').map(str::trim).map(str::to_lowercase).collect());

    let manifest = state.manifest.read().await;
    let plan = pick_trial(&manifest, &SamplerConfig::default(), allowed.as_ref())
        .ok_or_else(|| AppError::Conflict("no trials available — empty manifest or no encodings match this session's supported codecs".into()))?;
    let trial_id = Uuid::new_v4().to_string();
    let served_at = now_ms();

    let payload = match plan {
        TrialPlan::Single {
            source,
            encoding,
            staircase_target,
        } => {
            sqlx::query(
                "INSERT INTO trials (id, session_id, kind, source_hash, a_encoding_id, a_codec, \
                 a_quality, a_bytes, intrinsic_w, intrinsic_h, staircase_target, served_at) \
                 VALUES (?, ?, 'single', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&trial_id)
            .bind(&q.session_id)
            .bind(&source.hash)
            .bind(&encoding.id)
            .bind(&encoding.codec)
            .bind(encoding.quality)
            .bind(encoding.bytes as i64)
            .bind(source.width as i64)
            .bind(source.height as i64)
            .bind(staircase_target)
            .bind(served_at)
            .execute(&state.pool)
            .await?;

            TrialPayload {
                trial_id,
                kind: "single",
                source_hash: source.hash.clone(),
                source_url: format!("/api/proxy/source/{}", source.hash),
                source_w: source.width,
                source_h: source.height,
                a: TrialEncoding {
                    url: format!("/api/proxy/encoding/{}", encoding.id),
                    encoding_id: encoding.id.clone(),
                    codec: encoding.codec.clone(),
                    quality: encoding.quality,
                    bytes: encoding.bytes,
                },
                b: None,
                staircase_target: staircase_target.map(str::to_string),
            }
        }
        TrialPlan::Pair { source, a, b } => {
            sqlx::query(
                "INSERT INTO trials (id, session_id, kind, source_hash, a_encoding_id, a_codec, \
                 a_quality, a_bytes, b_encoding_id, b_codec, b_quality, b_bytes, intrinsic_w, \
                 intrinsic_h, served_at) \
                 VALUES (?, ?, 'pair', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&trial_id)
            .bind(&q.session_id)
            .bind(&source.hash)
            .bind(&a.id)
            .bind(&a.codec)
            .bind(a.quality)
            .bind(a.bytes as i64)
            .bind(&b.id)
            .bind(&b.codec)
            .bind(b.quality)
            .bind(b.bytes as i64)
            .bind(source.width as i64)
            .bind(source.height as i64)
            .bind(served_at)
            .execute(&state.pool)
            .await?;

            TrialPayload {
                trial_id,
                kind: "pair",
                source_hash: source.hash.clone(),
                source_url: format!("/api/proxy/source/{}", source.hash),
                source_w: source.width,
                source_h: source.height,
                a: TrialEncoding {
                    url: format!("/api/proxy/encoding/{}", a.id),
                    encoding_id: a.id.clone(),
                    codec: a.codec.clone(),
                    quality: a.quality,
                    bytes: a.bytes,
                },
                b: Some(TrialEncoding {
                    url: format!("/api/proxy/encoding/{}", b.id),
                    encoding_id: b.id.clone(),
                    codec: b.codec.clone(),
                    quality: b.quality,
                    bytes: b.bytes,
                }),
                staircase_target: None,
            }
        }
    };

    Ok(Json(payload))
}

#[derive(Debug, Deserialize)]
pub struct ResponseReq {
    pub choice: String,
    pub dwell_ms: i64,
    pub reveal_count: i64,
    pub reveal_ms_total: i64,
    pub zoom_used: bool,
    pub viewport_w_css: i64,
    pub viewport_h_css: i64,
    pub orientation: String,
    pub image_displayed_w_css: f64,
    pub image_displayed_h_css: f64,
    pub intrinsic_to_device_ratio: f64,
    pub pixels_per_degree: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct ResponseAck {
    pub total_trials: u32,
    pub milestone_badge: Option<String>,
    pub flags: Option<String>,
}

pub async fn record_response(
    State(state): State<SharedState>,
    Path(trial_id): Path<String>,
    Json(req): Json<ResponseReq>,
) -> Result<Json<ResponseAck>, AppError> {
    // Pull the trial we're answering so we can compute inline grading flags.
    let row = sqlx::query(
        "SELECT kind, is_golden, expected_choice, intrinsic_w \
         FROM trials WHERE id = ?",
    )
    .bind(&trial_id)
    .fetch_optional(&state.pool)
    .await?;
    let (kind, is_golden, expected_choice, intrinsic_w): (String, i64, Option<String>, i64) =
        match row {
            Some(r) => (r.get(0), r.get(1), r.get(2), r.get(3)),
            None => return Err(AppError::NotFound(format!("trial {trial_id}"))),
        };
    // Heuristic for dpr at trial time: image_displayed_w_css * dpr ≈ on-screen device px.
    // We don't carry dpr in the response payload; pull from the session.
    let dpr_row: (f64,) = sqlx::query_as(
        "SELECT s.device_pixel_ratio FROM sessions s \
         JOIN trials t ON t.session_id = s.id WHERE t.id = ?",
    )
    .bind(&trial_id)
    .fetch_one(&state.pool)
    .await?;
    let flags = compute_response_flags(&InlineGradeInput {
        kind: &kind,
        dwell_ms: req.dwell_ms,
        reveal_count: req.reveal_count,
        choice: &req.choice,
        is_golden: is_golden == 1,
        expected_choice: expected_choice.as_deref(),
        image_displayed_w_css: req.image_displayed_w_css,
        intrinsic_w,
        dpr: dpr_row.0,
    });

    sqlx::query(
        "INSERT INTO responses (trial_id, choice, dwell_ms, reveal_count, reveal_ms_total, \
         zoom_used, viewport_w_css, viewport_h_css, orientation, image_displayed_w_css, \
         image_displayed_h_css, intrinsic_to_device_ratio, pixels_per_degree, response_flags, \
         responded_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&trial_id)
    .bind(&req.choice)
    .bind(req.dwell_ms)
    .bind(req.reveal_count)
    .bind(req.reveal_ms_total)
    .bind(req.zoom_used as i64)
    .bind(req.viewport_w_css)
    .bind(req.viewport_h_css)
    .bind(&req.orientation)
    .bind(req.image_displayed_w_css)
    .bind(req.image_displayed_h_css)
    .bind(req.intrinsic_to_device_ratio)
    .bind(req.pixels_per_degree)
    .bind(flags.join())
    .bind(now_ms())
    .execute(&state.pool)
    .await?;

    // Increment the observer's total_trials and check for a milestone crossing.
    let observer: (String, i64) = sqlx::query_as(
        "SELECT s.observer_id, o.total_trials FROM sessions s \
         JOIN observers o ON o.id = s.observer_id \
         JOIN trials t ON t.session_id = s.id WHERE t.id = ?",
    )
    .bind(&trial_id)
    .fetch_one(&state.pool)
    .await?;
    let prev_total = observer.1 as u32;
    let new_total = prev_total + 1;
    sqlx::query("UPDATE observers SET total_trials = ? WHERE id = ?")
        .bind(new_total as i64)
        .bind(&observer.0)
        .execute(&state.pool)
        .await?;
    let milestone = crossed_trial_milestone(prev_total, new_total);
    if let Some(slug) = milestone {
        award_badge(&state.pool, &observer.0, slug).await?;
    }

    Ok(Json(ResponseAck {
        total_trials: new_total,
        milestone_badge: milestone.map(str::to_string),
        flags: flags.join(),
    }))
}

// ---------- proxy ----------

pub async fn proxy_source(
    State(state): State<SharedState>,
    Path(hash): Path<String>,
) -> Result<Response, AppError> {
    let (bytes, mime) = state.coefficient.fetch_source_png(&hash).await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(&mime)?);
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );
    Ok((StatusCode::OK, headers, Bytes::from(bytes)).into_response())
}

pub async fn proxy_encoding(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let (bytes, mime) = state.coefficient.fetch_encoding_blob(&id).await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(&mime)?);
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );
    Ok((StatusCode::OK, headers, Bytes::from(bytes)).into_response())
}

// ---------- export ----------

pub async fn export_pareto(State(state): State<SharedState>) -> Result<Response, AppError> {
    let body = crate::export::pareto_tsv(&state.pool).await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/tab-separated-values"),
    );
    Ok((StatusCode::OK, headers, body).into_response())
}

pub async fn export_thresholds(State(state): State<SharedState>) -> Result<Response, AppError> {
    let body = crate::export::thresholds_tsv(&state.pool).await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/tab-separated-values"),
    );
    Ok((StatusCode::OK, headers, body).into_response())
}

pub async fn export_responses(State(state): State<SharedState>) -> Result<Response, AppError> {
    let body = crate::export::responses_tsv(&state.pool).await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/tab-separated-values"),
    );
    Ok((StatusCode::OK, headers, body).into_response())
}

// ---------- optional email magic-link auth ----------

#[derive(Debug, Deserialize)]
pub struct AuthStartReq {
    pub email: String,
    pub observer_id: Option<String>,
    /// Where the magic link should land. Provided by the client so that
    /// the server doesn't need to know its public URL (Railway-friendly).
    pub origin: String,
}

#[derive(Debug, Serialize)]
pub struct AuthStartResp {
    pub ok: bool,
    pub message: String,
}

pub async fn auth_start(
    State(state): State<SharedState>,
    Json(req): Json<AuthStartReq>,
) -> Result<Json<AuthStartResp>, AppError> {
    let email = req.email.trim().to_lowercase();
    if !looks_like_email(&email) {
        return Err(AppError::BadRequest("invalid email".into()));
    }

    // We require an observer_id if one is on the device; new-device sign-ins can
    // pass null and the verify endpoint will give them a fresh observer.
    if let Some(id) = req.observer_id.as_deref() {
        if Uuid::parse_str(id).is_err() {
            return Err(AppError::BadRequest("invalid observer_id".into()));
        }
    }

    // Reject if the origin scheme/host shape is suspicious. The verify URL we
    // build from this string is what's mailed to the user.
    if let Err(e) = url::Url::parse(&req.origin) {
        return Err(AppError::BadRequest(format!("invalid origin: {e}")));
    }

    let cfg = ResendConfig::from_env().ok_or_else(|| {
        AppError::ServiceUnavailable(
            "Email login is not configured on this deployment (RESEND_API_KEY missing). \
             Anonymous use is unaffected."
                .into(),
        )
    })?;

    let token = generate_token();
    let token_hash = hash_token(&token);
    let now = now_ms();
    let expires_at = now + TOKEN_TTL_MS;

    sqlx::query(
        "INSERT INTO auth_tokens (token_hash, email, requesting_observer_id, expires_at, \
         consumed_at, created_at) VALUES (?, ?, ?, ?, NULL, ?)",
    )
    .bind(&token_hash)
    .bind(&email)
    .bind(req.observer_id.as_deref())
    .bind(expires_at)
    .bind(now)
    .execute(&state.pool)
    .await?;

    let link = format!(
        "{}/api/auth/verify?token={}",
        req.origin.trim_end_matches('/'),
        token
    );
    send_magic_link(
        &cfg,
        EmailMessage {
            to: &email,
            link_url: &link,
        },
    )
    .await?;

    Ok(Json(AuthStartResp {
        ok: true,
        message: format!(
            "If an account is associated with {email}, a sign-in link has been sent. \
             It expires in 15 minutes."
        ),
    }))
}

#[derive(Debug, Deserialize)]
pub struct AuthVerifyQuery {
    pub token: String,
}

/// GET /api/auth/verify?token=...
///
/// Returns a tiny self-contained HTML page that:
///   1. Shows a success/failure message.
///   2. On success, writes the resolved `observer_id` into localStorage and
///      redirects to `/`. Cross-tab sync is intentionally not used; a single
///      tab opens, succeeds, redirects.
pub async fn auth_verify(
    State(state): State<SharedState>,
    Query(q): Query<AuthVerifyQuery>,
) -> Result<Response, AppError> {
    if q.token.len() != 64 || !q.token.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(verify_page(
            VerifyOutcome::Invalid("That link looks malformed."),
            None,
        ));
    }

    let token_hash = hash_token(&q.token);
    let row: Option<(String, Option<String>, i64, Option<i64>)> = sqlx::query_as(
        "SELECT email, requesting_observer_id, expires_at, consumed_at \
         FROM auth_tokens WHERE token_hash = ?",
    )
    .bind(&token_hash)
    .fetch_optional(&state.pool)
    .await?;
    let Some((email, requesting_observer_id, expires_at, consumed_at)) = row else {
        return Ok(verify_page(
            VerifyOutcome::Invalid("That link wasn't recognised. Try requesting a new one."),
            None,
        ));
    };

    let now = now_ms();
    if let Some(used) = consumed_at {
        let _ = used;
        return Ok(verify_page(
            VerifyOutcome::Invalid(
                "That link was already used. Request a new one if you need to sign in again.",
            ),
            None,
        ));
    }
    if expires_at < now {
        return Ok(verify_page(
            VerifyOutcome::Invalid("That link has expired. Request a new one."),
            None,
        ));
    }

    // Resolve the canonical observer for this email.
    let canonical: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM observers WHERE LOWER(email) = ? AND id NOT IN \
         (SELECT alias_id FROM observer_aliases) LIMIT 1",
    )
    .bind(&email)
    .fetch_optional(&state.pool)
    .await?;

    let resolved_observer_id = match (canonical, requesting_observer_id.as_deref()) {
        (Some((canonical_id,)), Some(req_id)) if canonical_id != req_id => {
            // Merge: the requesting observer is now an alias of the canonical one.
            sqlx::query(
                "INSERT OR REPLACE INTO observer_aliases (alias_id, canonical_id, merged_at) \
                 VALUES (?, ?, ?)",
            )
            .bind(req_id)
            .bind(&canonical_id)
            .bind(now)
            .execute(&state.pool)
            .await?;
            canonical_id
        }
        (Some((canonical_id,)), _) => canonical_id,
        (None, Some(req_id)) => {
            // First sign-in for this email — bind the email to the requesting observer.
            sqlx::query(
                "UPDATE observers SET email = ?, email_verified_at = ?, account_tier = MAX(account_tier, 1) WHERE id = ?",
            )
            .bind(&email)
            .bind(now)
            .bind(req_id)
            .execute(&state.pool)
            .await?;
            req_id.to_string()
        }
        (None, None) => {
            // Cross-device first time — no observer record exists; create one.
            let new_id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO observers (id, created_at, email, email_verified_at, account_tier) \
                 VALUES (?, ?, ?, ?, 1)",
            )
            .bind(&new_id)
            .bind(now)
            .bind(&email)
            .bind(now)
            .execute(&state.pool)
            .await?;
            new_id
        }
    };

    sqlx::query("UPDATE auth_tokens SET consumed_at = ? WHERE token_hash = ?")
        .bind(now)
        .bind(&token_hash)
        .execute(&state.pool)
        .await?;

    Ok(verify_page(
        VerifyOutcome::Success { email },
        Some(resolved_observer_id),
    ))
}

enum VerifyOutcome {
    Success { email: String },
    Invalid(&'static str),
}

fn verify_page(outcome: VerifyOutcome, observer_id: Option<String>) -> Response {
    let (title, msg, status) = match &outcome {
        VerifyOutcome::Success { email } => (
            "Signed in",
            format!("Signed in as {email}. Redirecting to Squintly…"),
            StatusCode::OK,
        ),
        VerifyOutcome::Invalid(m) => ("Sign-in failed", m.to_string(), StatusCode::OK),
    };
    let observer_js = observer_id
        .map(|id| {
            format!(
                "try {{ localStorage.setItem('squintly:observer_id', {js}); }} catch (e) {{}}\n",
                js = serde_json::to_string(&id).unwrap_or_else(|_| "''".into())
            )
        })
        .unwrap_or_default();
    let redirect_js = if matches!(outcome, VerifyOutcome::Success { .. }) {
        "setTimeout(() => { location.href = '/'; }, 1200);"
    } else {
        ""
    };
    let html = format!(
        "<!doctype html><html><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\">\
         <title>{title} — Squintly</title>\
         <style>html,body{{margin:0;padding:0;background:#0a0a0c;color:#f0f0f2;font-family:-apple-system,BlinkMacSystemFont,system-ui,sans-serif;min-height:100dvh;display:flex;align-items:center;justify-content:center}} .card{{max-width:420px;padding:24px;text-align:center;line-height:1.5}} h1{{margin:0 0 8px;font-size:1.25rem}} p{{margin:8px 0;color:#cfcfd6}}</style>\
         </head><body><div class=card><h1>{title}</h1><p>{msg}</p></div>\
         <script>{observer_js}{redirect_js}</script></body></html>"
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    (status, headers, html).into_response()
}

// ---------- observer profile ----------

#[derive(Debug, Serialize)]
pub struct ObserverProfile {
    pub observer_id: String,
    pub streak_days: u32,
    pub streak_last_date: Option<String>,
    pub freezes_remaining: u32,
    pub total_trials: u32,
    pub skill_score: Option<f32>,
    pub compensation_mode: String,
    pub badges: Vec<BadgeAwarded>,
    pub themes: Vec<ThemeInfo>,
}

#[derive(Debug, Serialize)]
pub struct BadgeAwarded {
    pub slug: String,
    pub display_name: String,
    pub awarded_at: i64,
}

#[derive(Debug, Serialize)]
pub struct ThemeInfo {
    pub slug: String,
    pub display_name: String,
    pub is_default: bool,
}

type ProfileRow = (i64, Option<String>, i64, i64, Option<f64>, String);

pub async fn observer_profile(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<ObserverProfile>, AppError> {
    let row: Option<ProfileRow> = sqlx::query_as(
        "SELECT streak_days, streak_last_date, freezes_remaining, total_trials, \
                skill_score, compensation_mode \
         FROM observers WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await?;
    let (streak_days, streak_last_date, freezes_remaining, total_trials, skill_score, comp_mode) =
        match row {
            Some(r) => (r.0, r.1, r.2, r.3, r.4, r.5),
            None => return Err(AppError::NotFound(format!("observer {id}"))),
        };

    let badges = sqlx::query(
        "SELECT b.slug, b.display_name, ob.awarded_at FROM observer_badges ob \
         JOIN badges b ON b.slug = ob.badge_slug \
         WHERE ob.observer_id = ? ORDER BY ob.awarded_at",
    )
    .bind(&id)
    .fetch_all(&state.pool)
    .await?;
    let badges: Vec<BadgeAwarded> = badges
        .into_iter()
        .map(|r| BadgeAwarded {
            slug: r.get(0),
            display_name: r.get(1),
            awarded_at: r.get(2),
        })
        .collect();

    let themes = sqlx::query(
        "SELECT slug, display_name, is_default FROM corpus_themes WHERE enabled = 1 ORDER BY is_default DESC, slug",
    )
    .fetch_all(&state.pool)
    .await?;
    let themes: Vec<ThemeInfo> = themes
        .into_iter()
        .map(|r| ThemeInfo {
            slug: r.get(0),
            display_name: r.get(1),
            is_default: r.get::<i64, _>(2) != 0,
        })
        .collect();

    Ok(Json(ObserverProfile {
        observer_id: id,
        streak_days: streak_days as u32,
        streak_last_date,
        freezes_remaining: freezes_remaining as u32,
        total_trials: total_trials as u32,
        skill_score: skill_score.map(|v| v as f32),
        compensation_mode: comp_mode,
        badges,
        themes,
    }))
}

// ---------- stats / refresh ----------

#[derive(Debug, Serialize)]
pub struct Stats {
    pub observers: i64,
    pub sessions: i64,
    pub trials: i64,
    pub responses: i64,
    pub manifest_sources: usize,
    pub manifest_encodings: usize,
}

pub async fn stats(State(state): State<SharedState>) -> Result<Json<Stats>, AppError> {
    let observers = crate::db::count(&state.pool, "SELECT COUNT(*) FROM observers").await?;
    let sessions = crate::db::count(&state.pool, "SELECT COUNT(*) FROM sessions").await?;
    let trials = crate::db::count(&state.pool, "SELECT COUNT(*) FROM trials").await?;
    let responses = crate::db::count(&state.pool, "SELECT COUNT(*) FROM responses").await?;
    let m = state.manifest.read().await;
    Ok(Json(Stats {
        observers,
        sessions,
        trials,
        responses,
        manifest_sources: m.sources.len(),
        manifest_encodings: m.encodings.len(),
    }))
}

pub async fn refresh_manifest(State(state): State<SharedState>) -> Result<Json<Stats>, AppError> {
    let new_manifest = state.coefficient.refresh_manifest().await?;
    *state.manifest.write().await = new_manifest;
    stats(State(state)).await
}

// ---------- static frontend ----------

pub async fn serve_static<E: RustEmbed>(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match E::get(path).or_else(|| E::get("index.html")) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(mime.as_ref())
                    .unwrap_or(HeaderValue::from_static("application/octet-stream")),
            );
            (StatusCode::OK, headers, Bytes::from(file.data.into_owned())).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            "frontend assets not embedded — run `cd web && npm run build` and rebuild",
        )
            .into_response(),
    }
}

// ---------- error type ----------

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("anyhow: {0}")]
    Anyhow(#[from] anyhow::Error),
    #[error("invalid header: {0}")]
    Header(#[from] axum::http::header::InvalidHeaderValue),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (code, msg) = match &self {
            AppError::NotFound(s) => (StatusCode::NOT_FOUND, s.clone()),
            AppError::Conflict(s) => (StatusCode::CONFLICT, s.clone()),
            AppError::BadRequest(s) => (StatusCode::BAD_REQUEST, s.clone()),
            AppError::ServiceUnavailable(s) => (StatusCode::SERVICE_UNAVAILABLE, s.clone()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        tracing::warn!(?code, %msg, "request failed");
        (code, msg).into_response()
    }
}
