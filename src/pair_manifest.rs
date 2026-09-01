//! A pre-mined, pre-registered pair list, and the serving order over it.
//!
//! See `migrations/0027_study_pairs.sql` for why the plan is state in the
//! database rather than a rule in the sampler. This module owns three things
//! and nothing else:
//!
//! * parsing an ingest file into [`PairRow`]s (`parse_delimited`),
//! * writing them (`ingest`),
//! * answering "what is the next planned pair for this observer" (`next_pair`)
//!   and "how far through are they" (`progress`).
//!
//! It deliberately does NOT build a [`crate::sampling::TrialPlan`]. Turning a
//! row into a plan needs the manifest, and the one place that owns
//! manifest-shaped decisions is `sampling` — so the conversion lives there
//! (`sampling::plan_from_pair`) and this module stays a store.

use serde::{Deserialize, Serialize};

use crate::handlers::AppError;

/// One planned comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairRow {
    pub pair_id: String,
    pub seq: i64,
    pub source_hash: String,
    pub a_encoding_id: String,
    pub b_encoding_id: String,
    pub stratum: String,
    pub repeat_of_pair: Option<String>,
    pub expected_choice: Option<String>,
    /// Opaque to the server; round-trips to the export. See the migration.
    pub meta_json: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IngestSummary {
    pub study_id: String,
    pub rows: usize,
    pub strata: Vec<(String, usize)>,
    pub repeats: usize,
    pub with_expected_choice: usize,
    /// Rows naming a source or encoding the current manifest does not have.
    ///
    /// REPORTED, NOT REJECTED — and that asymmetry is deliberate. The manifest
    /// is refreshed independently of the ingest, so a pair list loaded before
    /// its corpus is staged would be rejected wholesale for a condition that
    /// fixes itself one `POST /api/manifest/refresh` later. But an operator who
    /// starts a session against a list that is 90 % unresolvable needs to know
    /// now, not after the observer hits a 409 on trial three.
    pub unresolved_in_manifest: usize,
}

