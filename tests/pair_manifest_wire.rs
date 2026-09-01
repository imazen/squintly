//! Integration test for the pre-mined pair list in `handlers::next_trial`.
//!
//! The unit tests in `pair_manifest` cover parsing. What they cannot cover is
//! the property the study actually rests on: that the trials an observer is
//! SERVED are the pairs that were REGISTERED, in the registered order, once
//! each, across sessions — and that a planned repeat is linked to the trial it
//! repeats. Every one of those is a handler-level fact.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::routing::get;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use squintly::coefficient::{CoefficientSource, HttpCoefficient};
use squintly::handlers::{self, AppState};

const STUDY: &str = "zensim-adjudication";

async fn fake_coefficient() -> Result<SocketAddr> {
    let app = Router::new()
        .route("/api/manifest", get(manifest))
        .route("/api/sources/{hash}/image", get(source_image))
        .route("/api/encodings/{id}/image", get(encoding_image));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Ok(addr)
}

/// Two sources with four encodings each, spanning two codecs — enough for the
/// cross-codec pairs an adjudication list is mostly made of, and deliberately
/// NOT an adjacent-quality ladder, so a pair the sampler would never draw is
/// distinguishable from one it might.
async fn manifest() -> Json<serde_json::Value> {
    Json(json!({
        "sources": [
            {"hash": "src0000000000001", "width": 384, "height": 288, "size_bytes": 9000, "corpus": "imazen26-7000-lilith-plots", "filename": "o_1042.png.scale384x288.png"},
            {"hash": "src0000000000002", "width": 512, "height": 384, "size_bytes": 12000, "corpus": "imazen26-7000-lilith-plots", "filename": "o_1044.png.scale512x384.png"}
        ],
        "encodings": [
            {"id": "e1jpeg85", "source_hash": "src0000000000001", "codec_name": "zenjpeg", "quality": 85.0, "encoded_size": 21000},
            {"id": "e1jpeg95", "source_hash": "src0000000000001", "codec_name": "zenjpeg", "quality": 95.0, "encoded_size": 39000},
            {"id": "e1avif60", "source_hash": "src0000000000001", "codec_name": "zenavif", "quality": 60.0, "encoded_size": 15000},
            {"id": "e1webp75", "source_hash": "src0000000000001", "codec_name": "zenwebp", "quality": 75.0, "encoded_size": 18000},
            {"id": "e2jpeg85", "source_hash": "src0000000000002", "codec_name": "zenjpeg", "quality": 85.0, "encoded_size": 26000},
            {"id": "e2avif60", "source_hash": "src0000000000002", "codec_name": "zenavif", "quality": 60.0, "encoded_size": 17000}
        ]
    }))
}

async fn source_image(Path(_hash): Path<String>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut h = HeaderMap::new();
    h.insert("content-type", "image/png".parse().unwrap());
    (StatusCode::OK, h, b"\x89PNG\r\n\x1a\nfake".to_vec())
}

async fn encoding_image(Path(_id): Path<String>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut h = HeaderMap::new();
    h.insert("content-type", "image/jpeg".parse().unwrap());
    (StatusCode::OK, h, b"\xff\xd8\xff\xe0fake".to_vec())
}

