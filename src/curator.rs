//! Corpus curator mode — see `docs/CORPUS_CURATOR_SPEC.md`.
//!
//! Squintly's curator mode is the human-in-the-loop that decides which images
//! enter which corpus groups (core/medium/full × zensim/encoding) and records
//! per-image `q_imperceptible` thresholds via the slider UI.
//!
//! This module owns:
//!
//! 1. Candidate-manifest ingestion. Two formats: corpus-builder TSV (`corpus`,
//!    `relative_path`, `width`, `height`, `size_bytes`, `suspected_category`,
//!    optional `has_alpha`) and the unified R2 JSONL manifest emitted by
//!    `scripts/upload_all.py` (`sha256`, `format`, `source`, `source_label`,
//!    `width`, `height`, `has_alpha`, `is_animated`, …). Both are parsed into
//!    [`Candidate`]s and inserted into `curator_candidates`.
//! 2. The HTTP route handlers under `/api/curator/*`.
//! 3. License surfacing — every candidate carries a license-policy id resolved
//!    from its corpus, fed back to the frontend on every stream/next response.
//!
//! The curator is anonymous: a UUID stored in the browser's localStorage
//! (same shape as the observer id used by the rating flow). One curator can
//! decide on a source at most once (UNIQUE in the schema); a second visit to
//! the same source updates the existing row.

use std::collections::HashMap;

use anyhow::{Context, Result};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::SqlitePool;

use crate::db::now_ms;
use crate::handlers::{AppError, SharedState};
use crate::licensing::{self, LicensePolicy};

// ---------- Candidate type ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub sha256: String,
    pub corpus: String,
    pub relative_path: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub size_bytes: Option<u64>,
    pub format: Option<String>,
    pub suspected_category: Option<String>,
    pub has_alpha: bool,
    pub has_animation: bool,
    /// License policy id resolved from `corpus`. Always set (falls back to
    /// `mixed-research` when unknown).
    pub license_id: String,
    /// Per-image attribution URL (only set when the manifest provided one).
    pub license_url: Option<String>,
    /// URL the browser should fetch to render the image. Empty if unknown.
    pub blob_url: String,
    pub order_hint: i64,
    /// libjpeg-style quality estimate (1-100) for JPEG sources, parsed from
    /// the DQT marker by `src/jpeg_q.rs`. Backfilled by
    /// `POST /api/curator/backfill-dims`. None for non-JPEG sources or rows
    /// not yet backfilled.
    pub source_q_detected: Option<f32>,
}

impl Candidate {
    fn license(&self) -> &'static LicensePolicy {
        licensing::by_id(&self.license_id)
    }
}

// ---------- TSV parser (corpus-builder) ----------

/// Parse a corpus-builder TSV (`# `-prefixed comment header followed by a
/// tab-separated table). Required columns: `corpus`, `relative_path`,
/// `width`, `height`, `size_bytes`. Optional: `suspected_category`,
/// `has_alpha`, `format`, `sha256`.
///
/// `blob_url_for_path` is invoked to produce the URL the browser will fetch
/// (e.g. `|p| format!("{base}/{p}", base = "/curator-blob")`). The TSV does
/// not generally carry a sha256 — pass `compute_sha` to compute one from
/// `(corpus, relative_path)` (deterministic placeholder hash for tests) or
/// supply an explicit hash via the manifest.
pub fn parse_tsv_manifest(
    body: &str,
    blob_url_for_path: impl Fn(&str, &str) -> String,
    compute_sha: impl Fn(&str, &str) -> String,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut header: Option<Vec<String>> = None;
    let mut order: i64 = 0;
    for line in body.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = trimmed.split('\t').collect();
        if header.is_none() {
            header = Some(cols.iter().map(|s| s.to_string()).collect());
            continue;
        }
        let h = header.as_ref().unwrap();
        let mut map: HashMap<&str, &str> = HashMap::new();
        for (i, val) in cols.iter().enumerate() {
            if let Some(name) = h.get(i) {
                map.insert(name.as_str(), val);
            }
        }
        let corpus = map.get("corpus").copied().unwrap_or("").to_string();
        let relative_path = map.get("relative_path").copied().unwrap_or("").to_string();
        if corpus.is_empty() || relative_path.is_empty() {
            continue;
        }
        let sha = map
            .get("sha256")
            .map(|s| s.to_string())
            .unwrap_or_else(|| compute_sha(&corpus, &relative_path));
        let policy = licensing::lookup(&corpus);
        out.push(Candidate {
            sha256: sha,
            corpus: corpus.clone(),
            relative_path: Some(relative_path.clone()),
            width: map.get("width").and_then(|s| s.parse::<u32>().ok()),
            height: map.get("height").and_then(|s| s.parse::<u32>().ok()),
            size_bytes: map.get("size_bytes").and_then(|s| s.parse::<u64>().ok()),
            format: map
                .get("format")
                .map(|s| s.to_string())
                .or_else(|| infer_format_from_path(&relative_path).map(|s| s.to_string())),
            suspected_category: map.get("suspected_category").map(|s| s.to_string()),
            has_alpha: map
                .get("has_alpha")
                .map(|s| matches!(*s, "1" | "true" | "True"))
                .unwrap_or(false),
            has_animation: map
                .get("is_animated")
                .map(|s| matches!(*s, "1" | "true" | "True"))
                .unwrap_or(false),
            license_id: policy.id.to_string(),
            license_url: map.get("license_url").map(|s| s.to_string()),
            blob_url: blob_url_for_path(&corpus, &relative_path),
            order_hint: order,
            source_q_detected: None,
        });
        order += 1;
    }
    out
}

/// Parse a JSONL manifest as emitted by corpus-builder's `upload_all.py`.
/// Each line is `{"sha256": …, "format": …, "source": …, "source_label": …,
/// "width": …, "height": …, …}`. `blob_url_for_sha` produces the URL the
/// browser uses for the bytes.
pub fn parse_jsonl_manifest(
    body: &str,
    blob_url_for_sha: impl Fn(&str) -> String,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut order: i64 = 0;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let sha = match v.get("sha256").and_then(|x| x.as_str()) {
            Some(s) if s.len() >= 32 => s.to_string(),
            _ => continue,
        };
        let corpus = v
            .get("source_label")
            .or_else(|| v.get("source"))
            .and_then(|x| x.as_str())
            .unwrap_or("unknown")
            .to_string();
        let policy = licensing::lookup(&corpus);
        out.push(Candidate {
            sha256: sha.clone(),
            corpus,
            relative_path: None,
            width: v.get("width").and_then(|x| x.as_u64()).map(|n| n as u32),
            height: v.get("height").and_then(|x| x.as_u64()).map(|n| n as u32),
            size_bytes: v.get("file_size").and_then(|x| x.as_u64()),
            format: v.get("format").and_then(|x| x.as_str()).map(str::to_string),
            suspected_category: v
                .get("primary_category")
                .or_else(|| v.get("suspected_category"))
                .and_then(|x| x.as_str())
                .map(str::to_string),
            has_alpha: v
                .get("has_alpha")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
            has_animation: v
                .get("is_animated")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
            license_id: policy.id.to_string(),
            license_url: v
                .get("license_url")
                .and_then(|x| x.as_str())
                .map(str::to_string),
            blob_url: blob_url_for_sha(&sha),
            order_hint: order,
            source_q_detected: None,
        });
        order += 1;
    }
    out
}

fn infer_format_from_path(p: &str) -> Option<&'static str> {
    let lower = p.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        Some("jpeg")
    } else if lower.ends_with(".png") {
        Some("png")
    } else if lower.ends_with(".webp") {
        Some("webp")
    } else if lower.ends_with(".avif") {
        Some("avif")
    } else if lower.ends_with(".jxl") {
        Some("jxl")
    } else if lower.ends_with(".gif") {
        Some("gif")
    } else {
        None
    }
}

/// Default R2 public URL for a content-addressed blob.
pub fn r2_blob_url(base: &str, sha: &str) -> String {
    if sha.len() < 4 {
        return format!("{base}/blobs/{sha}");
    }
    format!("{base}/blobs/{}/{}/{}", &sha[0..2], &sha[2..4], sha)
}

// ---------- DB persistence ----------

