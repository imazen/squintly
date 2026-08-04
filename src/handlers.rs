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
    EmailAllowlist, EmailMessage, RateLimit, RateVerdict, ResendConfig, SESSION_TTL_MS,
    SendFailure, TOKEN_TTL_MS, client_ip, generate_token, hash_ip, hash_token, looks_like_email,
    rate_verdict, send_magic_link, session_cookie, session_from_cookie_header,
};
use crate::coefficient::{CoefficientSource, EncodingMeta, Manifest};
use crate::db::now_ms;
use crate::grading::{InlineGradeInput, compute_response_flags, grade_session};
use crate::sampling::{
    ASAP_MIN_OBS, AnchorEntry, AnchorPool, SourceFlagMap, TrialPlan, pick_trial,
    select_pair_with_eig,
};
use crate::streaks::{
    StreakState, advance_streak, crossed_streak_milestone, crossed_trial_milestone,
};

pub struct AppState {
    pub pool: SqlitePool,
    pub coefficient: CoefficientSource,
    pub manifest: tokio::sync::RwLock<Manifest>,
    pub anchors: tokio::sync::RwLock<AnchorPool>,
    pub source_flags: tokio::sync::RwLock<SourceFlagMap>,
    /// Storage backend for public-suggestion uploads. R2 in production,
    /// local-disk fallback for dev/tests. See `src/suggestion_store.rs`.
    pub suggestions: crate::suggestion_store::SuggestionStore,
}

pub type SharedState = Arc<AppState>;

/// Load `corpus_anchors` from the database, classifying entries by `role`.
pub async fn load_anchor_pool(pool: &SqlitePool) -> Result<AnchorPool, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT source_hash, encoding_id, codec, quality, role, expected_choice \
         FROM corpus_anchors",
    )
    .fetch_all(pool)
    .await?;
    let mut p = AnchorPool::default();
    for row in rows {
        let entry = AnchorEntry {
            source_hash: row.get(0),
            encoding_id: row.get(1),
            codec: row.get(2),
            quality: row.get::<f64, _>(3) as f32,
            expected_choice: row.get(5),
        };
        let role: String = row.get(4);
        match role.as_str() {
            "honeypot" => p.honeypots.push(entry),
            _ => p.anchors.push(entry),
        }
    }
    Ok(p)
}

/// Load `source_flags` (held-out validation set, codec-version metadata).
pub async fn load_source_flags(pool: &SqlitePool) -> Result<SourceFlagMap, sqlx::Error> {
    let rows = sqlx::query("SELECT source_hash, held_out FROM source_flags")
        .fetch_all(pool)
        .await?;
    let mut m = SourceFlagMap::default();
    for row in rows {
        let h: String = row.get(0);
        let held_out: i64 = row.get(1);
        if held_out != 0 {
            m.held_out.insert(h);
        }
    }
    Ok(m)
}

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

    /// Which named study this session contributes to (`src/studies.rs`).
    /// Unknown ids are rejected rather than silently coerced — the study
    /// determines the trial stream, so quietly running a different one would
    /// put incompatible data in the same table.
    pub study_id: Option<String>,

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
    /// Echoed so the client can show which study it joined (and detect that a
    /// requested one was substituted).
    pub study_id: String,
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

    let study = match req
        .study_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(id) => crate::studies::by_id(id).ok_or_else(|| {
            AppError::BadRequest(format!(
                "unknown study_id {id:?}; known: {:?}",
                crate::studies::STUDIES
                    .iter()
                    .map(|s| s.id)
                    .collect::<Vec<_>>()
            ))
        })?,
        None => crate::studies::default_study(),
    };

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

    // Pin the most recent manifest snapshot for reproducibility — when
    // someone re-runs an analysis six months from now they can join from
    // sessions.manifest_snapshot_id → manifest_snapshots and recover the
    // exact R2 base + path + body sha256 the observer's trials drew
    // candidates from. NULL when there's no R2 snapshot (fresh DB, FS
    // coefficient mode).
    let manifest_snapshot_id: Option<String> = sqlx::query_as::<_, (String,)>(
        "SELECT id FROM manifest_snapshots ORDER BY loaded_at DESC LIMIT 1",
    )
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .map(|(id,)| id);

    sqlx::query(
        "INSERT INTO sessions (id, observer_id, started_at, device_pixel_ratio, \
         screen_width_css, screen_height_css, color_gamut, dynamic_range_high, prefers_dark, \
         pointer_type, timezone, viewing_distance_cm, ambient_light, css_px_per_mm, notes, \
         theme_slug, supported_codecs, codec_probe_cached, manifest_snapshot_id, study_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .bind(manifest_snapshot_id.as_deref())
    .bind(study.id)
    .execute(&state.pool)
    .await?;

    let total_trials: (i64,) = sqlx::query_as("SELECT total_trials FROM observers WHERE id = ?")
        .bind(&observer_id)
        .fetch_one(&state.pool)
        .await?;

    Ok(Json(CreateSessionResp {
        observer_id,
        session_id,
        study_id: study.id.to_string(),
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
    /// Corpus name from the manifest (`SourceMeta::corpus`). Used by the
    /// frontend to render a license badge on the trial.
    pub source_corpus: Option<String>,
    /// License-policy id for the source's corpus (e.g. "unsplash",
    /// "mixed-research"). The frontend looks up the full policy via
    /// `/api/curator/licenses` once and caches it.
    pub source_license_id: String,
    /// Human-readable license label for inline display.
    pub source_license_label: &'static str,
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

/// Replace the encodings in a `TrialPlan::Pair` with the highest-EIG adjacent
/// pair from the source's same-codec encoding ladder, using the BT-Davidson
/// fit over historical pair responses for that source. Silently returns the
/// input plan unchanged for:
///
/// - Single trials (ASAP has nothing to optimise).
/// - Golden / honeypot pair trials (their job is QC, not information gain).
/// - Sources with fewer than `ASAP_MIN_OBS` usable comparisons.
/// - DB errors (we log via `tracing` and keep the random pick).
async fn enhance_pair_with_asap(
    pool: &SqlitePool,
    manifest: &Manifest,
    plan: TrialPlan,
) -> TrialPlan {
    let TrialPlan::Pair {
        source,
        a,
        b,
        is_golden,
        expected_choice,
        held_out,
    } = plan
    else {
        return plan;
    };
    // Golden pairs are anchored on a fixed expected_choice — overriding the
    // encodings would invalidate the QC contract.
    if is_golden {
        return TrialPlan::Pair {
            source,
            a,
            b,
            is_golden,
            expected_choice,
            held_out,
        };
    }
    let codec = a.codec.clone();

    // Build the sorted ladder of same-codec encodings for this source.
    let mut sorted: Vec<EncodingMeta> = manifest
        .encodings_for(&source.hash)
        .into_iter()
        .filter(|e| e.codec == codec)
        .cloned()
        .collect();
    if sorted.len() < 2 {
        return TrialPlan::Pair {
            source,
            a,
            b,
            is_golden,
            expected_choice,
            held_out,
        };
    }
    sorted.sort_by(|x, y| {
        x.quality
            .unwrap_or(0.0)
            .partial_cmp(&y.quality.unwrap_or(0.0))
            .unwrap()
    });
    let id_to_idx: std::collections::HashMap<String, usize> = sorted
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id.clone(), i))
        .collect();

    // Pull prior pair responses for this source. Excludes held-out trials so
    // ASAP cannot leak the held-out condition bin into the active sample.
    // The 5000-row cap is a soft circuit-breaker; in practice we expect tens
    // of comparisons per source, hundreds at most.
    let rows: Vec<(String, Option<String>, String)> = match sqlx::query_as(
        "SELECT t.a_encoding_id, t.b_encoding_id, r.choice \
         FROM responses r \
         JOIN trials t ON t.id = r.trial_id \
         WHERE t.kind = 'pair' \
           AND t.source_hash = ? \
           AND t.held_out = 0 \
           AND t.b_encoding_id IS NOT NULL \
         LIMIT 5000",
    )
    .bind(&source.hash)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, source = %source.hash, "ASAP: pair-response query failed; falling back to random pair");
            return TrialPlan::Pair {
                source,
                a,
                b,
                is_golden,
                expected_choice,
                held_out,
            };
        }
    };

    let comps: Vec<crate::bt::Comparison> = rows
        .into_iter()
        .filter_map(|(aid, bid_opt, choice)| {
            let bid = bid_opt?;
            let i = *id_to_idx.get(&aid)?;
            let j = *id_to_idx.get(&bid)?;
            let outcome = match choice.as_str() {
                "a" => crate::bt::Outcome::AWins,
                "b" => crate::bt::Outcome::BWins,
                "tie" => crate::bt::Outcome::Tie,
                _ => return None,
            };
            Some(crate::bt::Comparison {
                a: i,
                b: j,
                outcome,
            })
        })
        .collect();

    let sorted_refs: Vec<&EncodingMeta> = sorted.iter().collect();
    let Some((i, j)) = select_pair_with_eig(&sorted_refs, &comps, ASAP_MIN_OBS) else {
        return TrialPlan::Pair {
            source,
            a,
            b,
            is_golden,
            expected_choice,
            held_out,
        };
    };
    TrialPlan::Pair {
        source,
        a: sorted[i].clone(),
        b: sorted[j].clone(),
        is_golden,
        expected_choice,
        held_out,
    }
}