/// The registered list. `p5` is a planned exact repeat of `p1`.
const PAIRS_TSV: &str = "\
pair_id\tseq\tsource_hash\ta_encoding_id\tb_encoding_id\tstratum\trepeat_of_pair\texpected_choice\tmeta_json
p1\t0\tsrc0000000000001\te1jpeg85\te1avif60\tdisagreement\t\t\t{\"zone\":\"nl\"}
p2\t1\tsrc0000000000001\te1jpeg95\te1webp75\tdisagreement\t\t\t{}
p3\t2\tsrc0000000000002\te2jpeg85\te2avif60\tladder\t\t\t{}
p4\t3\tsrc0000000000001\te1jpeg95\te1avif60\tcalibration\t\ta\t{}
p5\t4\tsrc0000000000001\te1jpeg85\te1avif60\trepeat\tp1\t\t{}
";

struct Harness {
    base: String,
    client: reqwest::Client,
    pool: sqlx::SqlitePool,
}

impl Harness {
    async fn new() -> Result<Self> {
        let coeff_addr = fake_coefficient().await?;
        let coeff = HttpCoefficient::new(&format!("http://{coeff_addr}"))?;
        let manifest = squintly::coefficient::Coefficient::refresh_manifest(&coeff).await?;
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;

        let rows = squintly::pair_manifest::parse_delimited(PAIRS_TSV, b'\t')
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        squintly::pair_manifest::ingest(&pool, STUDY, &rows)
            .await
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;

        let state = Arc::new(AppState {
            pool: pool.clone(),
            coefficient: CoefficientSource::Http(coeff),
            manifest: tokio::sync::RwLock::new(manifest),
            anchors: tokio::sync::RwLock::new(Default::default()),
            source_flags: tokio::sync::RwLock::new(Default::default()),
            suggestions: squintly::suggestion_store::SuggestionStore::LocalDisk(
                squintly::suggestion_store::LocalDiskStore::new(tempfile::tempdir()?.keep()),
            ),
            metric_scores: Default::default(),
        });
        let api = Router::new()
            .route("/trial/next", get(handlers::next_trial))
            .route(
                "/trial/{id}/response",
                axum::routing::post(handlers::record_response),
            )
            .route("/session", axum::routing::post(handlers::create_session));
        let app = Router::new().nest("/api", api).with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Ok(Self {
            base: format!("http://{addr}"),
            client: reqwest::Client::new(),
            pool,
        })
    }

    async fn session(&self, observer_id: Option<&str>) -> Result<(String, String)> {
        let s: serde_json::Value = self
            .client
            .post(format!("{}/api/session", self.base))
            .json(&json!({
                "observer_id": observer_id,
                "device_pixel_ratio": 3.0,
                "screen_width_css": 390,
                "screen_height_css": 844,
                "color_gamut": "p3",
                "viewing_distance_cm": 30,
                "ambient_light": "room",
                "supported_codecs": ["jpeg", "png", "webp", "avif"],
                "study_id": STUDY
            }))
            .send()
            .await?
            .json()
            .await?;
        Ok((
            s["session_id"].as_str().unwrap().to_string(),
            s["observer_id"].as_str().unwrap().to_string(),
        ))
    }

    /// Fetch the next trial and answer it, returning the raw payload.
    async fn serve_and_answer(&self, session: &str, choice: &str) -> Result<serde_json::Value> {
        let r = self
            .client
            .get(format!("{}/api/trial/next?session_id={session}", self.base))
            .send()
            .await?;
        assert_eq!(r.status(), StatusCode::OK, "{}", r.text().await?);
        let t: serde_json::Value = r.json().await?;
        let id = t["trial_id"].as_str().unwrap().to_string();
        let resp = self
            .client
            .post(format!("{}/api/trial/{id}/response", self.base))
            .json(&json!({
                "choice": choice,
                "dwell_ms": 4000,
                "reveal_count": 1,
                "reveal_ms_total": 500,
                "zoom_used": false,
                "viewport_w_css": 390,
                "viewport_h_css": 844,
                "orientation": "portrait",
                "image_displayed_w_css": 384.0,
                "image_displayed_h_css": 288.0,
                "intrinsic_to_device_ratio": 1.0
            }))
            .send()
            .await?;
        assert!(resp.status().is_success(), "{}", resp.text().await?);
        Ok(t)
    }

    async fn pair_id_of(&self, trial_id: &str) -> Result<Option<String>> {
        let r: Option<(Option<String>,)> =
            sqlx::query_as("SELECT study_pair_id FROM trials WHERE id = ?")
                .bind(trial_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(r.and_then(|(p,)| p))
    }
}

/// The registered list is served in the registered order, once each, and every
/// trial records which planned row it came from.
///
/// This is the whole reason the feature exists: a pre-registered stimulus set
/// that the serving path cannot quietly deviate from.
#[tokio::test]
async fn the_plan_is_served_in_order_once_each() -> Result<()> {
    let h = Harness::new().await?;
    let (session, _obs) = h.session(None).await?;

    let mut seen = Vec::new();
    for _ in 0..5 {
        let t = h.serve_and_answer(&session, "a").await?;
        let pid = h
            .pair_id_of(t["trial_id"].as_str().unwrap())
            .await?
            .expect("every planned trial records its study_pair_id");
        seen.push(pid);
    }
    assert_eq!(seen, vec!["p1", "p2", "p3", "p4", "p5"], "planned order");
    Ok(())
}

/// Counterbalancing still applies. The slots are randomised at the same choke
/// point as every other pair, so the ENCODINGS must match the plan as an
/// unordered set while the slot order is free to differ.
///
/// Without this a planned list would put the same encoding in slot A every
/// time — the exact side-bias `counterbalance_pair` exists to kill, reintroduced
/// through a new door.
#[tokio::test]
async fn planned_pairs_are_counterbalanced_but_never_substituted() -> Result<()> {
    let mut swapped = 0;
    let mut kept = 0;
    // p1's plan is (a=e1jpeg85, b=e1avif60). Fresh observers, one trial each,
    // so we sample the coin rather than the sequence.
    for _ in 0..40 {
        let h = Harness::new().await?;
        let (session, _) = h.session(None).await?;
        let t = h.serve_and_answer(&session, "a").await?;
        let a = t["a"]["encoding_id"].as_str().unwrap().to_string();
        let b = t["b"]["encoding_id"].as_str().unwrap().to_string();
        let mut got = [a.as_str(), b.as_str()];
        got.sort_unstable();
        assert_eq!(
            got,
            ["e1avif60", "e1jpeg85"],
            "served a pair that is not the planned one"
        );
        if a == "e1jpeg85" {
            kept += 1
        } else {
            swapped += 1
        }
    }
    assert!(
        swapped > 0 && kept > 0,
        "both slot layouts must occur (kept {kept}, swapped {swapped})"
    );
    Ok(())
}

/// A 10-hour study is many sessions. The plan is keyed to the OBSERVER, so a
/// new session continues where the last stopped instead of restarting the list
/// — which would spend the whole budget on the first stratum.
#[tokio::test]
async fn a_new_session_resumes_the_plan_rather_than_restarting_it() -> Result<()> {
    let h = Harness::new().await?;
    let (s1, observer) = h.session(None).await?;
    for _ in 0..2 {
        h.serve_and_answer(&s1, "a").await?;
    }
    let (s2, obs2) = h.session(Some(&observer)).await?;
    assert_eq!(obs2, observer, "same observer");
    let t = h.serve_and_answer(&s2, "b").await?;
    assert_eq!(
        h.pair_id_of(t["trial_id"].as_str().unwrap()).await?,
        Some("p3".to_string()),
        "the second session must continue at p3, not restart at p1"
    );
    Ok(())
}

/// A planned repeat links to the trial it repeats, through the SAME
/// `repeat_of_trial_id` column a probabilistic repeat uses — so the existing
/// export, grading and disposition paths need no second code path to find it.
#[tokio::test]
async fn a_planned_repeat_links_to_its_original_trial() -> Result<()> {
    let h = Harness::new().await?;
    let (session, _) = h.session(None).await?;
    let mut trial_of_pair = std::collections::HashMap::new();
    for _ in 0..5 {
        let t = h.serve_and_answer(&session, "a").await?;
        let tid = t["trial_id"].as_str().unwrap().to_string();
        trial_of_pair.insert(h.pair_id_of(&tid).await?.unwrap(), tid);
    }
    let repeat_trial = &trial_of_pair["p5"];
    let (link,): (Option<String>,) =
        sqlx::query_as("SELECT repeat_of_trial_id FROM trials WHERE id = ?")
            .bind(repeat_trial)
            .fetch_one(&h.pool)
            .await?;
    assert_eq!(
        link.as_deref(),
        Some(trial_of_pair["p1"].as_str()),
        "p5 must point at the trial that served p1"
    );
    Ok(())
}

/// A row with a known answer is an attention check, and marking it `is_golden`
/// is what routes it into the existing `grading.rs` golden path. The expected
/// side must travel WITH the counterbalancing swap, or the check fails everyone
/// half the time — the exact bug `counterbalancing_flips_the_expected_answer`
/// guards for sampler-drawn goldens.
#[tokio::test]
async fn a_calibration_row_is_golden_and_its_expected_answer_follows_the_slots() -> Result<()> {
    let mut checked = 0;
    for _ in 0..40 {
        let h = Harness::new().await?;
        let (session, _) = h.session(None).await?;
        for _ in 0..3 {
            h.serve_and_answer(&session, "a").await?;
        }
        let t = h.serve_and_answer(&session, "a").await?;
        let tid = t["trial_id"].as_str().unwrap();
        assert_eq!(h.pair_id_of(tid).await?, Some("p4".to_string()));
        let (is_golden, expected): (i64, Option<String>) =
            sqlx::query_as("SELECT is_golden, expected_choice FROM trials WHERE id = ?")
                .bind(tid)
                .fetch_one(&h.pool)
                .await?;
        assert_eq!(is_golden, 1, "a row with a known answer is a golden");
        // The plan says the answer is `e1jpeg95` (ingested as side a). Whatever
        // slot it landed in, `expected_choice` must name that slot.
        let expected = expected.expect("golden carries its expected answer");
        let slot = if t["a"]["encoding_id"] == "e1jpeg95" {
            "a"
        } else {
            "b"
        };
        assert_eq!(expected, slot, "expected answer did not follow the slots");
        checked += 1;
    }
    assert_eq!(checked, 40);
    Ok(())
}

/// Finishing the plan is a distinct outcome from "no trials available". An
/// operator who sees the generic message goes looking at the corpus; the real
/// answer is that the observer has answered every registered pair.
#[tokio::test]
async fn a_finished_plan_reports_completion_not_an_empty_corpus() -> Result<()> {
    let h = Harness::new().await?;
    let (session, _) = h.session(None).await?;
    for _ in 0..5 {
        h.serve_and_answer(&session, "a").await?;
    }
    let r = h
        .client
        .get(format!("{}/api/trial/next?session_id={session}", h.base))
        .send()
        .await?;
    assert_eq!(r.status(), StatusCode::CONFLICT);
    let body = r.text().await?;
    assert!(
        body.contains("plan complete") && body.contains("5 of 5"),
        "unhelpful completion message: {body}"
    );
    Ok(())
}

/// A planned row whose bytes are not staged must FAIL BY NAME, not be skipped.
///
/// Skipping to the next resolvable row is the dangerous behaviour: the study
/// would run to completion having served a different set than it registered,
/// and nothing in the output would say so.
#[tokio::test]
async fn an_unresolvable_planned_pair_names_itself_rather_than_being_skipped() -> Result<()> {
    let h = Harness::new().await?;
    let rows = squintly::pair_manifest::parse_delimited(
        "pair_id\tseq\tsource_hash\ta_encoding_id\tb_encoding_id\tstratum\n\
         pmissing\t0\tsrc0000000000001\te1jpeg85\tnot_staged_yet\tdisagreement\n\
         pok\t1\tsrc0000000000001\te1jpeg85\te1avif60\tdisagreement\n",
        b'\t',
    )
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    squintly::pair_manifest::ingest(&h.pool, STUDY, &rows)
        .await
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;

    let (session, _) = h.session(None).await?;
    let r = h
        .client
        .get(format!("{}/api/trial/next?session_id={session}", h.base))
        .send()
        .await?;
    assert_eq!(r.status(), StatusCode::CONFLICT);
    let body = r.text().await?;
    assert!(
        body.contains("pmissing") && body.contains("not_staged_yet"),
        "the failure must name the row and the missing id: {body}"
    );
    Ok(())
}

/// Progress is reported per stratum, which is what a timed study needs mid-run:
/// "am I on track for the disagreement arm" is not answerable from a single
/// total.
#[tokio::test]
async fn progress_is_reported_per_stratum() -> Result<()> {
    let h = Harness::new().await?;
    let (session, _) = h.session(None).await?;
    for _ in 0..3 {
        h.serve_and_answer(&session, "a").await?;
    }
    let p = squintly::pair_manifest::progress(&h.pool, STUDY, &session)
        .await
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    assert_eq!(p.planned, 5);
    assert_eq!(p.answered, 3);
    let by: std::collections::HashMap<&str, (i64, i64)> = p
        .per_stratum
        .iter()
        .map(|s| (s.stratum.as_str(), (s.planned, s.answered)))
        .collect();
    assert_eq!(by["disagreement"], (2, 2));
    assert_eq!(by["ladder"], (1, 1));
    assert_eq!(by["calibration"], (1, 0));
    assert_eq!(by["repeat"], (1, 0));
    Ok(())
}
