//! HTTP surface for objective metric scores: ingest, list, and per-encoding
//! lookup.
//!
//! Ingest is admin-only. Reading is admin-only too, and that is a deliberate
//! choice rather than an oversight: showing an observer the ssim2 score of the
//! image they are about to judge would tell them the answer to the question
//! being asked. The identifier panel exists to troubleshoot corrupted encodes,
//! so it stays available to everybody — but the metric rows inside it are
//! filtered out unless the viewer is an operator. See `handlers::trial_metrics`.

use crate::curator::require_admin;
use crate::handlers::{AppError, SharedState};
use crate::metrics::{self, IngestSummary, MetricRow};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct IngestParams {
    /// Shared-token fallback for an operator who is not signed in, matching
    /// every other admin endpoint's shape.
    pub admin_token: Option<String>,
    /// What to record as the provenance of these values. A filename, a sweep
    /// id — whatever lets somebody later ask where a number came from.
    pub source: Option<String>,
    /// `tsv`, `csv` or `parquet`. Inferred from the content type when absent.
    pub format: Option<String>,
}

/// `POST /api/admin/metrics` — ingest a wide table of metric scores.
///
/// Takes the file as a raw body rather than multipart, so the operator's path
/// is one curl with `--data-binary @file` and no form encoding of a multi-GB
/// parquet. Format comes from `?format=`, or from the content type.
///
/// Idempotent by (encoding_id, metric): re-ingesting replaces. A corrected
/// score should overwrite a wrong one rather than accumulating a second row
/// that later has to be tie-broken by ingest time.
pub async fn ingest(
    State(state): State<SharedState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<IngestParams>,
    body: Bytes,
) -> Result<Json<IngestSummary>, AppError> {
    require_admin(&state.pool, &headers, &params.admin_token).await?;

    let declared = params.format.as_deref().map(str::to_ascii_lowercase);
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let format = declared.unwrap_or_else(|| {
        if content_type.contains("parquet") || content_type.contains("octet-stream") {
            "parquet".to_string()
        } else if content_type.contains("csv") {
            "csv".to_string()
        } else {
            "tsv".to_string()
        }
    });

    let rows: Vec<MetricRow> = match format.as_str() {
        "parquet" => metrics::parse_parquet(body)?,
        "csv" => metrics::parse_delimited(&decode_utf8(&body)?, b',')?,
        "tsv" => metrics::parse_delimited(&decode_utf8(&body)?, b'\t')?,
        other => {
            return Err(AppError::BadRequest(format!(
                "unknown format `{other}`: expected tsv, csv or parquet"
            )));
        }
    };
    if rows.is_empty() {
        return Err(AppError::BadRequest(
            "parsed successfully but produced no rows — check the id column and \
             that at least one column is a metric rather than metadata"
                .into(),
        ));
    }

    let now = crate::db::now_ms();
    let source = params.source.as_deref();

    // One transaction: a half-ingested metrics file is worse than none, because
    // the encodings that made it in look measured and the rest look unmeasured,
    // and nothing distinguishes that from a genuinely partial sweep.
    let mut tx = state.pool.begin().await?;
    for r in &rows {
        sqlx::query(
            "INSERT INTO encoding_metrics (encoding_id, metric, value, source, ingested_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(encoding_id, metric) DO UPDATE SET \
               value = excluded.value, source = excluded.source, \
               ingested_at = excluded.ingested_at",
        )
        .bind(&r.encoding_id)
        .bind(&r.metric)
        .bind(r.value)
        .bind(source)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    let mut summary = metrics::summarise(&rows);

    // How many of these encodings the study has actually served. Reported, not
    // enforced — a metrics file legitimately covers a whole corpus while the
    // study has served part of it — but an ingest where NOTHING matches is
    // almost always an id-namespace mismatch, and that is worth seeing straight
    // away rather than discovering when the report comes back empty.
    let matched: (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT m.encoding_id) FROM encoding_metrics m \
         WHERE EXISTS (SELECT 1 FROM trials t \
                       WHERE t.a_encoding_id = m.encoding_id \
                          OR t.b_encoding_id = m.encoding_id)",
    )
    .fetch_one(&state.pool)
    .await?;
    summary.unmatched_encodings = summary.encodings.saturating_sub(matched.0 as usize);

    // Refresh the sampler's cache in place. Without this an ingest changes
    // nothing until the next restart, and an operator would reasonably conclude
    // the filter does not work — the symptom being identical to no scores at
    // all, because that is exactly the state the sampler would still be in.
    match crate::handlers::load_metric_scores(&state.pool).await {
        Ok(scores) => *state.metric_scores.write().await = scores,
        Err(e) => {
            tracing::warn!(error = %e, "ingest stored, but the sampler cache was not refreshed")
        }
    }

    tracing::info!(
        rows = summary.rows,
        encodings = summary.encodings,
        unmatched = summary.unmatched_encodings,
        "ingested encoding metrics"
    );
    Ok(Json(summary))
}