pub async fn next_trial(
    State(state): State<SharedState>,
    Query(q): Query<NextTrialQuery>,
) -> Result<Json<TrialPayload>, AppError> {
    // Read the session's supported_codecs and filter the sampler accordingly.
    let row: Option<(Option<String>, String)> =
        sqlx::query_as("SELECT supported_codecs, study_id FROM sessions WHERE id = ?")
            .bind(&q.session_id)
            .fetch_optional(&state.pool)
            .await?;
    let (codecs_csv, study_id) = match row {
        Some((c, s)) => (c, s),
        None => (None, crate::studies::DEFAULT_STUDY_ID.to_string()),
    };
    let allowed: Option<std::collections::HashSet<String>> =
        codecs_csv.map(|s| s.split(',').map(str::trim).map(str::to_lowercase).collect());

    // The study owns the trial mix. A session recorded under a study that no
    // longer exists in the binary keeps working on the default rather than
    // 500ing, but says so — silently swapping protocols mid-study would be
    // worse than a loud log.
    let study = match crate::studies::by_id(&study_id) {
        Some(s) => s,
        None => {
            tracing::warn!(
                study_id,
                "session references an unknown study; using default mix"
            );
            crate::studies::default_study()
        }
    };

    let sampler = study.sampler;

    let manifest = state.manifest.read().await;
    let anchors = state.anchors.read().await;
    let flags = state.source_flags.read().await;
    let plan = pick_trial(
        &manifest,
        &sampler,
        allowed.as_ref(),
        Some(&*anchors),
        Some(&*flags),
    )
    .ok_or_else(|| {
        // Name the content restriction. A study that filters its pool and then
        // reports the generic message sends an operator to look at the sampler
        // or the codec list when the real answer is that the corpus has no
        // matching sources.
        // A mixed study draws from both classes, so "eligible" is the union —
        // counting against the unresolved filter would be meaningless (and
        // trips the debug assert in `accepts`).
        let eligible = manifest
            .sources
            .iter()
            .filter(|s| {
                let class = crate::content_class::classify(s.corpus.as_deref());
                sampler.content.resolve_for_draw(0.0).accepts(class)
                    || sampler.content.resolve_for_draw(1.0).accepts(class)
            })
            .count();
        AppError::Conflict(format!(
            "no trials available for study {study_id:?} ({}): {eligible} of {} manifest sources \
             match, and none of those had encodings this session's codecs can decode",
            sampler.content.describe(),
            manifest.sources.len(),
        ))
    })?;

    // ASAP active-sampling override: when `pick_trial` returns a non-golden Pair
    // and we have enough prior pair responses on this source to fit BT, replace
    // the random adjacent pair with the highest-EIG adjacent pair. Falls back
    // silently if the fit is under-determined.
    let plan = enhance_pair_with_asap(&state.pool, &manifest, plan).await;

    // Position counterbalancing, applied here and nowhere else: this is the one
    // point every pair passes through, whether it came from `try_pair` or the
    // ASAP override. Without it slot B held the higher-quality encoding on every
    // trial (measured 60/60 live), so "which is closer to the original" had the
    // same answer every time. See `sampling::counterbalance_pair`.
    let mut plan = crate::sampling::counterbalance_pair(plan, &mut rand::rng());

    // Test-retest control: sometimes re-serve a pair this observer already
    // answered in this session. Their agreement with themselves is the ceiling
    // any metric could reach, and without it the headline SROCC is
    // uninterpretable — see `Study::p_repeat`.
    //
    // Done here rather than in the sampler because it needs response history,
    // and the sampler is deliberately a pure function of the manifest.
    let mut repeat_of: Option<String> = None;
    if study.p_repeat > 0.0 && rand::Rng::random::<f32>(&mut rand::rng()) < study.p_repeat {
        let prior: Option<(String, String, String, String)> = sqlx::query_as(
            "SELECT t.id, t.source_hash, t.a_encoding_id, t.b_encoding_id \
             FROM trials t \
             JOIN responses r ON r.trial_id = t.id \
             JOIN sessions s ON s.id = t.session_id \
             WHERE s.observer_id = (SELECT observer_id FROM sessions WHERE id = ?) \
               AND s.study_id = ? AND t.kind = 'pair' AND t.b_encoding_id IS NOT NULL \
               AND t.repeat_of_trial_id IS NULL \
               AND t.id NOT IN (SELECT repeat_of_trial_id FROM trials \
                                WHERE repeat_of_trial_id IS NOT NULL) \
             ORDER BY RANDOM() LIMIT 1",
        )
        .bind(&q.session_id)
        .bind(&study_id)
        .fetch_optional(&state.pool)
        .await?;
        if let Some((prior_id, src_hash, a_id, b_id)) = prior {
            if let (Some(source), Some(a), Some(b)) = (
                manifest.source(&src_hash),
                manifest.encoding(&a_id),
                manifest.encoding(&b_id),
            ) {
                repeat_of = Some(prior_id);
                // Counterbalanced again, independently: a repeat that always
                // reproduced the original slot order would measure "did they
                // remember the layout" rather than "do they judge it the same".
                plan = crate::sampling::counterbalance_pair(
                    TrialPlan::Pair {
                        source: source.clone(),
                        a: a.clone(),
                        b: b.clone(),
                        is_golden: false,
                        expected_choice: None,
                        held_out: false,
                    },
                    &mut rand::rng(),
                );
            }
        }
    }

    let trial_id = Uuid::new_v4().to_string();
    let served_at = now_ms();

    let payload = match plan {
        TrialPlan::Single {
            source,
            encoding,
            staircase_target,
            is_golden,
            expected_choice,
            held_out,
        } => {
            sqlx::query(
                "INSERT INTO trials (id, session_id, kind, source_hash, a_encoding_id, a_codec, \
                 a_quality, a_bytes, intrinsic_w, intrinsic_h, staircase_target, is_golden, \
                 expected_choice, held_out, served_at, source_corpus, content_class, \
                 repeat_of_trial_id) \
                 VALUES (?, ?, 'single', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
            .bind(is_golden as i64)
            .bind(expected_choice.as_deref())
            .bind(held_out as i64)
            .bind(served_at)
            // Recorded as classified AT SERVE TIME. Deriving it at export time
            // would relabel history whenever the registry changed — which it
            // just did, when AI product shots moved from non-photo to photo.
            .bind(source.corpus.as_deref())
            .bind(crate::content_class::classify(source.corpus.as_deref()).as_str())
            .bind(repeat_of.as_deref())
            .execute(&state.pool)
            .await?;

            let policy = crate::licensing::lookup(source.corpus.as_deref().unwrap_or(""));
            TrialPayload {
                trial_id,
                kind: "single",
                source_hash: source.hash.clone(),
                source_url: format!("/api/proxy/source/{}", source.hash),
                source_w: source.width,
                source_h: source.height,
                source_corpus: source.corpus.clone(),
                source_license_id: policy.id.to_string(),
                source_license_label: policy.label,
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
        TrialPlan::Pair {
            source,
            a,
            b,
            is_golden,
            expected_choice,
            held_out,
        } => {
            sqlx::query(
                "INSERT INTO trials (id, session_id, kind, source_hash, a_encoding_id, a_codec, \
                 a_quality, a_bytes, b_encoding_id, b_codec, b_quality, b_bytes, intrinsic_w, \
                 intrinsic_h, is_golden, expected_choice, held_out, served_at, source_corpus, \
                 content_class, repeat_of_trial_id) \
                 VALUES (?, ?, 'pair', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
            .bind(is_golden as i64)
            .bind(expected_choice.as_deref())
            .bind(held_out as i64)
            .bind(served_at)
            // Recorded as classified AT SERVE TIME. Deriving it at export time
            // would relabel history whenever the registry changed — which it
            // just did, when AI product shots moved from non-photo to photo.
            .bind(source.corpus.as_deref())
            .bind(crate::content_class::classify(source.corpus.as_deref()).as_str())
            .bind(repeat_of.as_deref())
            .execute(&state.pool)
            .await?;

            let policy = crate::licensing::lookup(source.corpus.as_deref().unwrap_or(""));
            TrialPayload {
                trial_id,
                kind: "pair",
                source_hash: source.hash.clone(),
                source_url: format!("/api/proxy/source/{}", source.hash),
                source_w: source.width,
                source_h: source.height,
                source_corpus: source.corpus.clone(),
                source_license_id: policy.id.to_string(),
                source_license_label: policy.label,
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

fn one() -> f64 {
    1.0
}

fn tap_mode() -> String {
    "tap".to_string()
}

/// The input modes the UI can be driven in. An unrecognised value is refused
/// rather than stored: this column tells an analyst how to read
/// `reveal_ms_total`, so a typo silently persisted would quietly mislabel the
/// data it exists to disambiguate.
///
/// * `tap` — the encoding is on screen; tap A / B / Original, or press and hold
///   to peek at the reference. `reveal_ms_total` is a deliberate peek.
/// * `hold` — the reference is the resting view; press the left or right *half*
///   of the frame for A or B. `reveal_ms_total` measures the default state and
///   is naturally large.
/// * `buttons` — as `hold`, but the mouse *button* picks the side rather than
///   the position pressed. Desktop only.
///
/// Documented here rather than in migration 0017, whose comments described only
/// the first two: sqlx checksums migration files, so editing an applied one —
/// even its comments — fails every subsequent startup with "migration 17 was
/// previously applied but has been modified". Migrations are immutable once
/// shipped; prose about the column belongs next to the code that validates it.
pub const INPUT_MODES: &[&str] = &["tap", "hold", "buttons"];

#[derive(Debug, Deserialize)]
pub struct ResponseReq {
    pub choice: String,
    pub dwell_ms: i64,
    pub reveal_count: i64,
    pub reveal_ms_total: i64,
    pub zoom_used: bool,
    /// Panning telemetry. The stimulus renders at a hard minimum of 1:1 device
    /// pixels, so anything larger than the screen is only partly visible and
    /// `image_displayed_*` no longer describes what the observer looked at.
    /// Defaulted so an older client still records a valid response.
    #[serde(default)]
    pub pan_count: i64,
    #[serde(default)]
    pub pan_distance_css: f64,
    #[serde(default)]
    pub pannable_w_css: f64,
    #[serde(default)]
    pub pannable_h_css: f64,
    #[serde(default)]
    pub visible_w_css: f64,
    #[serde(default)]
    pub visible_h_css: f64,
    /// Magnification at response time; 1.0 = native 1:1. Integers only, and
    /// never below 1 — the display rule forbids downscaling.
    #[serde(default = "one")]
    pub zoom_factor: f64,
    pub viewport_w_css: i64,
    pub viewport_h_css: i64,
    pub orientation: String,
    pub image_displayed_w_css: f64,
    pub image_displayed_h_css: f64,
    pub intrinsic_to_device_ratio: f64,
    pub pixels_per_degree: Option<f64>,
    /// How the observer drove the UI: `tap` (segmented control, hold-to-reveal)
    /// or `hold` (reference at rest, mouse button flicks to A/B). It changes
    /// what `reveal_ms_total` measures — see migration 0017 — so it is stored
    /// rather than inferred. Unknown values are rejected, not coerced.
    #[serde(default = "tap_mode")]
    pub input_mode: String,
    #[serde(default)]
    pub keyboard_used: bool,
    /// Time from trial render to the judged image being painted. Kept out of
    /// `dwell_ms`'s interpretation: a slow first paint is not deliberation.
    #[serde(default)]
    pub ui_ready_ms: Option<i64>,
    /// Difficulty signal. `reveal_ms_total` only ever measured the reference,
    /// which under `hold`/`buttons` is the resting view — so it reflects "not
    /// pressing anything" rather than effort. These are per-view, raw, and
    /// deliberately un-normalised (see migration 0019).
    #[serde(default)]
    pub switch_count: i64,
    #[serde(default)]
    pub ms_on_a: i64,
    #[serde(default)]
    pub ms_on_b: i64,
    #[serde(default)]
    pub ms_on_ref: i64,
    /// When the UI first suggested "can't tell" on this trial, in ms from the
    /// trial appearing. `None` means it never did.
    ///
    /// Recorded because it is a nudge toward one specific answer, fired exactly
    /// on the trials where the answer is hardest — so tie rates on hinted
    /// trials are not comparable with unhinted ones unless you can tell them
    /// apart. See migration 0021.
    #[serde(default)]
    pub cant_tell_hint_ms: Option<i64>,
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
    // Already answered? Then this is a correction, not a new judgement.
    //
    // Allowed ONLY when it is the most recent response in the session. That is
    // the "I just misclicked" window; letting an observer reach back past
    // later trials would let them revise in light of what they saw afterwards,
    // which is a different and much less innocent thing.
    let existing: Option<(String, i64, Option<String>)> = sqlx::query_as(
        "SELECT r.choice, r.revision_count, r.original_choice FROM responses r \
         WHERE r.trial_id = ?",
    )
    .bind(&trial_id)
    .fetch_optional(&state.pool)
    .await?;
    if let Some((prev_choice, revisions, original)) = existing {
        let is_latest: Option<(i64,)> = sqlx::query_as(
            "SELECT COUNT(*) FROM responses r2 \
             JOIN trials t2 ON t2.id = r2.trial_id \
             WHERE t2.session_id = (SELECT session_id FROM trials WHERE id = ?) \
               AND r2.responded_at > (SELECT responded_at FROM responses WHERE trial_id = ?)",
        )
        .bind(&trial_id)
        .bind(&trial_id)
        .fetch_optional(&state.pool)
        .await?;
        if is_latest.map(|(n,)| n).unwrap_or(0) > 0 {
            return Err(AppError::Conflict(
                "only the most recent response can be corrected — later trials have \
                 been answered since this one"
                    .into(),
            ));
        }
        // The FIRST answer is preserved once and never overwritten again.
        //
        // `cant_tell_hint_ms` is COALESCEd for the same reason: the hint acted
        // on the answer that was actually given, so a correction must not erase
        // the fact that it fired. Reopening a trial builds fresh client state,
        // which would otherwise send NULL and quietly launder a hinted trial
        // into an unhinted one.
        let keep_original = original.unwrap_or(prev_choice);
        sqlx::query(
            "UPDATE responses SET choice = ?, original_choice = ?, revised_at = ?, \
             revision_count = ?, dwell_ms = ?, switch_count = ?, ms_on_a = ?, ms_on_b = ?, \
             ms_on_ref = ?, cant_tell_hint_ms = COALESCE(cant_tell_hint_ms, ?) \
             WHERE trial_id = ?",
        )
        .bind(&req.choice)
        .bind(&keep_original)
        .bind(now_ms())
        .bind(revisions + 1)
        .bind(req.dwell_ms)
        .bind(req.switch_count)
        .bind(req.ms_on_a)
        .bind(req.ms_on_b)
        .bind(req.ms_on_ref)
        .bind(req.cant_tell_hint_ms)
        .bind(&trial_id)
        .execute(&state.pool)
        .await?;
        // A correction does not re-count the trial: the observer answered once
        // and fixed it, which is one contribution, not two.
        let total: (i64,) = sqlx::query_as(
            "SELECT total_trials FROM observers WHERE id = \
             (SELECT observer_id FROM sessions WHERE id = \
              (SELECT session_id FROM trials WHERE id = ?))",
        )
        .bind(&trial_id)
        .fetch_one(&state.pool)
        .await
        .unwrap_or((0,));
        return Ok(Json(ResponseAck {
            total_trials: total.0.max(0) as u32,
            milestone_badge: None,
            flags: None,
        }));
    }

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
    if !INPUT_MODES.contains(&req.input_mode.as_str()) {
        return Err(AppError::BadRequest(format!(
            "unknown input_mode {:?}; known: {:?}",
            req.input_mode, INPUT_MODES
        )));
    }

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
        image_displayed_h_css: req.image_displayed_h_css,
        visible_w_css: req.visible_w_css,
        visible_h_css: req.visible_h_css,
        pan_count: req.pan_count,
        intrinsic_w,
        dpr: dpr_row.0,
    });

    sqlx::query(
        "INSERT INTO responses (trial_id, choice, dwell_ms, reveal_count, reveal_ms_total, \
         zoom_used, viewport_w_css, viewport_h_css, orientation, image_displayed_w_css, \
         image_displayed_h_css, intrinsic_to_device_ratio, pixels_per_degree, response_flags, \
         responded_at, pan_count, pan_distance_css, pannable_w_css, pannable_h_css, \
         visible_w_css, visible_h_css, zoom_factor, input_mode, keyboard_used, ui_ready_ms, \
         switch_count, ms_on_a, ms_on_b, ms_on_ref, cant_tell_hint_ms) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .bind(req.pan_count)
    .bind(req.pan_distance_css)
    .bind(req.pannable_w_css)
    .bind(req.pannable_h_css)
    .bind(req.visible_w_css)
    .bind(req.visible_h_css)
    .bind(req.zoom_factor.max(1.0))
    .bind(&req.input_mode)
    .bind(req.keyboard_used as i64)
    .bind(req.ui_ready_ms)
    .bind(req.switch_count)
    .bind(req.ms_on_a)
    .bind(req.ms_on_b)
    .bind(req.ms_on_ref)
    .bind(req.cant_tell_hint_ms)
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

/// The studies an observer may join. Unlisted ones are omitted; they are still
/// selectable by id for operator runs.
pub async fn list_studies() -> Json<Vec<&'static crate::studies::Study>> {
    Json(crate::studies::listed())
}

// ---------- export ----------

/// Build-time git commit baked in via `build.rs`. Every export manifest
/// carries this so "is this data still valid?" stays a single grep against
/// the source tree, not a forensic audit (CLAUDE.md ML-data §2).
///
/// Falls back to `"unknown"` when the build script didn't run (off-tree
/// builds — cargo-install, vendored). `option_env!`, not `env!`: the latter
/// is a hard compile error in that case, which cannot deliver this fallback
/// at all — it bricked the Docker build for two months.
pub const BUILD_COMMIT: &str = match option_env!("SQUINTLY_BUILD_COMMIT") {
    Some(c) => c,
    None => "unknown",
};

/// Per-export schema version. Bump when an export TSV's columns or
/// semantics change so downstream consumers can refuse stale shapes.
/// Bound separately for each export so a `pareto` schema bump doesn't
/// invalidate cached `responses` data.
fn schema_version(kind: ExportKind) -> u32 {
    match kind {
        ExportKind::Pareto => 1,
        ExportKind::Thresholds => 1,
        // v2 (2026-07-27): appended `study_id` (runtime study selection) plus
        // the pan/visible-area telemetry that the 1:1 display rule made
        // necessary. Appended rather than inserted so positional consumers
        // keep working; the bump is here so strict ones can refuse.
        ExportKind::Responses => 8,
        ExportKind::Unified => 1,
    }
}

#[derive(Debug, Clone, Copy)]
enum ExportKind {
    Pareto,
    Thresholds,
    Responses,
    Unified,
}

impl ExportKind {
    fn name(self) -> &'static str {
        match self {
            ExportKind::Pareto => "pareto",
            ExportKind::Thresholds => "thresholds",
            ExportKind::Responses => "responses",
            ExportKind::Unified => "unified",
        }
    }
    fn source_query(self) -> &'static str {
        match self {
            ExportKind::Pareto => "src/export.rs::pareto_tsv",
            ExportKind::Thresholds => "src/export.rs::thresholds_tsv",
            ExportKind::Responses => "src/export.rs::responses_tsv",
            ExportKind::Unified => "src/export.rs::unified_tsv",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ExportManifest {
    /// Identifier for the export — one of `pareto`/`thresholds`/`responses`/`unified`.
    kind: &'static str,
    /// Per-kind schema version. Bump when columns or semantics change.
    schema_version: u32,
    /// Git SHA the binary was built from (or `unknown` for off-tree builds).
    build_commit: &'static str,
    /// ISO-8601 UTC of when this manifest was produced.
    exported_at: String,
    /// Body row count (excludes the header row).
    row_count: u64,
    /// Hex-encoded SHA-256 of the TSV body (header included). Lets a
    /// downstream consumer detect silent corruption or transcoding without
    /// re-running the query.
    sha256: String,
    /// Pointer into the binary's source that produced the TSV — the
    /// reproduction recipe lives there, not duplicated in the manifest.
    source_query: &'static str,
    /// Byte size of the TSV body.
    body_bytes: u64,
    /// Reminder that exports are private. Squintly does not redistribute
    /// observer data; consumers must respect the same.
    redistribution: &'static str,
}

/// Build the manifest JSON for a given TSV body. Pure function — only the
/// body, kind, and current clock are inputs.
fn build_export_manifest(kind: ExportKind, body: &str) -> ExportManifest {
    use sha2::Digest;
    let row_count = body.lines().count().saturating_sub(1) as u64;
    let digest = sha2::Sha256::digest(body.as_bytes());
    let sha256 = digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    ExportManifest {
        kind: kind.name(),
        schema_version: schema_version(kind),
        build_commit: BUILD_COMMIT,
        exported_at: chrono::Utc::now().to_rfc3339(),
        row_count,
        sha256,
        source_query: kind.source_query(),
        body_bytes: body.len() as u64,
        redistribution: "private — not for redistribution",
    }
}

fn tsv_response(body: String, kind: ExportKind) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/tab-separated-values"),
    );
    // Link header points at the sidecar so a wget/curl/etc consumer that
    // follows Link rels gets the provenance pair, not just the bare TSV.
    headers.insert(
        header::LINK,
        HeaderValue::from_str(&format!(
            "</api/export/{}.manifest.json>; rel=\"describedby\"",
            kind.name()
        ))
        .unwrap(),
    );
    (StatusCode::OK, headers, body).into_response()
}

pub async fn export_pareto(State(state): State<SharedState>) -> Result<Response, AppError> {
    let body = crate::export::pareto_tsv(&state.pool).await?;
    Ok(tsv_response(body, ExportKind::Pareto))
}

pub async fn export_thresholds(State(state): State<SharedState>) -> Result<Response, AppError> {
    let body = crate::export::thresholds_tsv(&state.pool).await?;
    Ok(tsv_response(body, ExportKind::Thresholds))
}

pub async fn export_responses(State(state): State<SharedState>) -> Result<Response, AppError> {
    let body = crate::export::responses_tsv(&state.pool).await?;
    Ok(tsv_response(body, ExportKind::Responses))
}

pub async fn export_unified(State(state): State<SharedState>) -> Result<Response, AppError> {
    let body = crate::export::unified_tsv(&state.pool).await?;
    Ok(tsv_response(body, ExportKind::Unified))
}

pub async fn export_pareto_manifest(
    State(state): State<SharedState>,
) -> Result<Json<ExportManifest>, AppError> {
    let body = crate::export::pareto_tsv(&state.pool).await?;
    Ok(Json(build_export_manifest(ExportKind::Pareto, &body)))
}

pub async fn export_thresholds_manifest(
    State(state): State<SharedState>,
) -> Result<Json<ExportManifest>, AppError> {
    let body = crate::export::thresholds_tsv(&state.pool).await?;
    Ok(Json(build_export_manifest(ExportKind::Thresholds, &body)))
}

pub async fn export_responses_manifest(
    State(state): State<SharedState>,
) -> Result<Json<ExportManifest>, AppError> {
    let body = crate::export::responses_tsv(&state.pool).await?;
    Ok(Json(build_export_manifest(ExportKind::Responses, &body)))
}

pub async fn export_unified_manifest(
    State(state): State<SharedState>,
) -> Result<Json<ExportManifest>, AppError> {
    let body = crate::export::unified_tsv(&state.pool).await?;
    Ok(Json(build_export_manifest(ExportKind::Unified, &body)))
}

/// One reviewer's public row.
///
/// Every field answers "should I trust this reviewer's judgements, and how much
/// have they done" — the two things the board exists to convey. Nothing here
/// identifies a person: the handle is a salted, unreversible derivation (see
/// `handle.rs`), and no email, observer id or client address appears.
#[derive(Debug, Serialize)]
pub struct LeaderboardRow {
    pub handle: String,
    // --- work ---
    pub trials: i64,
    pub sessions: i64,
    pub active_days: i64,
    // --- quality ---
    /// Share of attention-check pairs answered correctly. `None` when none have
    /// been served yet — distinct from 0.0, which would be a failing reviewer.
    pub golden_pass_rate: Option<f32>,
    /// Agreement with THEMSELVES on re-served pairs. The reliability number
    /// that matters: it is the ceiling any metric could reach against this
    /// reviewer, so a reviewer with high volume and low self-agreement is
    /// contributing noise, not data.
    pub self_agreement: Option<f32>,
    pub repeat_pairs: i64,
    /// Median seconds per judgement. Read WITH `median_switches`: fast and
    /// decisive differs from fast and careless only by whether they looked.
    pub median_seconds: Option<f64>,
    /// Median view swaps per trial — how much comparing they actually did.
    pub median_switches: Option<f64>,
}

/// GET /api/leaderboard
///
/// Deliberately not ranked by volume alone. Sorting purely by trial count
/// rewards clicking through, which is the behaviour the honeypots exist to
/// catch; the client can sort by any column, and the payload carries the
/// quality fields needed to judge a high count.
pub async fn leaderboard(
    State(state): State<SharedState>,
) -> Result<Json<Vec<LeaderboardRow>>, AppError> {
    let salt = crate::handle::salt();
    let rows = sqlx::query(
        "SELECT s.observer_id AS oid, \
                COALESCE(o.email, s.observer_id) AS identity, \
                COUNT(*) AS trials, \
                COUNT(DISTINCT s.id) AS sessions, \
                COUNT(DISTINCT DATE(r.responded_at / 1000, 'unixepoch')) AS days \
         FROM responses r \
         JOIN trials t ON t.id = r.trial_id \
         JOIN sessions s ON s.id = t.session_id \
         JOIN observers o ON o.id = s.observer_id \
         GROUP BY s.observer_id HAVING trials > 0",
    )
    .fetch_all(&state.pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let oid: String = row.get("oid");
        let identity: String = row.get("identity");

        // Attention checks.
        let golden: Option<(i64, i64)> = sqlx::query_as(
            // First answer, for the same reason as `grading.rs`: undo must not
            // launder a failed attention check.
            "SELECT COUNT(*), SUM(CASE WHEN COALESCE(r.original_choice, r.choice) \
                                          = t.expected_choice THEN 1 ELSE 0 END) \
             FROM responses r JOIN trials t ON t.id = r.trial_id \
             JOIN sessions s ON s.id = t.session_id \
             WHERE s.observer_id = ? AND t.is_golden = 1 AND t.expected_choice IS NOT NULL",
        )
        .bind(&oid)
        .fetch_optional(&state.pool)
        .await?;

        // Test-retest: did they answer the repeat the same way as the original?
        // Slots are counterbalanced independently, so compare the ENCODING
        // chosen, never the slot letter — otherwise this would measure whether
        // they remembered the layout.
        let repeats: Vec<(String, String, String, String, String, String)> = sqlx::query_as(
            "SELECT rep.choice, t2.a_encoding_id, t2.b_encoding_id, \
                    orig.choice, t1.a_encoding_id, t1.b_encoding_id \
             FROM trials t2 \
             JOIN responses rep ON rep.trial_id = t2.id \
             JOIN trials t1 ON t1.id = t2.repeat_of_trial_id \
             JOIN responses orig ON orig.trial_id = t1.id \
             JOIN sessions s ON s.id = t2.session_id \
             WHERE s.observer_id = ? AND t2.repeat_of_trial_id IS NOT NULL",
        )
        .bind(&oid)
        .fetch_all(&state.pool)
        .await?;
        let chosen = |c: &str, a: &str, b: &str| -> Option<String> {
            match c {
                "a" => Some(a.to_string()),
                "b" => Some(b.to_string()),
                "tie" => Some("tie".to_string()),
                _ => None,
            }
        };
        let mut agree = 0i64;
        let mut comparable = 0i64;
        for (c2, a2, b2, c1, a1, b1) in &repeats {
            if let (Some(x), Some(y)) = (chosen(c2, a2, b2), chosen(c1, a1, b1)) {
                comparable += 1;
                if x == y {
                    agree += 1;
                }
            }
        }

        let timing: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT r.dwell_ms, r.switch_count FROM responses r \
             JOIN trials t ON t.id = r.trial_id \
             JOIN sessions s ON s.id = t.session_id \
             WHERE s.observer_id = ?",
        )
        .bind(&oid)
        .fetch_all(&state.pool)
        .await?;
        let median = |mut v: Vec<f64>| -> Option<f64> {
            if v.is_empty() {
                return None;
            }
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            Some(v[v.len() / 2])
        };

        out.push(LeaderboardRow {
            handle: crate::handle::handle_for(&identity, &salt),
            trials: row.get("trials"),
            sessions: row.get("sessions"),
            active_days: row.get("days"),
            golden_pass_rate: match golden {
                Some((n, ok)) if n > 0 => Some(ok as f32 / n as f32),
                _ => None,
            },
            self_agreement: (comparable > 0).then(|| agree as f32 / comparable as f32),
            repeat_pairs: comparable,
            median_seconds: median(timing.iter().map(|(d, _)| *d as f64 / 1000.0).collect())
                .map(|v| (v * 10.0).round() / 10.0),
            median_switches: median(timing.iter().map(|(_, s)| *s as f64).collect()),
        });
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.trials));
    Ok(Json(out))
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

/// Salt for the client-IP bucket.
///
/// Set `SQUINTLY_IP_HASH_SALT` in production. Unset, we generate one per
/// process: still unreversible, but the per-network counters reset on every
/// restart, so say so once rather than let a redeploy silently widen the limit.
fn ip_hash_salt() -> String {
    static SALT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SALT.get_or_init(|| {
        std::env::var("SQUINTLY_IP_HASH_SALT")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                tracing::warn!(
                    "SQUINTLY_IP_HASH_SALT unset — using a per-process salt, so per-network \
                     sign-in rate limits reset on restart"
                );
                generate_token()
            })
    })
    .clone()
}

pub async fn auth_start(
    State(state): State<SharedState>,
    headers: HeaderMap,
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
            "Email login is not configured on this deployment (POSTMARK_SERVER_TOKEN/FROM_EMAIL missing). \
             Anonymous use is unaffected."
                .into(),
        )
    })?;

    // Sign-in is open to any address, so this is the only thing between the
    // endpoint and an inbox. Counts come from `auth_tokens` itself — a row is
    // written for every accepted request, so the token store *is* the request
    // log and cannot disagree with it.
    let now = now_ms();
    let limit = RateLimit::from_env();
    let ip = client_ip(
        headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()),
        None,
    );
    let ip_hash = ip.as_deref().map(|s| hash_ip(s, &ip_hash_salt()));
    let hour_ago = now - 3_600_000;

    let last_for_email: Option<i64> =
        sqlx::query_scalar("SELECT MAX(created_at) FROM auth_tokens WHERE email = ?")
            .bind(&email)
            .fetch_one(&state.pool)
            .await?;
    let email_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM auth_tokens WHERE email = ? AND created_at >= ?")
            .bind(&email)
            .bind(hour_ago)
            .fetch_one(&state.pool)
            .await?;
    // No client address (direct hit with no proxy header) means no per-network
    // limit is enforceable; the per-address limits still are.
    let ip_count: i64 =
        match ip_hash.as_deref() {
            Some(h) => sqlx::query_scalar(
                "SELECT COUNT(*) FROM auth_tokens WHERE requester_ip_hash = ? AND created_at >= ?",
            )
            .bind(h)
            .bind(hour_ago)
            .fetch_one(&state.pool)
            .await?,
            None => 0,
        };

    if let RateVerdict::Deny {
        retry_after_s,
        reason,
    } = rate_verdict(&limit, now, last_for_email, email_count, ip_count)
    {
        tracing::warn!(
            email = %email,
            email_count_last_hour = email_count,
            ip_count_last_hour = ip_count,
            retry_after_s,
            "rate-limited a magic-link request"
        );
        return Err(AppError::TooManyRequests {
            retry_after_s,
            message: format!(
                "Slow down — {reason}. No link was sent. Anonymous use is unaffected."
            ),
        });
    }

    let token = generate_token();
    let token_hash = hash_token(&token);
    let expires_at = now + TOKEN_TTL_MS;

    sqlx::query(
        "INSERT INTO auth_tokens (token_hash, email, requesting_observer_id, expires_at, \
         consumed_at, created_at, requester_ip_hash) VALUES (?, ?, ?, ?, NULL, ?, ?)",
    )
    .bind(&token_hash)
    .bind(&email)
    .bind(req.observer_id.as_deref())
    .bind(expires_at)
    .bind(now)
    .bind(ip_hash.as_deref())
    .execute(&state.pool)
    .await?;

    let link = format!(
        "{}/api/auth/verify?token={}",
        req.origin.trim_end_matches('/'),
        token
    );
    if let Err(e) = send_magic_link(
        &cfg,
        EmailMessage {
            to: &email,
            link_url: &link,
        },
    )
    .await
    {
        // A recipient the mail provider won't deliver to is the caller's
        // problem, not a server fault — telling someone who typo'd their
        // address that the site is broken sends them to the wrong place.
        if e.downcast_ref::<SendFailure>().is_some() {
            tracing::warn!(email = %email, "mail provider refused the recipient");
            return Err(AppError::BadRequest(format!(
                "{email} was refused by our mail provider — it usually means the address \
                 doesn't exist or has previously bounced. Check the spelling, or use a \
                 different address. Anonymous use is unaffected."
            )));
        }
        return Err(e.into());
    }

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

    // Mint a real session. Until now verify handed the browser an observer id
    // and nothing more, so "signed in" was a claim only the client held and the
    // server had no way to check — which is why admin could only be a shared
    // token. The cookie is a second secret, stored hashed like the magic link.
    let session_token = generate_token();
    sqlx::query(
        "INSERT INTO auth_sessions (token_hash, observer_id, email, created_at, expires_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(hash_token(&session_token))
    .bind(&resolved_observer_id)
    .bind(&email)
    .bind(now)
    .bind(now + SESSION_TTL_MS)
    .execute(&state.pool)
    .await?;

    let mut resp = verify_page(VerifyOutcome::Success { email }, Some(resolved_observer_id));
    // The link is opened from a mail client, which may well be plain HTTP on a
    // dev box; a Secure cookie would be dropped there and sign-in would appear
    // to succeed while granting nothing.
    let secure = use_secure_cookies();
    if let Ok(v) = axum::http::HeaderValue::from_str(&session_cookie(
        &session_token,
        secure,
        SESSION_TTL_MS / 1000,
    )) {
        resp.headers_mut().append("set-cookie", v);
    }
    Ok(resp)
}

