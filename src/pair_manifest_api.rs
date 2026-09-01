//! HTTP surface for the pre-mined pair list: ingest and progress.
//!
//! Shaped exactly like `metrics_api`: admin-only, raw body rather than
//! multipart, so an operator's path is one `curl --data-binary @file`.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use serde::Deserialize;

use crate::curator::require_admin;
use crate::handlers::{AppError, SharedState};
use crate::pair_manifest::{self, IngestSummary, PairRow, Progress};

#[derive(Debug, Deserialize)]
pub struct IngestParams {
    pub admin_token: Option<String>,
    /// Which study's list this is. Must name a compiled study, and that study
    /// must actually use `PairingRule::FromManifest` — ingesting a list for a
    /// study that draws its own pairs would leave rows nothing ever serves.
    pub study_id: String,
    /// `tsv` (default) or `csv`.
    pub format: Option<String>,
}

/// `POST /api/admin/study-pairs` — replace one study's pair list.
pub async fn ingest(
    State(state): State<SharedState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<IngestParams>,
    body: Bytes,
) -> Result<Json<IngestSummary>, AppError> {
    require_admin(&state.pool, &headers, &params.admin_token).await?;

    let study = crate::studies::by_id(&params.study_id).ok_or_else(|| {
        AppError::BadRequest(format!(
            "unknown study {:?}: known studies are {:?}",
            params.study_id,
            crate::studies::STUDIES
                .iter()
                .map(|s| s.id)
                .collect::<Vec<_>>()
        ))
    })?;
    if study.sampler.pairing != crate::sampling::PairingRule::FromManifest {
        return Err(AppError::BadRequest(format!(
            "study {:?} draws its own pairs ({:?}); a pair list would never be served",
            study.id, study.sampler.pairing
        )));
    }

    let format = params
        .format
        .as_deref()
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "tsv".to_string());
    let text = std::str::from_utf8(&body)
        .map_err(|e| AppError::BadRequest(format!("body is not UTF-8: {e}")))?;
    let rows: Vec<PairRow> = match format.as_str() {
        "tsv" => pair_manifest::parse_delimited(text, b'\t')?,
        "csv" => pair_manifest::parse_delimited(text, b',')?,
        other => {
            return Err(AppError::BadRequest(format!(
                "unknown format {other:?}: expected tsv or csv"
            )));
        }
    };

    let n = pair_manifest::ingest(&state.pool, study.id, &rows).await?;

    let manifest = state.manifest.read().await;
    let unresolved = rows
        .iter()
        .filter(|r| {
            manifest.source(&r.source_hash).is_none()
                || manifest.encoding(&r.a_encoding_id).is_none()
                || manifest.encoding(&r.b_encoding_id).is_none()
        })
        .count();
    let mut per: std::collections::BTreeMap<&str, usize> = Default::default();
    for r in &rows {
        *per.entry(r.stratum.as_str()).or_default() += 1;
    }
    Ok(Json(IngestSummary {
        study_id: study.id.to_string(),
        rows: n,
        strata: per.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        repeats: rows.iter().filter(|r| r.repeat_of_pair.is_some()).count(),
        with_expected_choice: rows.iter().filter(|r| r.expected_choice.is_some()).count(),
        unresolved_in_manifest: unresolved,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ProgressParams {
    pub session_id: String,
}

/// `GET /api/study-pairs/progress?session_id=…` — how far through the plan
/// this session's observer is. Not admin-gated: it is the observer's own
/// progress, and it names no stimulus and no metric value.
pub async fn progress(
    State(state): State<SharedState>,
    Query(p): Query<ProgressParams>,
) -> Result<Json<Progress>, AppError> {
    let (study_id,): (String,) = sqlx::query_as("SELECT study_id FROM sessions WHERE id = ?")
        .bind(&p.session_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::BadRequest(format!("no session {:?}", p.session_id)))?;
    Ok(Json(
        pair_manifest::progress(&state.pool, &study_id, &p.session_id).await?,
    ))
}