#[derive(Debug, Serialize)]
pub struct MetricCatalogRow {
    pub metric: String,
    pub encodings: i64,
    pub direction: &'static str,
    pub blurb: Option<&'static str>,
    pub min: f64,
    pub max: f64,
    /// How many of this metric's encodings the study has actually served. The
    /// number that decides whether a correlation is possible, as distinct from
    /// how much was ingested.
    pub covered_encodings: i64,
}

#[derive(Debug, Deserialize)]
pub struct CatalogParams {
    pub admin_token: Option<String>,
}

/// `GET /api/admin/metrics` — what has been ingested, and whether it is usable.
pub async fn catalog(
    State(state): State<SharedState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<CatalogParams>,
) -> Result<Json<Vec<MetricCatalogRow>>, AppError> {
    require_admin(&state.pool, &headers, &params.admin_token).await?;
    let rows: Vec<(String, i64, f64, f64, i64)> = sqlx::query_as(
        "SELECT m.metric, COUNT(*), MIN(m.value), MAX(m.value), \
                COUNT(DISTINCT CASE WHEN EXISTS ( \
                  SELECT 1 FROM trials t WHERE t.a_encoding_id = m.encoding_id \
                                            OR t.b_encoding_id = m.encoding_id) \
                  THEN m.encoding_id END) \
         FROM encoding_metrics m GROUP BY m.metric ORDER BY m.metric",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(metric, encodings, min, max, covered)| MetricCatalogRow {
                direction: metrics::direction_of(&metric).as_str(),
                blurb: metrics::blurb_of(&metric),
                metric,
                encodings,
                min,
                max,
                covered_encodings: covered,
            })
            .collect(),
    ))
}

fn decode_utf8(b: &Bytes) -> Result<String, AppError> {
    String::from_utf8(b.to_vec())
        .map_err(|_| AppError::BadRequest("body is not valid UTF-8 text".into()))
}

#[derive(Debug, Deserialize)]
pub struct DispositionParams {
    pub admin_token: Option<String>,
    /// Which study to report on. Defaults to the deployment's default study,
    /// which is the one the headline question is about.
    pub study_id: Option<String>,
}

/// `GET /api/admin/disposition` — how close the study is to its answer.
///
/// Admin-only for the same reason the metric values are: it reports how well
/// the metric under test agrees with the observers, and an observer who reads
/// that has been told something about the answer to the question they are being
/// asked.
pub async fn disposition(
    State(state): State<SharedState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<DispositionParams>,
) -> Result<Json<crate::disposition::Disposition>, AppError> {
    require_admin(&state.pool, &headers, &params.admin_token).await?;
    let study = params
        .study_id
        .unwrap_or_else(|| crate::studies::default_study().id.to_string());
    let d = crate::disposition::compute(&state.pool, &study)
        .await
        .map_err(AppError::Anyhow)?;
    Ok(Json(d))
}

#[derive(Debug, Deserialize)]
pub struct ForEncodingsParams {
    pub admin_token: Option<String>,
    /// Comma-separated encoding ids.
    pub ids: String,
}

/// `GET /api/admin/metrics/encodings?ids=a,b` — scores for specific encodings.
///
/// Feeds the trial identifier panel, which exists so an operator can diagnose a
/// corrupted encode without leaving the trial. Admin-gated, and the browser
/// treats a 403 as "no metric rows to show" rather than an error — so an
/// ordinary observer simply sees the panel they always saw, with no gap where
/// something was withheld.
///
/// That gating is not incidental. Showing somebody the ssim2 score of the image
/// they are about to judge tells them the answer to the question being asked,
/// and a study that leaks its own hypothesis to its observers has measured
/// their reading comprehension.
pub async fn for_encodings(
    State(state): State<SharedState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<ForEncodingsParams>,
) -> Result<Json<Vec<EncodingMetricRow>>, AppError> {
    require_admin(&state.pool, &headers, &params.admin_token).await?;
    let ids: Vec<&str> = params
        .ids
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if ids.is_empty() {
        return Ok(Json(Vec::new()));
    }
    // Bound the fan-out: this is reached from a trial panel with two encodings,
    // and an unbounded IN list from a query string is an easy way to ask the
    // database for the whole table.
    let ids: Vec<&str> = ids.into_iter().take(8).collect();
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT encoding_id, metric, value FROM encoding_metrics \
         WHERE encoding_id IN ({placeholders}) ORDER BY encoding_id, metric"
    );
    let mut q = sqlx::query_as::<_, (String, String, f64)>(&sql);
    for id in &ids {
        q = q.bind(*id);
    }
    Ok(Json(
        q.fetch_all(&state.pool)
            .await?
            .into_iter()
            .map(|(encoding_id, metric, value)| EncodingMetricRow {
                encoding_id,
                metric,
                value,
            })
            .collect(),
    ))
}

#[derive(Debug, Serialize)]
pub struct EncodingMetricRow {
    pub encoding_id: String,
    pub metric: String,
    pub value: f64,
}