/// Whether to mark the session cookie `Secure`.
///
/// Defaults to on — a public deployment is HTTPS and marking it insecure there
/// would be a downgrade. `SQUINTLY_INSECURE_COOKIES=1` is the local-dev escape
/// hatch, named so it cannot be mistaken for something to set in production.
fn use_secure_cookies() -> bool {
    !std::env::var("SQUINTLY_INSECURE_COOKIES")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// The signed-in identity behind a request, if any.
pub struct SignedIn {
    pub observer_id: String,
    pub email: String,
    pub is_admin: bool,
}

/// Resolve the session cookie to an identity, and decide admin from the
/// *current* allowlist rather than anything stored at sign-in time — so
/// removing an address from `SQUINTLY_ADMIN_EMAILS` takes effect immediately.
pub async fn signed_in(
    pool: &sqlx::SqlitePool,
    headers: &HeaderMap,
) -> Result<Option<SignedIn>, AppError> {
    let Some(raw) = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(session_from_cookie_header)
    else {
        return Ok(None);
    };
    if raw.len() != 64 || !raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(None);
    }
    let row: Option<(String, String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT observer_id, email, expires_at, revoked_at FROM auth_sessions WHERE token_hash = ?",
    )
    .bind(hash_token(&raw))
    .fetch_optional(pool)
    .await?;
    let Some((observer_id, email, expires_at, revoked_at)) = row else {
        return Ok(None);
    };
    if revoked_at.is_some() || expires_at < now_ms() {
        return Ok(None);
    }
    let email = email.trim().to_ascii_lowercase();
    let is_admin = EmailAllowlist::admins().allows(&email);
    Ok(Some(SignedIn {
        observer_id,
        email,
        is_admin,
    }))
}