/// Column aliases accepted on ingest, so a mining script does not have to
/// rename its own columns to talk to this.
fn norm(h: &str) -> String {
    h.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

fn column(headers: &[String], names: &[&str]) -> Option<usize> {
    headers.iter().position(|h| names.contains(&h.as_str()))
}

/// Parse a TSV/CSV pair list.
///
/// Required columns: `pair_id`, `source_hash`, `a_encoding_id`,
/// `b_encoding_id`, `stratum`. Optional: `seq` (defaults to file order),
/// `repeat_of_pair`, `expected_choice`, `meta_json`.
pub fn parse_delimited(text: &str, delimiter: u8) -> Result<Vec<PairRow>, AppError> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers: Vec<String> = rdr
        .headers()
        .map_err(|e| AppError::BadRequest(format!("unreadable header row: {e}")))?
        .iter()
        .map(norm)
        .collect();

    let need = |names: &[&str]| -> Result<usize, AppError> {
        column(&headers, names).ok_or_else(|| {
            AppError::BadRequest(format!(
                "missing required column: expected one of {names:?}, found {headers:?}"
            ))
        })
    };
    let i_id = need(&["pair_id", "id"])?;
    let i_src = need(&["source_hash", "source"])?;
    let i_a = need(&["a_encoding_id", "a", "a_id"])?;
    let i_b = need(&["b_encoding_id", "b", "b_id"])?;
    let i_stratum = need(&["stratum", "arm"])?;
    let i_seq = column(&headers, &["seq", "order", "index"]);
    let i_rep = column(&headers, &["repeat_of_pair", "repeat_of"]);
    let i_exp = column(&headers, &["expected_choice", "expected"]);
    let i_meta = column(&headers, &["meta_json", "meta"]);

    let get = |rec: &csv::StringRecord, at: Option<usize>| -> Option<String> {
        let v = rec.get(at?)?.trim();
        if v.is_empty() {
            None
        } else {
            Some(v.to_string())
        }
    };

    let mut out = Vec::new();
    for (n, rec) in rdr.records().enumerate() {
        let rec = rec.map_err(|e| AppError::BadRequest(format!("row {}: {e}", n + 2)))?;
        let field = |at: usize| -> Result<String, AppError> {
            rec.get(at)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .ok_or_else(|| AppError::BadRequest(format!("row {}: empty required field", n + 2)))
        };
        let a = field(i_a)?;
        let b = field(i_b)?;
        if a == b {
            // A pair of an encoding with itself is not a comparison. Rejecting
            // loudly at ingest beats serving a trial whose answer is undefined.
            return Err(AppError::BadRequest(format!(
                "row {}: a_encoding_id == b_encoding_id ({a})",
                n + 2
            )));
        }
        let expected = get(&rec, i_exp);
        if let Some(e) = expected.as_deref() {
            if e != "a" && e != "b" {
                return Err(AppError::BadRequest(format!(
                    "row {}: expected_choice must be 'a' or 'b', got {e:?}",
                    n + 2
                )));
            }
        }
        out.push(PairRow {
            pair_id: field(i_id)?,
            seq: get(&rec, i_seq)
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(n as i64),
            source_hash: field(i_src)?,
            a_encoding_id: a,
            b_encoding_id: b,
            stratum: field(i_stratum)?,
            repeat_of_pair: get(&rec, i_rep),
            expected_choice: expected,
            meta_json: get(&rec, i_meta).unwrap_or_else(|| "{}".to_string()),
        });
    }
    if out.is_empty() {
        return Err(AppError::BadRequest(
            "parsed successfully but produced no rows".into(),
        ));
    }

    // A repeat must point at a row in the same file, and must come after it —
    // a "repeat" served before its original measures nothing.
    let index: std::collections::HashMap<&str, i64> =
        out.iter().map(|r| (r.pair_id.as_str(), r.seq)).collect();
    for r in &out {
        if let Some(target) = r.repeat_of_pair.as_deref() {
            match index.get(target) {
                None => {
                    return Err(AppError::BadRequest(format!(
                        "pair {} repeats {target}, which is not in this file",
                        r.pair_id
                    )));
                }
                Some(&orig_seq) if orig_seq >= r.seq => {
                    return Err(AppError::BadRequest(format!(
                        "pair {} (seq {}) repeats {target} (seq {orig_seq}), which is not earlier",
                        r.pair_id, r.seq
                    )));
                }
                Some(_) => {}
            }
        }
    }
    Ok(out)
}