pub async fn upsert_candidates(pool: &SqlitePool, candidates: &[Candidate]) -> Result<u64> {
    let mut tx = pool.begin().await?;
    let mut inserted = 0u64;
    for c in candidates {
        sqlx::query(
            "INSERT INTO curator_candidates \
                (sha256, corpus, relative_path, width, height, size_bytes, \
                 format, suspected_category, has_alpha, has_animation, \
                 license_id, license_url, blob_url, order_hint) \
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(sha256) DO UPDATE SET \
                corpus = excluded.corpus, \
                relative_path = excluded.relative_path, \
                width = COALESCE(excluded.width, width), \
                height = COALESCE(excluded.height, height), \
                size_bytes = COALESCE(excluded.size_bytes, size_bytes), \
                format = COALESCE(excluded.format, format), \
                suspected_category = COALESCE(excluded.suspected_category, suspected_category), \
                has_alpha = excluded.has_alpha, \
                has_animation = excluded.has_animation, \
                license_id = excluded.license_id, \
                license_url = COALESCE(excluded.license_url, license_url), \
                blob_url = excluded.blob_url, \
                order_hint = excluded.order_hint",
        )
        .bind(&c.sha256)
        .bind(&c.corpus)
        .bind(c.relative_path.as_deref())
        .bind(c.width.map(|x| x as i64))
        .bind(c.height.map(|x| x as i64))
        .bind(c.size_bytes.map(|x| x as i64))
        .bind(c.format.as_deref())
        .bind(c.suspected_category.as_deref())
        .bind(if c.has_alpha { 1i64 } else { 0 })
        .bind(if c.has_animation { 1i64 } else { 0 })
        .bind(&c.license_id)
        .bind(c.license_url.as_deref())
        .bind(&c.blob_url)
        .bind(c.order_hint)
        .execute(&mut *tx)
        .await
        .context("inserting candidate")?;
        inserted += 1;
    }
    tx.commit().await?;
    Ok(inserted)
}

async fn fetch_candidate(pool: &SqlitePool, sha: &str) -> Result<Option<Candidate>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT sha256, corpus, relative_path, width, height, size_bytes, format, \
                suspected_category, has_alpha, has_animation, license_id, license_url, \
                blob_url, order_hint, source_q_detected \
         FROM curator_candidates WHERE sha256 = ?",
    )
    .bind(sha)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(row_to_candidate))
}

fn row_to_candidate(row: sqlx::sqlite::SqliteRow) -> Candidate {
    Candidate {
        sha256: row.get(0),
        corpus: row.get(1),
        relative_path: row.try_get(2).ok(),
        width: row
            .try_get::<Option<i64>, _>(3)
            .ok()
            .flatten()
            .map(|n| n as u32),
        height: row
            .try_get::<Option<i64>, _>(4)
            .ok()
            .flatten()
            .map(|n| n as u32),
        size_bytes: row
            .try_get::<Option<i64>, _>(5)
            .ok()
            .flatten()
            .map(|n| n as u64),
        format: row.try_get(6).ok(),
        suspected_category: row.try_get(7).ok(),
        has_alpha: row.try_get::<i64, _>(8).unwrap_or(0) != 0,
        has_animation: row.try_get::<i64, _>(9).unwrap_or(0) != 0,
        license_id: row
            .try_get(10)
            .unwrap_or_else(|_| "mixed-research".to_string()),
        license_url: row.try_get(11).ok(),
        blob_url: row.try_get(12).unwrap_or_default(),
        order_hint: row.try_get(13).unwrap_or(0),
        source_q_detected: row
            .try_get::<Option<f64>, _>(14)
            .ok()
            .flatten()
            .map(|q| q as f32),
    }
}

// ---------- Heuristics for default groups + size variants ----------

#[derive(Debug, Serialize)]
pub struct Suggestion {
    pub groups: Vec<&'static str>,
    pub sizes: Vec<u32>,
    pub recommended_max_dim: u32,
}

const DEFAULT_SIZE_CHIPS: &[u32] = &[64, 128, 256, 384, 512, 768, 1024, 1536];

pub fn suggest(c: &Candidate, source_q_detected: Option<f32>) -> Suggestion {
    let dim_known = c.width.is_some() && c.height.is_some();
    let max_native = c.width.unwrap_or(0).max(c.height.unwrap_or(0));
    let safe_max = if dim_known {
        match source_q_detected {
            Some(q) if q >= 95.0 => max_native,
            Some(q) if q >= 85.0 => max_native / 2,
            Some(q) if q >= 75.0 => max_native / 4,
            Some(q) if q >= 60.0 => max_native / 4,
            Some(_) => max_native / 8,
            None => max_native, // unknown q with known dims → assume safe
        }
    } else {
        u32::MAX
    };
    let groups = match (source_q_detected, c.format.as_deref()) {
        (Some(q), _) if q >= 95.0 => vec!["core_zensim", "core_encoding"],
        (_, Some("png" | "jxl")) => vec!["core_zensim", "core_encoding"],
        (Some(q), _) if q >= 85.0 => vec!["medium_zensim", "medium_encoding"],
        (Some(q), _) if q >= 70.0 => vec!["full_zensim", "full_encoding"],
        _ => vec![],
    };
    // When dims aren't known, surface every chip so the curator can decide
    // — the spec is explicit that greying out chips means "would upscale",
    // not "no info." Returning [] makes the UI useless.
    let sizes: Vec<u32> = if dim_known {
        DEFAULT_SIZE_CHIPS
            .iter()
            .copied()
            .filter(|d| *d <= safe_max && *d <= max_native.max(1))
            .collect()
    } else {
        DEFAULT_SIZE_CHIPS.to_vec()
    };
    Suggestion {
        groups,
        sizes,
        recommended_max_dim: if dim_known { safe_max } else { 0 },
    }
}

// ---------- bpp gate ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BppVerdict {
    /// Dimensions or size unknown — gate disabled.
    Unknown,
    /// Healthy bpp for the format.
    Ok,
    /// Suspiciously low bpp for the format — heavily compressed; flag for
    /// reconsideration (re-encoding will compound the artifacts).
    Low,
    /// Surprisingly high bpp for a lossy format — likely near-lossless or
    /// not actually compressed. Useful but not blocking.
    High,
}

#[derive(Debug, Clone, Serialize)]
pub struct BppGate {
    pub bpp: Option<f32>,
    pub verdict: BppVerdict,
    pub message: String,
}

/// Evaluate a candidate's bytes-per-pixel against per-format healthy ranges.
///
/// Lossy: jpeg/webp/avif/jxl. Healthy 0.3–4.0 bpp.
/// Lossless: png/gif. Healthy ≥ 1.0 bpp (well-encoded PNGs are 4-32).
/// Unknown format → use lossy ranges with wider bands.
pub fn bpp_gate(c: &Candidate) -> BppGate {
    let (w, h, size) = match (c.width, c.height, c.size_bytes) {
        (Some(w), Some(h), Some(s)) if w > 0 && h > 0 && s > 0 => (w as u64, h as u64, s),
        _ => {
            return BppGate {
                bpp: None,
                verdict: BppVerdict::Unknown,
                message: "bpp gate disabled — image dimensions or size unknown".to_string(),
            };
        }
    };
    let pixels = (w * h) as f64;
    let bits = (size as f64) * 8.0;
    let bpp = (bits / pixels) as f32;
    let lossless = matches!(c.format.as_deref(), Some("png" | "gif" | "apng"));
    let (verdict, message) = if lossless {
        if bpp < 1.0 {
            (
                BppVerdict::Low,
                format!(
                    "bpp = {bpp:.2} for {} — unusually low for a lossless format. \
                     Possibly an over-quantized PNG/GIF; check before training.",
                    c.format.as_deref().unwrap_or("?")
                ),
            )
        } else {
            (BppVerdict::Ok, format!("bpp = {bpp:.2} (lossless ✓)"))
        }
    } else if bpp < 0.3 {
        (
            BppVerdict::Low,
            format!(
                "bpp = {bpp:.2} — heavily compressed source. Re-encoding will \
                 compound artifacts; reject or only use at small target_max_dim."
            ),
        )
    } else if bpp > 4.0 {
        (
            BppVerdict::High,
            format!(
                "bpp = {bpp:.2} — near-lossless. Likely safe to use at any \
                 target size."
            ),
        )
    } else {
        (BppVerdict::Ok, format!("bpp = {bpp:.2} ✓"))
    };
    BppGate {
        bpp: Some(bpp),
        verdict,
        message,
    }
}

// ---------- HTTP handlers ----------