#[derive(Debug, Serialize)]
pub struct WhoAmIResp {
    pub signed_in: bool,
    pub email: Option<String>,
    pub observer_id: Option<String>,
    pub is_admin: bool,
}

/// GET /api/auth/whoami — lets the UI show admin controls only to admins.
/// Authoritative for display only; every privileged route re-checks.
pub async fn auth_whoami(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<WhoAmIResp>, AppError> {
    Ok(Json(match signed_in(&state.pool, &headers).await? {
        Some(s) => WhoAmIResp {
            signed_in: true,
            email: Some(s.email),
            observer_id: Some(s.observer_id),
            is_admin: s.is_admin,
        },
        None => WhoAmIResp {
            signed_in: false,
            email: None,
            observer_id: None,
            is_admin: false,
        },
    }))
}

/// POST /api/auth/signout — revoke the current session.
pub async fn auth_signout(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if let Some(raw) = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(session_from_cookie_header)
    {
        sqlx::query("UPDATE auth_sessions SET revoked_at = ? WHERE token_hash = ?")
            .bind(now_ms())
            .bind(hash_token(&raw))
            .execute(&state.pool)
            .await?;
    }
    let mut resp = (StatusCode::OK, "signed out").into_response();
    if let Ok(v) = axum::http::HeaderValue::from_str(&session_cookie("", true, 0)) {
        resp.headers_mut().append("set-cookie", v);
    }
    Ok(resp)
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

// ---------- onboarding calibration ----------
//
// Per docs/methodology.md §3.7: every session starts with up to 5 calibration
// trials with known answers and immediate feedback. Below 60% on calibration
// → observers.calibrated=0 (soft-fail; data is filtered at training time,
// not at session time).

#[derive(Debug, Serialize)]
pub struct CalibrationItem {
    pub id: String,
    pub kind: String,
    pub description: String,
    pub source_url: Option<String>,
    pub a_url: Option<String>,
    pub b_url: Option<String>,
    pub a_codec: Option<String>,
    pub b_codec: Option<String>,
    pub a_quality: Option<f32>,
    pub b_quality: Option<f32>,
    pub intrinsic_w: Option<i64>,
    pub intrinsic_h: Option<i64>,
    pub feedback_text: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CalibrationListResp {
    pub items: Vec<CalibrationItem>,
}

pub async fn calibration_list(
    State(state): State<SharedState>,
) -> Result<Json<CalibrationListResp>, AppError> {
    let rows = sqlx::query(
        "SELECT id, kind, description, source_url, a_url, b_url, a_codec, b_codec, \
                a_quality, b_quality, intrinsic_w, intrinsic_h, feedback_text \
         FROM calibration_pool ORDER BY order_hint, id LIMIT 5",
    )
    .fetch_all(&state.pool)
    .await?;
    let items = rows
        .into_iter()
        .map(|r| CalibrationItem {
            id: r.get(0),
            kind: r.get(1),
            description: r.get(2),
            source_url: r.get(3),
            a_url: r.get(4),
            b_url: r.get(5),
            a_codec: r.get(6),
            b_codec: r.get(7),
            a_quality: r.get::<Option<f64>, _>(8).map(|v| v as f32),
            b_quality: r.get::<Option<f64>, _>(9).map(|v| v as f32),
            intrinsic_w: r.get(10),
            intrinsic_h: r.get(11),
            feedback_text: r.get(12),
        })
        .collect();
    Ok(Json(CalibrationListResp { items }))
}

#[derive(Debug, Deserialize)]
pub struct CalibrationResponseReq {
    pub session_id: String,
    pub observer_id: String,
    pub pool_id: String,
    pub choice: String,
    pub dwell_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct CalibrationResponseAck {
    pub correct: bool,
    pub expected_choice: String,
    pub feedback_text: Option<String>,
}

pub async fn calibration_response(
    State(state): State<SharedState>,
    Json(req): Json<CalibrationResponseReq>,
) -> Result<Json<CalibrationResponseAck>, AppError> {
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT expected_choice, feedback_text FROM calibration_pool WHERE id = ?")
            .bind(&req.pool_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((expected_choice, feedback_text)) = row else {
        return Err(AppError::NotFound(format!(
            "calibration item {}",
            req.pool_id
        )));
    };
    let correct = expected_choice == req.choice;
    let now = now_ms();
    sqlx::query(
        "INSERT INTO calibration_responses (id, observer_id, session_id, pool_id, choice, \
         correct, dwell_ms, served_at, responded_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&req.observer_id)
    .bind(&req.session_id)
    .bind(&req.pool_id)
    .bind(&req.choice)
    .bind(correct as i64)
    .bind(req.dwell_ms)
    .bind(now)
    .bind(now)
    .execute(&state.pool)
    .await?;
    Ok(Json(CalibrationResponseAck {
        correct,
        expected_choice,
        feedback_text,
    }))
}

#[derive(Debug, Deserialize)]
pub struct CalibrationFinalizeReq {
    pub observer_id: String,
}

#[derive(Debug, Serialize)]
pub struct CalibrationFinalizeResp {
    pub calibrated: bool,
    pub score: f32,
}

pub async fn calibration_finalize(
    State(state): State<SharedState>,
    Json(req): Json<CalibrationFinalizeReq>,
) -> Result<Json<CalibrationFinalizeResp>, AppError> {
    let row: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(correct), 0) FROM calibration_responses \
         WHERE observer_id = ?",
    )
    .bind(&req.observer_id)
    .fetch_one(&state.pool)
    .await?;
    let (total, correct) = (row.0, row.1);
    let score = if total > 0 {
        correct as f32 / total as f32
    } else {
        0.0
    };
    let calibrated = score >= 0.60;
    sqlx::query(
        "UPDATE observers SET calibrated = ?, calibration_score = ?, calibrated_at = ? \
         WHERE id = ?",
    )
    .bind(calibrated as i64)
    .bind(score)
    .bind(now_ms())
    .bind(&req.observer_id)
    .execute(&state.pool)
    .await?;
    Ok(Json(CalibrationFinalizeResp { calibrated, score }))
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
    /// Email this observer linked via the magic-link flow, if any. Read-only;
    /// the frontend uses this to pre-fill suggestion forms etc.
    pub email: Option<String>,
    /// Unix-millis timestamp when the magic-link verification confirmed the
    /// email. Null when the observer is still anonymous.
    pub email_verified_at: Option<i64>,
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

type ProfileRow = (
    i64,
    Option<String>,
    i64,
    i64,
    Option<f64>,
    String,
    Option<String>,
    Option<i64>,
);

pub async fn observer_profile(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<ObserverProfile>, AppError> {
    let row: Option<ProfileRow> = sqlx::query_as(
        "SELECT streak_days, streak_last_date, freezes_remaining, total_trials, \
                skill_score, compensation_mode, email, email_verified_at \
         FROM observers WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.pool)
    .await?;
    let (
        streak_days,
        streak_last_date,
        freezes_remaining,
        total_trials,
        skill_score,
        comp_mode,
        email,
        email_verified_at,
    ) = match row {
        Some(r) => (r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7),
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
        email,
        email_verified_at,
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
    /// Git SHA this binary was built from (`unknown` for off-tree builds).
    ///
    /// Published here as well as on the export manifests because this endpoint
    /// is cheap and constant-cost, while a manifest computes its export to
    /// report a row count — so anything that just wants "which build is this?"
    /// (the trial screen's identifier panel, a deploy check, a bug report) must
    /// not be paying for an export to find out.
    pub build_commit: &'static str,
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
        build_commit: BUILD_COMMIT,
    }))
}

pub async fn refresh_manifest(State(state): State<SharedState>) -> Result<Json<Stats>, AppError> {
    let new_manifest = state.coefficient.refresh_manifest().await?;
    *state.manifest.write().await = new_manifest;
    // Also refresh anchors and source-flags — operators may have populated
    // them since startup.
    if let Ok(p) = load_anchor_pool(&state.pool).await {
        *state.anchors.write().await = p;
    }
    if let Ok(f) = load_source_flags(&state.pool).await {
        *state.source_flags.write().await = f;
    }
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
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("too many requests: {message}")]
    TooManyRequests { retry_after_s: i64, message: String },
    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (code, msg) = match &self {
            AppError::NotFound(s) => (StatusCode::NOT_FOUND, s.clone()),
            AppError::Conflict(s) => (StatusCode::CONFLICT, s.clone()),
            AppError::BadRequest(s) => (StatusCode::BAD_REQUEST, s.clone()),
            AppError::Forbidden(s) => (StatusCode::FORBIDDEN, s.clone()),
            // 429 carries Retry-After below; a bare status would leave the
            // caller guessing how long to wait, which is how clients end up
            // hammering a limiter.
            AppError::TooManyRequests { message, .. } => {
                (StatusCode::TOO_MANY_REQUESTS, message.clone())
            }
            AppError::ServiceUnavailable(s) => (StatusCode::SERVICE_UNAVAILABLE, s.clone()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        tracing::warn!(?code, %msg, "request failed");
        if let AppError::TooManyRequests { retry_after_s, .. } = &self {
            let mut r = (code, msg).into_response();
            if let Ok(v) = axum::http::HeaderValue::from_str(&retry_after_s.to_string()) {
                r.headers_mut().insert("retry-after", v);
            }
            return r;
        }
        (code, msg).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// New export columns must come with a schema bump, or a downstream
    /// consumer has no way to tell a v1 file from a v2 one.
    #[test]
    fn responses_schema_version_reflects_the_appended_columns() {
        assert_eq!(
            schema_version(ExportKind::Responses),
            8,
            "v2 added study_id + pan/visible telemetry; v3 the participant exclusion \
             disposition; v4 input_mode + keyboard_used + ui_ready_ms; v5 source_corpus + \
             content_class; v6 per-view dwell, switch_count and repeat_of_trial_id; \
             v7 response revisions; v8 cant_tell_hint_ms. Bump whenever columns change."
        );
        assert_eq!(schema_version(ExportKind::Pareto), 1, "pareto is unchanged");
    }

    #[test]
    fn export_manifest_hashes_body_and_counts_rows() {
        let body = "image_id\tsize\tconfig_name\n\
                    img1\tS\tmozjpeg-q40\n\
                    img2\tM\tzenwebp-q70\n\
                    img3\tL\tzenjxl-d2\n";
        let m = build_export_manifest(ExportKind::Pareto, body);
        assert_eq!(m.kind, "pareto");
        assert_eq!(m.schema_version, 1);
        assert_eq!(m.row_count, 3); // header excluded
        assert_eq!(m.body_bytes, body.len() as u64);
        // sha256 of the body should be deterministic and 64 hex chars.
        assert_eq!(m.sha256.len(), 64);
        let again = build_export_manifest(ExportKind::Pareto, body);
        assert_eq!(m.sha256, again.sha256, "sha256 should be deterministic");
        // Different body → different hash.
        let other = build_export_manifest(ExportKind::Pareto, "image_id\tsize\nimg1\tS\n");
        assert_ne!(m.sha256, other.sha256);
        assert_eq!(m.source_query, "src/export.rs::pareto_tsv");
        assert_eq!(m.redistribution, "private — not for redistribution");
    }

    #[test]
    fn export_manifest_handles_empty_body() {
        // Header-only TSV (no data rows) should report row_count = 0.
        let m = build_export_manifest(ExportKind::Responses, "trial_id\tsession_id\n");
        assert_eq!(m.row_count, 0);

        // Truly empty body → saturating_sub keeps row_count at 0.
        let m2 = build_export_manifest(ExportKind::Responses, "");
        assert_eq!(m2.row_count, 0);
    }
}