/// Replace this study's pair list with `rows`.
///
/// Idempotent by `pair_id` and scoped to one study: re-ingesting a corrected
/// list replaces it. Serving order is by `(seq, pair_id)`, so ingest order is
/// not load-bearing — but rows are inserted in `seq` order anyway, because a
/// repeat's foreign key needs its original to exist first.
pub async fn ingest(
    pool: &sqlx::SqlitePool,
    study_id: &str,
    rows: &[PairRow],
) -> Result<usize, AppError> {
    let now = chrono::Utc::now().timestamp();
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM study_pairs WHERE study_id = ?")
        .bind(study_id)
        .execute(&mut *tx)
        .await?;
    let mut ordered: Vec<&PairRow> = rows.iter().collect();
    ordered.sort_by(|x, y| (x.seq, &x.pair_id).cmp(&(y.seq, &y.pair_id)));
    for r in &ordered {
        sqlx::query(
            "INSERT INTO study_pairs (pair_id, study_id, seq, source_hash, a_encoding_id, \
             b_encoding_id, stratum, repeat_of_pair, expected_choice, meta_json, ingested_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&r.pair_id)
        .bind(study_id)
        .bind(r.seq)
        .bind(&r.source_hash)
        .bind(&r.a_encoding_id)
        .bind(&r.b_encoding_id)
        .bind(&r.stratum)
        .bind(r.repeat_of_pair.as_deref())
        .bind(r.expected_choice.as_deref())
        .bind(&r.meta_json)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(ordered.len())
}

/// The next planned pair for this observer: the lowest-`seq` row of the study
/// that this observer has not already been SERVED.
///
/// Keyed on the observer, not the session, so a 10-hour study spread over many
/// sessions resumes exactly where it stopped instead of restarting the list.
/// "Served", not "answered": a trial the observer skipped or abandoned is not
/// re-offered, because re-offering it would silently turn it into a repeat and
/// contaminate the consistency measurement with unplanned re-exposure.
/// Column tuple `next_pair`'s query maps to `PairRow`, in select order:
/// `(pair_id, seq, source_hash, a_encoding_id, b_encoding_id, stratum,
/// repeat_of_pair, expected_choice, meta_json)`.
type PairRowTuple = (
    String,
    i64,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
);

pub async fn next_pair(
    pool: &sqlx::SqlitePool,
    study_id: &str,
    session_id: &str,
) -> Result<Option<PairRow>, AppError> {
    let row: Option<PairRowTuple> = sqlx::query_as(
        "SELECT p.pair_id, p.seq, p.source_hash, p.a_encoding_id, p.b_encoding_id, \
                    p.stratum, p.repeat_of_pair, p.expected_choice, p.meta_json \
             FROM study_pairs p \
             WHERE p.study_id = ? \
               AND p.pair_id NOT IN ( \
                   SELECT t.study_pair_id FROM trials t \
                   JOIN sessions s ON s.id = t.session_id \
                   WHERE s.observer_id = (SELECT observer_id FROM sessions WHERE id = ?) \
                     AND t.study_pair_id IS NOT NULL) \
             ORDER BY p.seq, p.pair_id LIMIT 1",
    )
    .bind(study_id)
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(
        |(pair_id, seq, source_hash, a, b, stratum, repeat_of_pair, expected_choice, meta_json)| {
            PairRow {
                pair_id,
                seq,
                source_hash,
                a_encoding_id: a,
                b_encoding_id: b,
                stratum,
                repeat_of_pair,
                expected_choice,
                meta_json,
            }
        },
    ))
}

/// The trial id at which this observer was served `pair_id`, if they were.
///
/// Used to fill `trials.repeat_of_trial_id` on a planned repeat, so the
/// existing test-retest export and grading paths see it exactly as they see a
/// probabilistic repeat.
pub async fn trial_for_pair(
    pool: &sqlx::SqlitePool,
    session_id: &str,
    pair_id: &str,
) -> Result<Option<String>, AppError> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT t.id FROM trials t \
         JOIN sessions s ON s.id = t.session_id \
         WHERE s.observer_id = (SELECT observer_id FROM sessions WHERE id = ?) \
           AND t.study_pair_id = ? ORDER BY t.served_at LIMIT 1",
    )
    .bind(session_id)
    .bind(pair_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id,)| id))
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub study_id: String,
    pub planned: i64,
    pub served: i64,
    pub answered: i64,
    pub per_stratum: Vec<StratumProgress>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StratumProgress {
    pub stratum: String,
    pub planned: i64,
    pub answered: i64,
}