#[derive(Debug, Deserialize)]
pub struct StreamQuery {
    pub curator_id: Option<String>,
    pub source_q_detected: Option<f32>,
    /// Optional: skip this many decided sources (resume-style).
    pub skip: Option<i64>,
    /// Comma-separated corpus allow-list (e.g. `unsplash-webp,wide-gamut`).
    /// When set, only candidates whose corpus matches one of these strings
    /// are eligible. Settings-cog UI populates this from the user's
    /// preferences in localStorage.
    pub corpus: Option<String>,
    /// Comma-separated license-id allow-list (e.g. `unsplash,wikimedia-mixed`).
    pub license_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StreamResp {
    pub candidate: Option<Candidate>,
    pub license: Option<&'static LicensePolicy>,
    pub suggestion: Option<Suggestion>,
    pub bpp_gate: Option<BppGate>,
    pub remaining: i64,
    pub total: i64,
}

pub async fn stream_next(
    State(state): State<SharedState>,
    Query(q): Query<StreamQuery>,
) -> Result<Json<StreamResp>, AppError> {
    let curator_id = q.curator_id.unwrap_or_default();
    let skip = q.skip.unwrap_or(0).max(0);
    let corpus_filter: Vec<String> = parse_csv_filter(q.corpus.as_deref());
    let license_filter: Vec<String> = parse_csv_filter(q.license_id.as_deref());

    // Build the WHERE fragment dynamically; bind args after curator_id.
    let mut where_extra = String::new();
    if !corpus_filter.is_empty() {
        where_extra.push_str(" AND c.corpus IN (");
        for (i, _) in corpus_filter.iter().enumerate() {
            if i > 0 {
                where_extra.push(',');
            }
            where_extra.push('?');
        }
        where_extra.push(')');
    }
    if !license_filter.is_empty() {
        where_extra.push_str(" AND c.license_id IN (");
        for (i, _) in license_filter.iter().enumerate() {
            if i > 0 {
                where_extra.push(',');
            }
            where_extra.push('?');
        }
        where_extra.push(')');
    }

    // Total + decided counts honor the same filter so remaining reflects what
    // the curator actually sees.
    let total_sql = format!("SELECT COUNT(*) FROM curator_candidates c WHERE 1=1{where_extra}");
    let mut total_q = sqlx::query(&total_sql);
    for c in &corpus_filter {
        total_q = total_q.bind(c);
    }
    for l in &license_filter {
        total_q = total_q.bind(l);
    }
    let total: i64 = total_q.fetch_one(&state.pool).await?.get::<i64, _>(0);

    let row_sql = format!(
        "SELECT c.sha256, c.corpus, c.relative_path, c.width, c.height, c.size_bytes, \
                c.format, c.suspected_category, c.has_alpha, c.has_animation, c.license_id, \
                c.license_url, c.blob_url, c.order_hint, c.source_q_detected \
         FROM curator_candidates c \
         LEFT JOIN curator_decisions d \
           ON d.source_sha256 = c.sha256 AND d.curator_id = ? \
         WHERE d.id IS NULL{where_extra} \
         ORDER BY c.order_hint, c.sha256 \
         LIMIT 1 OFFSET ?"
    );
    let mut row_q = sqlx::query(&row_sql).bind(&curator_id);
    for c in &corpus_filter {
        row_q = row_q.bind(c);
    }
    for l in &license_filter {
        row_q = row_q.bind(l);
    }
    let row = row_q.bind(skip).fetch_optional(&state.pool).await?;

    let decided_sql = format!(
        "SELECT COUNT(*) FROM curator_decisions d \
         JOIN curator_candidates c ON c.sha256 = d.source_sha256 \
         WHERE d.curator_id = ?{where_extra}"
    );
    let mut decided_q = sqlx::query(&decided_sql).bind(&curator_id);
    for c in &corpus_filter {
        decided_q = decided_q.bind(c);
    }
    for l in &license_filter {
        decided_q = decided_q.bind(l);
    }
    let decided: i64 = decided_q.fetch_one(&state.pool).await?.get::<i64, _>(0);
    let remaining = (total - decided).max(0);

    if let Some(row) = row {
        let c = row_to_candidate(row);
        // Per-row detected q wins unless the caller explicitly overrides via
        // ?source_q_detected= (useful for ad-hoc what-ifs from the UI).
        let q_for_suggest = q.source_q_detected.or(c.source_q_detected);
        let suggestion = suggest(&c, q_for_suggest);
        let gate = bpp_gate(&c);
        let lic = c.license();
        Ok(Json(StreamResp {
            candidate: Some(c),
            license: Some(lic),
            suggestion: Some(suggestion),
            bpp_gate: Some(gate),
            remaining,
            total,
        }))
    } else {
        Ok(Json(StreamResp {
            candidate: None,
            license: None,
            suggestion: None,
            bpp_gate: None,
            remaining: 0,
            total,
        }))
    }
}

#[derive(Debug, Deserialize)]
pub struct DecisionReq {
    pub source_sha256: String,
    pub curator_id: String,
    pub decision: String, // 'take' | 'reject' | 'flag'
    pub reject_reason: Option<String>,
    pub groups: Option<DecisionGroups>,
    pub sizes: Option<Vec<u32>>,
    pub source_q_detected: Option<f32>,
    pub recommended_max_dim: Option<u32>,
    pub source_codec: Option<String>,
    pub decision_dpr: Option<f64>,
    pub decision_viewport_w: Option<i64>,
    pub decision_viewport_h: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
pub struct DecisionGroups {
    #[serde(default)]
    pub core_zensim: bool,
    #[serde(default)]
    pub medium_zensim: bool,
    #[serde(default)]
    pub full_zensim: bool,
    #[serde(default)]
    pub core_encoding: bool,
    #[serde(default)]
    pub medium_encoding: bool,
    #[serde(default)]
    pub full_encoding: bool,
}

#[derive(Debug, Serialize)]
pub struct DecisionResp {
    pub decision_id: i64,
    pub took: bool,
}

/// `POST /api/curator/generate-variant` — runs the decode → Mitchell-resize
/// → JPEG-encode → R2-upload pipeline for a chosen (decision, target_max_dim)
/// pair. Updates `curator_size_variants` with the generated sha256 + path.
///
/// Quality defaults to `q_imperceptible` from the matching threshold row when
/// available, else 92. Caller can override via `quality`.
///
/// AVIF and JXL sources currently return 409 (decoders not yet wired); JPEG,
/// PNG and WebP work.
#[derive(Debug, Deserialize)]
pub struct GenerateVariantReq {
    pub decision_id: i64,
    pub target_max_dim: u32,
    /// Output format: "png" (default, lossless RGBA8 — spec-correct for
    /// ground-truth corpus material) or "jpeg" (lossy; preview/thumbnail
    /// only — never feed back into training as ground truth).
    /// Case-insensitive.
    #[serde(default)]
    pub format: Option<String>,
    /// JPEG quality (1..100). Only honored when `format = "jpeg"`. Defaults
    /// to the matching threshold's `q_imperceptible`, else 92.
    pub quality: Option<u8>,
}

#[derive(Debug, Serialize)]
pub struct GenerateVariantResp {
    pub ok: bool,
    pub generated_sha256: String,
    pub generated_url: String,
    pub width: u32,
    pub height: u32,
    pub size_bytes: usize,
    /// "png-rgba8-lossless" or "jpeg-qNN" — written to
    /// `curator_size_variants.encoder_label` for downstream filtering.
    pub encoder_label: String,
}

pub async fn generate_variant(
    State(state): State<SharedState>,
    Json(req): Json<GenerateVariantReq>,
) -> Result<Json<GenerateVariantResp>, AppError> {
    if !(16..=4096).contains(&req.target_max_dim) {
        return Err(AppError::BadRequest(
            "target_max_dim must be in [16, 4096]".into(),
        ));
    }
    let row = sqlx::query(
        "SELECT c.sha256, c.blob_url, c.format \
         FROM curator_decisions d \
         JOIN curator_candidates c ON c.sha256 = d.source_sha256 \
         WHERE d.id = ?",
    )
    .bind(req.decision_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("decision {}", req.decision_id)))?;
    let _source_sha: String = row.get(0);
    let blob_url: String = row.get(1);
    let format: Option<String> = row.try_get(2).ok();