/// How far through the plan this observer is. The number an operator running a
/// timed study actually needs, and the one an observer sees on the screen.
pub async fn progress(
    pool: &sqlx::SqlitePool,
    study_id: &str,
    session_id: &str,
) -> Result<Progress, AppError> {
    let (planned,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM study_pairs WHERE study_id = ?")
        .bind(study_id)
        .fetch_one(pool)
        .await?;
    let (served, answered): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(CASE WHEN r.trial_id IS NULL THEN 0 ELSE 1 END), 0) \
         FROM trials t \
         JOIN sessions s ON s.id = t.session_id \
         LEFT JOIN responses r ON r.trial_id = t.id \
         WHERE s.observer_id = (SELECT observer_id FROM sessions WHERE id = ?) \
           AND s.study_id = ? AND t.study_pair_id IS NOT NULL",
    )
    .bind(session_id)
    .bind(study_id)
    .fetch_one(pool)
    .await?;
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT p.stratum, COUNT(*), \
                COALESCE(SUM(CASE WHEN r.trial_id IS NULL THEN 0 ELSE 1 END), 0) \
         FROM study_pairs p \
         LEFT JOIN trials t ON t.study_pair_id = p.pair_id \
         LEFT JOIN sessions s ON s.id = t.session_id \
             AND s.observer_id = (SELECT observer_id FROM sessions WHERE id = ?) \
         LEFT JOIN responses r ON r.trial_id = t.id \
         WHERE p.study_id = ? GROUP BY p.stratum ORDER BY p.stratum",
    )
    .bind(session_id)
    .bind(study_id)
    .fetch_all(pool)
    .await?;
    Ok(Progress {
        study_id: study_id.to_string(),
        planned,
        served,
        answered,
        per_stratum: rows
            .into_iter()
            .map(|(stratum, planned, answered)| StratumProgress {
                stratum,
                planned,
                answered,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "pair_id\tsource_hash\ta_encoding_id\tb_encoding_id\tstratum\tseq";

    #[test]
    fn parses_the_minimal_column_set() {
        let rows = parse_delimited(&format!("{HEAD}\np1\ts1\tea\teb\tdisagreement\t0\n"), b'\t')
            .expect("parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pair_id, "p1");
        assert_eq!(rows[0].stratum, "disagreement");
        assert_eq!(rows[0].meta_json, "{}");
        assert!(rows[0].expected_choice.is_none());
    }

    /// A pair of an encoding with itself has no answer. Better to fail the
    /// ingest than to serve a trial whose "correct" side is undefined.
    #[test]
    fn a_self_pair_is_rejected() {
        let e = parse_delimited(&format!("{HEAD}\np1\ts1\tea\tea\tdisagreement\t0\n"), b'\t')
            .expect_err("must reject");
        assert!(format!("{e:?}").contains("a_encoding_id == b_encoding_id"));
    }

    #[test]
    fn expected_choice_must_name_a_side() {
        let head = format!("{HEAD}\texpected_choice");
        assert!(
            parse_delimited(
                &format!("{head}\np1\ts1\tea\teb\tcalibration\t0\ta\n"),
                b'\t'
            )
            .is_ok()
        );
        let e = parse_delimited(
            &format!("{head}\np1\ts1\tea\teb\tcalibration\t0\tleft\n"),
            b'\t',
        )
        .expect_err("must reject");
        assert!(format!("{e:?}").contains("expected_choice"));
    }

    /// The consistency measurement is "did they answer the same pair the same
    /// way the second time". A repeat scheduled BEFORE its original is not a
    /// second look at anything, and a repeat of a row that is not in the file
    /// has no original at all. Both are plan errors, caught at ingest rather
    /// than discovered in the analysis.
    #[test]
    fn a_repeat_must_follow_an_original_that_exists() {
        let head = format!("{HEAD}\trepeat_of_pair");
        let ok = parse_delimited(
            &format!("{head}\np1\ts1\tea\teb\tdisagreement\t0\t\np2\ts1\tea\teb\trepeat\t5\tp1\n"),
            b'\t',
        )
        .expect("parse");
        assert_eq!(ok[1].repeat_of_pair.as_deref(), Some("p1"));

        let dangling =
            parse_delimited(&format!("{head}\np2\ts1\tea\teb\trepeat\t5\tnope\n"), b'\t')
                .expect_err("must reject");
        assert!(format!("{dangling:?}").contains("not in this file"));

        let backwards = parse_delimited(
            &format!("{head}\np1\ts1\tea\teb\tdisagreement\t9\t\np2\ts1\tea\teb\trepeat\t5\tp1\n"),
            b'\t',
        )
        .expect_err("must reject");
        assert!(format!("{backwards:?}").contains("not earlier"));
    }

    /// `seq` is optional; file order stands in for it. Without this an operator
    /// who hand-writes a small list gets every row at seq 0 and a serving order
    /// decided by pair_id spelling.
    #[test]
    fn absent_seq_falls_back_to_file_order() {
        let rows = parse_delimited(
            "pair_id\tsource_hash\ta_encoding_id\tb_encoding_id\tstratum\n\
             pz\ts1\tea\teb\tx\n\
             pa\ts1\tec\ted\tx\n",
            b'\t',
        )
        .expect("parse");
        assert_eq!(rows[0].pair_id, "pz");
        assert_eq!(rows[0].seq, 0);
        assert_eq!(rows[1].seq, 1);
    }
}