    // Resolve the output format. Default = PNG (lossless, ground-truth-safe).
    // JPEG only when explicitly requested.
    let format_lc = req.format.as_deref().map(|s| s.to_ascii_lowercase());
    let out_format = match format_lc.as_deref() {
        None | Some("png") => crate::variant_gen::VariantFormat::Png,
        Some("jpeg") | Some("jpg") => {
            // Resolve quality: explicit override → threshold-q → 92.
            let q = if let Some(q) = req.quality {
                q
            } else {
                let threshold_q: Option<f64> = match sqlx::query(
                    "SELECT q_imperceptible FROM curator_thresholds \
                     WHERE decision_id = ? AND target_max_dim = ?",
                )
                .bind(req.decision_id)
                .bind(req.target_max_dim as i64)
                .fetch_optional(&state.pool)
                .await?
                {
                    Some(r) => r.try_get::<f64, _>(0).ok(),
                    None => None,
                };
                threshold_q
                    .map(|q| q.round().clamp(1.0, 100.0) as u8)
                    .unwrap_or(92)
            };
            crate::variant_gen::VariantFormat::Jpeg { quality: q }
        }
        Some(other) => {
            return Err(AppError::BadRequest(format!(
                "format must be 'png' or 'jpeg', got {other}"
            )));
        } // exhaustive: None handled by the first arm
    };

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("http client: {e}")))?;
    let bytes = crate::variant_gen::fetch_source(&http, &blob_url)
        .await
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!(e.to_string())))?;

    let variant = match crate::variant_gen::generate(
        &bytes,
        format.as_deref(),
        req.target_max_dim,
        out_format,
    ) {
        Ok(v) => v,
        Err(crate::variant_gen::VariantError::UnsupportedFormat(f)) => {
            return Err(AppError::Conflict(format!(
                "source format {f} not supported by variant pipeline (jpeg/png/webp/avif/jxl)"
            )));
        }
        Err(other) => {
            return Err(AppError::Anyhow(anyhow::anyhow!(other.to_string())));
        }
    };

    let stored = state
        .suggestions
        .put_with_prefix(
            "variants",
            &variant.sha256,
            variant.format.ext(),
            &variant.bytes,
            variant.mime,
        )
        .await
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("upload: {e}")))?;
    let url = state
        .suggestions
        .public_url(&stored.locator)
        .unwrap_or_else(|| format!("/api/curator/variant-blob/{}", variant.sha256));

    sqlx::query(
        "INSERT OR REPLACE INTO curator_size_variants \
            (decision_id, target_max_dim, generated_sha256, generated_path) \
         VALUES (?,?,?,?)",
    )
    .bind(req.decision_id)
    .bind(req.target_max_dim as i64)
    .bind(&variant.sha256)
    .bind(&url)
    .execute(&state.pool)
    .await?;

    let label = variant.format.label();
    Ok(Json(GenerateVariantResp {
        ok: true,
        generated_sha256: variant.sha256,
        generated_url: url,
        width: variant.width,
        height: variant.height,
        size_bytes: variant.bytes.len(),
        encoder_label: label,
    }))
}

/// `POST /api/curator/decision/undo` — drops the last decision for the given
/// curator+source, restoring the candidate to undecided. Cascade-deletes any
/// size variants and thresholds attached to that decision (FK ON DELETE
/// CASCADE in 0007_curator.sql). The caller picks which sha to undo —
/// frontend tracks "last decided" in memory rather than us doing time-based
/// guessing.
#[derive(Debug, Deserialize)]
pub struct UndoReq {
    pub curator_id: String,
    /// Optional: when omitted, undo the most-recently-decided source for
    /// this curator (from `decided_at`). Provide explicitly when the client
    /// wants to undo a specific earlier decision.
    pub source_sha256: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UndoResp {
    pub undone: bool,
    pub source_sha256: Option<String>,
    pub had_threshold: bool,
}

pub async fn undo_decision(
    State(state): State<SharedState>,
    Json(req): Json<UndoReq>,
) -> Result<Json<UndoResp>, AppError> {
    if req.curator_id.is_empty() {
        return Err(AppError::BadRequest("curator_id required".into()));
    }
    // Find the target row.
    let row = if let Some(sha) = req.source_sha256.as_deref() {
        sqlx::query(
            "SELECT id, source_sha256 FROM curator_decisions \
             WHERE curator_id = ? AND source_sha256 = ? \
             ORDER BY decided_at DESC LIMIT 1",
        )
        .bind(&req.curator_id)
        .bind(sha)
        .fetch_optional(&state.pool)
        .await?
    } else {
        sqlx::query(
            "SELECT id, source_sha256 FROM curator_decisions \
             WHERE curator_id = ? ORDER BY decided_at DESC LIMIT 1",
        )
        .bind(&req.curator_id)
        .fetch_optional(&state.pool)
        .await?
    };
    let row = match row {
        Some(r) => r,
        None => {
            return Ok(Json(UndoResp {
                undone: false,
                source_sha256: None,
                had_threshold: false,
            }));
        }
    };
    let id: i64 = row.get(0);
    let sha: String = row.get(1);
    let threshold_count: i64 =
        sqlx::query("SELECT COUNT(*) FROM curator_thresholds WHERE decision_id = ?")
            .bind(id)
            .fetch_one(&state.pool)
            .await?
            .get::<i64, _>(0);
    sqlx::query("DELETE FROM curator_decisions WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(Json(UndoResp {
        undone: true,
        source_sha256: Some(sha),
        had_threshold: threshold_count > 0,
    }))
}

pub async fn decision(
    State(state): State<SharedState>,
    Json(req): Json<DecisionReq>,
) -> Result<Json<DecisionResp>, AppError> {
    if req.curator_id.is_empty() {
        return Err(AppError::BadRequest("curator_id required".into()));
    }
    if !matches!(req.decision.as_str(), "take" | "reject" | "flag") {
        return Err(AppError::BadRequest(
            "decision must be take/reject/flag".into(),
        ));
    }
    let cand = fetch_candidate(&state.pool, &req.source_sha256)
        .await?
        .ok_or_else(|| AppError::NotFound("source not in candidate manifest".into()))?;
    let groups = req.groups.unwrap_or_default();
    let now = now_ms();

    let row = sqlx::query(
        "INSERT INTO curator_decisions \
            (source_sha256, curator_id, decided_at, decision, reject_reason, \
             in_core_zensim, in_medium_zensim, in_full_zensim, \
             in_core_encoding, in_medium_encoding, in_full_encoding, \
             source_codec, source_q_detected, source_w, source_h, recommended_max_dim, \
             decision_dpr, decision_viewport_w, decision_viewport_h) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) \
         ON CONFLICT(source_sha256, curator_id) DO UPDATE SET \
            decision = excluded.decision, \
            reject_reason = excluded.reject_reason, \
            in_core_zensim = excluded.in_core_zensim, \
            in_medium_zensim = excluded.in_medium_zensim, \
            in_full_zensim = excluded.in_full_zensim, \
            in_core_encoding = excluded.in_core_encoding, \
            in_medium_encoding = excluded.in_medium_encoding, \
            in_full_encoding = excluded.in_full_encoding, \
            source_codec = excluded.source_codec, \
            source_q_detected = excluded.source_q_detected, \
            recommended_max_dim = excluded.recommended_max_dim, \
            decision_dpr = excluded.decision_dpr, \
            decision_viewport_w = excluded.decision_viewport_w, \
            decision_viewport_h = excluded.decision_viewport_h, \
            decided_at = excluded.decided_at \
         RETURNING id",
    )
    .bind(&req.source_sha256)
    .bind(&req.curator_id)
    .bind(now)
    .bind(&req.decision)
    .bind(req.reject_reason.as_deref())
    .bind(if groups.core_zensim { 1i64 } else { 0 })
    .bind(if groups.medium_zensim { 1i64 } else { 0 })
    .bind(if groups.full_zensim { 1i64 } else { 0 })
    .bind(if groups.core_encoding { 1i64 } else { 0 })
    .bind(if groups.medium_encoding { 1i64 } else { 0 })
    .bind(if groups.full_encoding { 1i64 } else { 0 })
    .bind(req.source_codec.as_deref())
    .bind(req.source_q_detected.map(|x| x as f64))
    .bind(cand.width.unwrap_or(0) as i64)
    .bind(cand.height.unwrap_or(0) as i64)
    .bind(req.recommended_max_dim.map(|x| x as i64))
    .bind(req.decision_dpr)
    .bind(req.decision_viewport_w)
    .bind(req.decision_viewport_h)
    .fetch_one(&state.pool)
    .await?;
    let decision_id: i64 = row.get(0);

    if req.decision == "take" {
        if let Some(sizes) = req.sizes {
            for d in sizes.iter().copied() {
                sqlx::query(
                    "INSERT OR IGNORE INTO curator_size_variants (decision_id, target_max_dim) \
                     VALUES (?, ?)",
                )
                .bind(decision_id)
                .bind(d as i64)
                .execute(&state.pool)
                .await?;
            }
        }
    }

    Ok(Json(DecisionResp {
        decision_id,
        took: req.decision == "take",
    }))
}

#[derive(Debug, Deserialize)]
pub struct ThresholdReq {
    pub decision_id: i64,
    pub target_max_dim: u32,
    pub q_imperceptible: f32,
    pub measurement_dpr: f32,
    pub measurement_distance_cm: Option<f32>,
    pub encoder_label: Option<String>,
}

pub async fn threshold(
    State(state): State<SharedState>,
    Json(req): Json<ThresholdReq>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !(0.0..=100.0).contains(&req.q_imperceptible) {
        return Err(AppError::BadRequest(
            "q_imperceptible must be in [0, 100]".into(),
        ));
    }
    let now = now_ms();
    sqlx::query(
        "INSERT INTO curator_thresholds \
            (decision_id, target_max_dim, q_imperceptible, measured_at, \
             measurement_dpr, measurement_distance_cm, encoder_label) \
         VALUES (?,?,?,?,?,?,?) \
         ON CONFLICT(decision_id, target_max_dim) DO UPDATE SET \
             q_imperceptible = excluded.q_imperceptible, \
             measured_at = excluded.measured_at, \
             measurement_dpr = excluded.measurement_dpr, \
             measurement_distance_cm = excluded.measurement_distance_cm, \
             encoder_label = excluded.encoder_label",
    )
    .bind(req.decision_id)
    .bind(req.target_max_dim as i64)
    .bind(req.q_imperceptible as f64)
    .bind(now)
    .bind(req.measurement_dpr as f64)
    .bind(req.measurement_distance_cm.map(|x| x as f64))
    .bind(
        req.encoder_label
            .unwrap_or_else(|| "browser-canvas-jpeg".to_string()),
    )
    .execute(&state.pool)
    .await?;
    Ok(Json(serde_json::json!({"ok": true})))
}

#[derive(Debug, Deserialize)]
pub struct ProgressQuery {
    pub curator_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProgressResp {
    pub total_candidates: i64,
    pub decisions: i64,
    pub takes: i64,
    pub rejects: i64,
    pub flags: i64,
    pub thresholds: i64,
    pub by_corpus: Vec<CorpusProgress>,
}

#[derive(Debug, Serialize)]
pub struct CorpusProgress {
    pub corpus: String,
    pub total: i64,
    pub decided: i64,
}

pub async fn progress(
    State(state): State<SharedState>,
    Query(q): Query<ProgressQuery>,
) -> Result<Json<ProgressResp>, AppError> {
    let curator_id = q.curator_id.unwrap_or_default();
    let total: i64 = sqlx::query("SELECT COUNT(*) FROM curator_candidates")
        .fetch_one(&state.pool)
        .await
        .map(|r| r.get::<i64, _>(0))?;
    let row = sqlx::query(
        "SELECT \
            COUNT(*), \
            SUM(CASE WHEN decision = 'take' THEN 1 ELSE 0 END), \
            SUM(CASE WHEN decision = 'reject' THEN 1 ELSE 0 END), \
            SUM(CASE WHEN decision = 'flag' THEN 1 ELSE 0 END) \
         FROM curator_decisions WHERE curator_id = ?",
    )
    .bind(&curator_id)
    .fetch_one(&state.pool)
    .await?;
    let decisions: i64 = row.try_get(0).unwrap_or(0);
    let takes: i64 = row.try_get(1).unwrap_or(0);
    let rejects: i64 = row.try_get(2).unwrap_or(0);
    let flags: i64 = row.try_get(3).unwrap_or(0);
    let thresholds: i64 = sqlx::query(
        "SELECT COUNT(*) FROM curator_thresholds t \
         JOIN curator_decisions d ON d.id = t.decision_id \
         WHERE d.curator_id = ?",
    )
    .bind(&curator_id)
    .fetch_one(&state.pool)
    .await
    .map(|r| r.get::<i64, _>(0))?;
    let by_corpus_rows = sqlx::query(
        "SELECT c.corpus, COUNT(*) AS total, \
                COALESCE(SUM(CASE WHEN d.id IS NULL THEN 0 ELSE 1 END), 0) AS decided \
         FROM curator_candidates c \
         LEFT JOIN curator_decisions d ON d.source_sha256 = c.sha256 AND d.curator_id = ? \
         GROUP BY c.corpus ORDER BY c.corpus",
    )
    .bind(&curator_id)
    .fetch_all(&state.pool)
    .await?;
    let by_corpus = by_corpus_rows
        .into_iter()
        .map(|r| CorpusProgress {
            corpus: r.try_get(0).unwrap_or_default(),
            total: r.try_get(1).unwrap_or(0),
            decided: r.try_get(2).unwrap_or(0),
        })
        .collect();
    Ok(Json(ProgressResp {
        total_candidates: total,
        decisions,
        takes,
        rejects,
        flags,
        thresholds,
        by_corpus,
    }))
}

/// `GET /api/curator/licenses` — emit the license registry for the welcome /
/// credits panel. Always-on; doesn't need any DB state.
pub async fn license_registry() -> Json<Vec<&'static LicensePolicy>> {
    Json(licensing::all_policies().to_vec())
}

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    pub curator_id: Option<String>,
}

/// `GET /api/curator/export.tsv` — every decision joined with size variants,
/// thresholds, and per-source license metadata.
pub async fn export_tsv(
    State(state): State<SharedState>,
    Query(q): Query<ExportQuery>,
) -> Result<Response, AppError> {
    let mut sql = String::from(
        "SELECT d.id, d.source_sha256, d.curator_id, d.decided_at, d.decision, \
                d.in_core_zensim, d.in_medium_zensim, d.in_full_zensim, \
                d.in_core_encoding, d.in_medium_encoding, d.in_full_encoding, \
                d.source_codec, d.source_q_detected, d.source_w, d.source_h, \
                d.recommended_max_dim, c.corpus, c.relative_path, c.format, \
                c.license_id, c.license_url, \
                t.target_max_dim, t.q_imperceptible, t.measurement_dpr, \
                t.measurement_distance_cm, t.encoder_label \
         FROM curator_decisions d \
         JOIN curator_candidates c ON c.sha256 = d.source_sha256 \
         LEFT JOIN curator_thresholds t ON t.decision_id = d.id ",
    );
    if q.curator_id.is_some() {
        sql.push_str("WHERE d.curator_id = ? ");
    }
    sql.push_str("ORDER BY d.decided_at, d.id, t.target_max_dim");
    let mut query = sqlx::query(&sql);
    if let Some(id) = q.curator_id.as_deref() {
        query = query.bind(id);
    }
    let rows = query.fetch_all(&state.pool).await?;

    let header = "decision_id\tsource_sha256\tcurator_id\tdecided_at_ms\tdecision\t\
        groups\tsource_codec\tsource_q_detected\tsource_w\tsource_h\t\
        recommended_max_dim\tcorpus\trelative_path\tformat\t\
        license_id\tlicense_label\tlicense_terms_url\tlicense_attribution_url\t\
        license_redistribute\tlicense_commercial_training\t\
        target_max_dim\tq_imperceptible\tmeasurement_dpr\tmeasurement_distance_cm\tencoder_label\n";
    let mut body = String::new();
    body.push_str(header);
    for row in rows {
        let decision_id: i64 = row.try_get(0).unwrap_or(0);
        let sha: String = row.try_get(1).unwrap_or_default();
        let curator_id: String = row.try_get(2).unwrap_or_default();
        let decided_at: i64 = row.try_get(3).unwrap_or(0);
        let decision: String = row.try_get(4).unwrap_or_default();
        let in_cz: i64 = row.try_get(5).unwrap_or(0);
        let in_mz: i64 = row.try_get(6).unwrap_or(0);
        let in_fz: i64 = row.try_get(7).unwrap_or(0);
        let in_ce: i64 = row.try_get(8).unwrap_or(0);
        let in_me: i64 = row.try_get(9).unwrap_or(0);
        let in_fe: i64 = row.try_get(10).unwrap_or(0);
        let groups = format_groups(in_cz, in_mz, in_fz, in_ce, in_me, in_fe);
        let source_codec: Option<String> = row.try_get(11).ok();
        let source_q: Option<f64> = row.try_get(12).ok();
        let source_w: i64 = row.try_get(13).unwrap_or(0);
        let source_h: i64 = row.try_get(14).unwrap_or(0);
        let rec_max: Option<i64> = row.try_get(15).ok();
        let corpus: String = row.try_get(16).unwrap_or_default();
        let relative_path: Option<String> = row.try_get(17).ok();
        let format: Option<String> = row.try_get(18).ok();
        let license_id: String = row
            .try_get(19)
            .unwrap_or_else(|_| "mixed-research".to_string());
        let license_url: Option<String> = row.try_get(20).ok();
        let target_max_dim: Option<i64> = row.try_get(21).ok();
        let q_imp: Option<f64> = row.try_get(22).ok();
        let m_dpr: Option<f64> = row.try_get(23).ok();
        let m_dist: Option<f64> = row.try_get(24).ok();
        let encoder_label: Option<String> = row.try_get(25).ok();

        let policy = licensing::by_id(&license_id);

        body.push_str(&format!(
            "{decision_id}\t{sha}\t{curator_id}\t{decided_at}\t{decision}\t\
             {groups}\t{codec}\t{sq}\t{sw}\t{sh}\t\
             {rm}\t{corpus}\t{rp}\t{fmt}\t\
             {lid}\t{ll}\t{lt}\t{lu}\t\
             {lr}\t{lct}\t\
             {tmd}\t{qi}\t{mdpr}\t{mdist}\t{enc}\n",
            codec = source_codec.as_deref().unwrap_or(""),
            sq = source_q.map(|x| format!("{:.2}", x)).unwrap_or_default(),
            sw = source_w,
            sh = source_h,
            rm = rec_max.map(|x| x.to_string()).unwrap_or_default(),
            rp = relative_path.as_deref().unwrap_or(""),
            fmt = format.as_deref().unwrap_or(""),
            lid = policy.id,
            ll = policy.label,
            lt = policy.terms_url,
            lu = license_url.as_deref().unwrap_or(""),
            lr = policy.redistribute_bytes as u8,
            lct = policy.commercial_training as u8,
            tmd = target_max_dim.map(|x| x.to_string()).unwrap_or_default(),
            qi = q_imp.map(|x| format!("{:.2}", x)).unwrap_or_default(),
            mdpr = m_dpr.map(|x| format!("{:.3}", x)).unwrap_or_default(),
            mdist = m_dist.map(|x| format!("{:.1}", x)).unwrap_or_default(),
            enc = encoder_label.as_deref().unwrap_or(""),
        ));
    }
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/tab-separated-values")],
        body,
    )
        .into_response())
}

fn format_groups(cz: i64, mz: i64, fz: i64, ce: i64, me: i64, fe: i64) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if cz != 0 {
        parts.push("core_zensim");
    }
    if mz != 0 {
        parts.push("medium_zensim");
    }
    if fz != 0 {
        parts.push("full_zensim");
    }
    if ce != 0 {
        parts.push("core_encoding");
    }
    if me != 0 {
        parts.push("medium_encoding");
    }
    if fe != 0 {
        parts.push("full_encoding");
    }
    if parts.is_empty() {
        "-".to_string()
    } else {
        parts.join(",")
    }
}

#[derive(Debug, Deserialize)]
pub struct LoadManifestReq {
    /// Either "tsv" or "jsonl". Determines parser.
    pub kind: String,
    /// Manifest body (full text).
    pub body: String,
    /// URL to attach as the blob URL when the manifest doesn't carry one.
    /// For `kind=jsonl` this is the R2 public-read base (e.g.
    /// "https://pub-….r2.dev"); for `kind=tsv` it is the prefix that
    /// `relative_path` is appended to.
    pub blob_url_base: String,
    /// Optional allow-list of `license_id` values; rows whose corpus resolves
    /// to a policy not in this set are skipped silently. Use this when bulk-
    /// loading from a mixed-provenance manifest (e.g. corpus-builder R2) and
    /// you only want the redistributable subset. Empty/None disables filtering.
    #[serde(default)]
    pub license_filter: Option<Vec<String>>,
    /// Convenience flag: when `true`, skips any row whose policy has
    /// `redistribute_bytes = false`. Equivalent to passing
    /// `license_filter = ["unsplash", "wikimedia-mixed", "flickr-mixed"]`
    /// (the three currently-marked-redistributable policies). Mutually
    /// inclusive with `license_filter`: a row passes if it matches *either*.
    #[serde(default)]
    pub redistributable_only: bool,
}

#[derive(Debug, Serialize)]
pub struct LoadManifestResp {
    pub inserted: u64,
    pub total: i64,
}

/// `POST /api/curator/manifest` — load a candidate manifest into the DB.
/// Idempotent. Tests call this with a small fixture; in production the operator
/// can POST a 30 MB JSONL manifest and Squintly streams it page-by-page.
pub async fn load_manifest(
    State(state): State<SharedState>,
    Json(req): Json<LoadManifestReq>,
) -> Result<Json<LoadManifestResp>, AppError> {
    let mut candidates = match req.kind.as_str() {
        "tsv" => parse_tsv_manifest(
            &req.body,
            |corpus, rel| format!("{}/{corpus}/{rel}", req.blob_url_base.trim_end_matches('/')),
            |corpus, rel| {
                use sha2::Digest;
                let mut h = sha2::Sha256::new();
                h.update(corpus.as_bytes());
                h.update(b"|");
                h.update(rel.as_bytes());
                hex::encode(h.finalize())
            },
        ),
        "jsonl" => parse_jsonl_manifest(&req.body, |sha| {
            r2_blob_url(req.blob_url_base.trim_end_matches('/'), sha)
        }),
        _ => return Err(AppError::BadRequest("kind must be 'tsv' or 'jsonl'".into())),
    };
    let parsed = candidates.len();
    candidates = filter_candidates(
        candidates,
        req.license_filter.as_deref(),
        req.redistributable_only,
    );
    let kept = candidates.len();
    if candidates.is_empty() {
        return Err(AppError::BadRequest(format!(
            "manifest parsed to {parsed} rows but the filter dropped them all; \
             relax license_filter or redistributable_only"
        )));
    }
    let inserted = upsert_candidates(&state.pool, &candidates)
        .await
        .map_err(AppError::from)?;
    let total: i64 = sqlx::query("SELECT COUNT(*) FROM curator_candidates")
        .fetch_one(&state.pool)
        .await
        .map(|r| r.get::<i64, _>(0))?;
    tracing::info!(parsed, kept, inserted, total, "load_manifest completed");
    Ok(Json(LoadManifestResp { inserted, total }))
}

/// Apply optional license filtering. When neither filter is set the input is
/// returned unchanged. When set, a row passes if its `license_id` matches the
/// allow-list **OR** (if `redistributable_only`) its policy permits byte
/// redistribution. Drops are silent — caller decides via the `parsed` vs
/// `kept` counts how aggressive the filter was.
pub fn filter_candidates(
    candidates: Vec<Candidate>,
    license_filter: Option<&[String]>,
    redistributable_only: bool,
) -> Vec<Candidate> {
    if license_filter.is_none() && !redistributable_only {
        return candidates;
    }
    let allow: std::collections::HashSet<&str> = license_filter
        .map(|v| v.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();
    candidates
        .into_iter()
        .filter(|c| {
            if allow.contains(c.license_id.as_str()) {
                return true;
            }
            if redistributable_only {
                return licensing::by_id(&c.license_id).redistribute_bytes;
            }
            false
        })
        .collect()
}

/// `POST /api/curator/load-r2-public` — admin-gated convenience that fetches
/// `manifest.jsonl` from a public-read R2 bucket and bulk-loads the
/// redistributable-only slice. Use this in production to seed the curator
/// queue from corpus-builder without round-tripping the 30 MB body through
/// the operator's machine.
#[derive(Debug, Deserialize)]
pub struct LoadR2Req {
    pub admin_token: String,
    /// Public-read base, e.g. `https://pub-….r2.dev`. Trailing slashes ok.
    pub r2_public_base: String,
    /// Optional manifest path (default `manifest.jsonl`).
    pub manifest_path: Option<String>,
    /// Cap on inserted rows. Defaults to 5000; safety against accidentally
    /// loading the whole 17k-row R2 manifest in one shot.
    pub limit: Option<usize>,
    /// Override the default allow-list. Defaults to
    /// `["unsplash", "wikimedia-mixed"]` — the two policies marked both
    /// `redistribute_bytes = true` AND `commercial_training = true`.
    /// Pass `["unsplash", "wikimedia-mixed", "flickr-mixed"]` to also
    /// include Flickr photos (which are CC-mixed, mostly non-commercial).
    pub license_filter: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct LoadR2Resp {
    pub fetched_lines: usize,
    pub parsed: usize,
    pub kept_after_filter: usize,
    pub inserted: u64,
    pub total: i64,
    pub r2_public_base: String,
}

pub async fn load_r2_public(
    State(state): State<SharedState>,
    Json(req): Json<LoadR2Req>,
) -> Result<Json<LoadR2Resp>, AppError> {
    require_curator_admin(&Some(req.admin_token.clone()))?;
    let base = req.r2_public_base.trim_end_matches('/').to_string();
    let manifest_path = req.manifest_path.as_deref().unwrap_or("manifest.jsonl");
    let url = format!("{base}/{manifest_path}");
    let limit = req.limit.unwrap_or(5000);
    let allow = req
        .license_filter
        .unwrap_or_else(|| vec!["unsplash".to_string(), "wikimedia-mixed".to_string()]);

    tracing::info!(url, limit, ?allow, "fetching R2 manifest for bulk load");
    let resp = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("fetch {url}: {e}")))?
        .error_for_status()
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("R2 manifest HTTP error: {e}")))?;
    let body = resp
        .text()
        .await
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("read R2 manifest body: {e}")))?;
    let total_lines = body.lines().filter(|l| !l.trim().is_empty()).count();
    let mut candidates = parse_jsonl_manifest(&body, |sha| r2_blob_url(&base, sha));
    let parsed = candidates.len();
    candidates = filter_candidates(candidates, Some(&allow), false);
    let kept = candidates.len();
    if candidates.len() > limit {
        candidates.truncate(limit);
    }
    let inserted = if candidates.is_empty() {
        0
    } else {
        upsert_candidates(&state.pool, &candidates)
            .await
            .map_err(AppError::from)?
    };
    let total: i64 = sqlx::query("SELECT COUNT(*) FROM curator_candidates")
        .fetch_one(&state.pool)
        .await
        .map(|r| r.get::<i64, _>(0))?;
    Ok(Json(LoadR2Resp {
        fetched_lines: total_lines,
        parsed,
        kept_after_filter: kept,
        inserted,
        total,
        r2_public_base: base,
    }))
}

/// `POST /api/curator/backfill-dims` — admin-gated. Walks `curator_candidates`
/// rows with NULL width or height, fetches a Range-bounded prefix of each
/// `blob_url`, parses the image header via `imagesize`, and updates the row.
/// Designed for one-shot recovery: the R2 JSONL doesn't always include
/// dimensions (zcimg enrichment was opt-in upstream), so wide-gamut/non-srgb
/// entries land with `width=null` and the bpp gate degrades to Unknown.
///
/// Concurrency is bounded (default 16 in-flight). `limit` caps the row count
/// per call so we can chunk a 1000-row backfill across several invocations.
#[derive(Debug, Deserialize)]
pub struct BackfillDimsReq {
    pub admin_token: String,
    pub limit: Option<i64>,
    /// Bytes to range-fetch per blob (default 262_144 = 256 KB; jpeg/png/webp/
    /// avif headers fit comfortably in the first few KB but some progressive
    /// JPEGs need more). Hard ceiling 4 MB.
    pub fetch_bytes: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct BackfillDimsResp {
    pub considered: usize,
    pub fetched_ok: usize,
    pub parsed_ok: usize,
    pub updated: usize,
    pub failures: usize,
    /// Of the rows where we got bytes, how many also produced a JPEG quality
    /// estimate via DQT-table parse. Non-JPEGs always count as 0 here.
    pub q_estimated: usize,
}

#[derive(Debug, Deserialize, Default)]
pub struct BackfillDimsReqMode {
    /// When true, also re-process rows that already have dims but are missing
    /// `source_q_detected` (useful after the 0009 migration on a DB that
    /// already has populated widths).
    #[serde(default)]
    pub include_dim_ok_missing_q: bool,
}

pub async fn backfill_dims(
    State(state): State<SharedState>,
    Json(req): Json<BackfillDimsReq>,
) -> Result<Json<BackfillDimsResp>, AppError> {
    require_curator_admin(&Some(req.admin_token.clone()))?;
    let limit = req.limit.unwrap_or(500).clamp(1, 5000);
    let fetch_bytes = req
        .fetch_bytes
        .unwrap_or(262_144)
        .clamp(1024, 4 * 1024 * 1024);
    // We process rows missing dims OR (after the 0009 migration) missing
    // source_q_detected on JPEG sources. The single SQL covers both because
    // backfill is idempotent — re-fetching a row that already has dims still
    // costs the same range request, but lets us populate q in one pass.
    let rows = sqlx::query(
        "SELECT sha256, blob_url, format FROM curator_candidates \
         WHERE width IS NULL OR height IS NULL OR width = 0 OR height = 0 \
            OR (format = 'jpeg' AND source_q_detected IS NULL) \
         ORDER BY order_hint LIMIT ?",
    )
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;
    let candidates: Vec<(String, String, Option<String>)> = rows
        .into_iter()
        .map(|r| {
            (
                r.get::<String, _>(0),
                r.get::<String, _>(1),
                r.try_get::<Option<String>, _>(2).ok().flatten(),
            )
        })
        .collect();
    let considered = candidates.len();
    if considered == 0 {
        return Ok(Json(BackfillDimsResp {
            considered: 0,
            fetched_ok: 0,
            parsed_ok: 0,
            updated: 0,
            failures: 0,
            q_estimated: 0,
        }));
    }
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::Anyhow(anyhow::anyhow!("http client: {e}")))?;

    struct Probe {
        sha: String,
        dims: Option<(i64, i64)>,
        q: Option<f32>,
        err: String,
    }

    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(16));
    let mut tasks = tokio::task::JoinSet::new();
    for (sha, url, fmt) in candidates {
        let sem = semaphore.clone();
        let http = http.clone();
        tasks.spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore");
            let resp = http
                .get(&url)
                .header("range", format!("bytes=0-{}", fetch_bytes - 1))
                .send()
                .await;
            let resp = match resp {
                Ok(r) if r.status().is_success() || r.status().as_u16() == 206 => r,
                Ok(r) => {
                    return Probe {
                        sha,
                        dims: None,
                        q: None,
                        err: format!("HTTP {}", r.status()),
                    };
                }
                Err(e) => {
                    return Probe {
                        sha,
                        dims: None,
                        q: None,
                        err: format!("{e}"),
                    };
                }
            };
            let bytes = match resp.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    return Probe {
                        sha,
                        dims: None,
                        q: None,
                        err: format!("read body: {e}"),
                    };
                }
            };
            let dims = imagesize::blob_size(&bytes)
                .ok()
                .map(|d| (d.width as i64, d.height as i64));
            // q estimate only meaningful for JPEGs — try anyway and discard
            // if format mismatches.
            let q = if matches!(fmt.as_deref(), Some("jpeg") | Some("jpg") | Some("JPEG"))
                || (fmt.is_none() && bytes.len() >= 4 && bytes[0] == 0xFF && bytes[1] == 0xD8)
            {
                crate::jpeg_q::estimate_quality(&bytes)
            } else {
                None
            };
            let err = if dims.is_none() {
                "could not parse dims".to_string()
            } else {
                String::new()
            };
            Probe { sha, dims, q, err }
        });
    }
    let mut fetched_ok = 0usize;
    let mut parsed_ok = 0usize;
    let mut updated = 0usize;
    let mut failures = 0usize;
    let mut q_estimated = 0usize;
    while let Some(joined) = tasks.join_next().await {
        let probe = match joined {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "backfill task panicked");
                failures += 1;
                continue;
            }
        };
        // We treat any row where we successfully read bytes as "fetched_ok",
        // even if dims didn't parse — since q may still come through.
        let any_signal = probe.dims.is_some() || probe.q.is_some();
        if !any_signal {
            failures += 1;
            tracing::debug!(sha = %probe.sha, err = %probe.err, "backfill no signal");
            continue;
        }
        fetched_ok += 1;
        if probe.dims.is_some() {
            parsed_ok += 1;
        }
        if probe.q.is_some() {
            q_estimated += 1;
        }
        // Build an UPDATE that only touches the columns we have a value for.
        // SQLite's COALESCE pattern keeps the existing column when the
        // bound parameter is NULL.
        let res = sqlx::query(
            "UPDATE curator_candidates SET \
                width = COALESCE(?, width), \
                height = COALESCE(?, height), \
                source_q_detected = COALESCE(?, source_q_detected) \
             WHERE sha256 = ?",
        )
        .bind(probe.dims.map(|d| d.0))
        .bind(probe.dims.map(|d| d.1))
        .bind(probe.q.map(|q| q as f64))
        .bind(&probe.sha)
        .execute(&state.pool)
        .await;
        match res {
            Ok(r) if r.rows_affected() > 0 => updated += 1,
            Ok(_) => {}
            Err(e) => {
                failures += 1;
                tracing::warn!(sha = %probe.sha, error = %e, "update failed");
            }
        }
    }
    Ok(Json(BackfillDimsResp {
        considered,
        fetched_ok,
        parsed_ok,
        updated,
        failures,
        q_estimated,
    }))
}

/// Curator-side admin gate. Reuses `SQUINTLY_SUGGESTION_ADMIN_TOKEN` so
/// operators only need to set one secret. When unset, returns 503.
fn require_curator_admin(provided: &Option<String>) -> Result<(), AppError> {
    let expected = std::env::var("SQUINTLY_SUGGESTION_ADMIN_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AppError::ServiceUnavailable(
                "Curator admin actions disabled: SQUINTLY_SUGGESTION_ADMIN_TOKEN not set.".into(),
            )
        })?;
    let provided = provided.as_deref().unwrap_or("");
    if !ct_eq_str(&expected, provided) {
        return Err(AppError::BadRequest("admin_token mismatch".into()));
    }
    Ok(())
}

/// Split a comma-separated query-string allow-list, trimming whitespace and
/// dropping empty entries. Empty input → empty Vec (callers treat that as
/// "no filter").
fn parse_csv_filter(s: Option<&str>) -> Vec<String> {
    s.map(|raw| {
        raw.split(',')
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

fn ct_eq_str(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.as_bytes().iter().zip(b.as_bytes().iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tsv_basic() {
        let body = "# header comment\n\
                    corpus\trelative_path\twidth\theight\tsize_bytes\n\
                    unsplash-webp\twebp/foo.webp\t2400\t1758\t819512\n\
                    source_jpegs\tjpg/bar.jpg\t1024\t768\t450000\n";
        let cands = parse_tsv_manifest(
            body,
            |c, r| format!("https://r2/{c}/{r}"),
            |c, r| format!("fakehash_{c}_{r}"),
        );
        assert_eq!(cands.len(), 2);
        assert_eq!(cands[0].corpus, "unsplash-webp");
        assert_eq!(cands[0].license_id, "unsplash");
        assert_eq!(cands[0].format.as_deref(), Some("webp"));
        assert_eq!(cands[0].width, Some(2400));
        assert_eq!(cands[1].corpus, "source_jpegs");
    }

    #[test]
    fn parse_jsonl_basic() {
        let body = r#"{"sha256":"abcd1234567890abcd1234567890abcd1234567890abcd1234567890abcdef","format":"webp","source":"internet","source_label":"scraping/webp","width":2400,"height":1800}
{"sha256":"deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef","format":"jpeg","source":"unsplash","source_label":"unsplash-webp","width":3000,"height":2000}"#;
        let cands = parse_jsonl_manifest(body, |sha| r2_blob_url("https://r2.dev", sha));
        assert_eq!(cands.len(), 2);
        assert_eq!(
            cands[0].blob_url,
            "https://r2.dev/blobs/ab/cd/abcd1234567890abcd1234567890abcd1234567890abcd1234567890abcdef"
        );
        assert_eq!(cands[1].license_id, "unsplash");
    }

    #[test]
    fn r2_blob_url_layout() {
        let url = r2_blob_url(
            "https://pub-x.r2.dev",
            "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd",
        );
        assert_eq!(
            url,
            "https://pub-x.r2.dev/blobs/ab/cd/abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd"
        );
    }

    #[test]
    fn suggest_high_q_jpeg_picks_core() {
        let c = Candidate {
            sha256: "h".into(),
            corpus: "source_jpegs".into(),
            relative_path: None,
            width: Some(2400),
            height: Some(1800),
            size_bytes: Some(1_000_000),
            format: Some("jpeg".into()),
            suspected_category: None,
            has_alpha: false,
            has_animation: false,
            license_id: "mixed-research".into(),
            license_url: None,
            blob_url: "https://r2/x".into(),
            order_hint: 0,
            source_q_detected: None,
        };
        let s = suggest(&c, Some(96.0));
        assert!(s.groups.contains(&"core_zensim"));
        assert!(s.sizes.contains(&1024));
    }

    #[test]
    fn suggest_low_q_jpeg_truncates_sizes() {
        let c = Candidate {
            sha256: "h".into(),
            corpus: "source_jpegs".into(),
            relative_path: None,
            width: Some(2400),
            height: Some(1800),
            size_bytes: Some(1_000_000),
            format: Some("jpeg".into()),
            suspected_category: None,
            has_alpha: false,
            has_animation: false,
            license_id: "mixed-research".into(),
            license_url: None,
            blob_url: "https://r2/x".into(),
            order_hint: 0,
            source_q_detected: None,
        };
        let s = suggest(&c, Some(50.0));
        // 2400 / 8 = 300 → 64, 128, 256 fit; 384+ get filtered out.
        assert!(s.sizes.iter().all(|d| *d <= 300));
    }

    #[test]
    fn suggest_unknown_dims_returns_all_chips() {
        let c = Candidate {
            sha256: "h".into(),
            corpus: "wide-gamut".into(),
            relative_path: None,
            width: None,
            height: None,
            size_bytes: Some(125_642),
            format: Some("jpeg".into()),
            suspected_category: None,
            has_alpha: false,
            has_animation: false,
            license_id: "wikimedia-mixed".into(),
            license_url: None,
            blob_url: "https://r2/x".into(),
            order_hint: 0,
            source_q_detected: None,
        };
        let s = suggest(&c, None);
        // dims unknown → curator should be able to pick any chip
        assert_eq!(s.sizes.len(), 8);
        assert!(s.sizes.contains(&64));
        assert!(s.sizes.contains(&1536));
    }

    #[test]
    fn bpp_gate_unknown_when_dims_missing() {
        let c = Candidate {
            sha256: "h".into(),
            corpus: "x".into(),
            relative_path: None,
            width: None,
            height: None,
            size_bytes: Some(100_000),
            format: Some("jpeg".into()),
            suspected_category: None,
            has_alpha: false,
            has_animation: false,
            license_id: "mixed-research".into(),
            license_url: None,
            blob_url: "https://r2/x".into(),
            order_hint: 0,
            source_q_detected: None,
        };
        let g = bpp_gate(&c);
        assert!(matches!(g.verdict, BppVerdict::Unknown));
        assert!(g.bpp.is_none());
    }

    #[test]
    fn bpp_gate_flags_low_jpeg() {
        // 2400×1800 = 4.32 MP. 100_000 bytes * 8 / 4.32e6 ≈ 0.185 bpp → low
        let c = Candidate {
            sha256: "h".into(),
            corpus: "x".into(),
            relative_path: None,
            width: Some(2400),
            height: Some(1800),
            size_bytes: Some(100_000),
            format: Some("jpeg".into()),
            suspected_category: None,
            has_alpha: false,
            has_animation: false,
            license_id: "mixed-research".into(),
            license_url: None,
            blob_url: "https://r2/x".into(),
            order_hint: 0,
            source_q_detected: None,
        };
        let g = bpp_gate(&c);
        assert!(matches!(g.verdict, BppVerdict::Low));
        let bpp = g.bpp.unwrap();
        assert!(bpp < 0.3, "got {bpp}");
    }

    #[test]
    fn bpp_gate_ok_for_typical_jpeg() {
        // 2400×1800 ≈ 4.32 MP, 800 KB → bpp ≈ 1.48 → OK
        let c = Candidate {
            sha256: "h".into(),
            corpus: "x".into(),
            relative_path: None,
            width: Some(2400),
            height: Some(1800),
            size_bytes: Some(800_000),
            format: Some("jpeg".into()),
            suspected_category: None,
            has_alpha: false,
            has_animation: false,
            license_id: "mixed-research".into(),
            license_url: None,
            blob_url: "https://r2/x".into(),
            order_hint: 0,
            source_q_detected: None,
        };
        let g = bpp_gate(&c);
        assert!(matches!(g.verdict, BppVerdict::Ok), "got {:?}", g.verdict);
    }

    #[test]
    fn bpp_gate_low_for_undercompressed_png() {
        // 100×100 = 10 KP, 1 KB → bpp = 0.8 → suspicious for lossless
        let c = Candidate {
            sha256: "h".into(),
            corpus: "x".into(),
            relative_path: None,
            width: Some(100),
            height: Some(100),
            size_bytes: Some(1000),
            format: Some("png".into()),
            suspected_category: None,
            has_alpha: false,
            has_animation: false,
            license_id: "mixed-research".into(),
            license_url: None,
            blob_url: "https://r2/x".into(),
            order_hint: 0,
            source_q_detected: None,
        };
        let g = bpp_gate(&c);
        assert!(matches!(g.verdict, BppVerdict::Low), "got {:?}", g.verdict);
    }
}
