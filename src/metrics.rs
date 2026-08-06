//! Objective metric scores per encoding: what they mean, and how to read a file
//! of them.
//!
//! Squintly collects one side of a correlation — how people rank encodings. The
//! other side is a metric's ranking of the same encodings, and it is computed
//! elsewhere (zenmetrics) and ingested here.
//!
//! # Names are open, directions are closed
//!
//! Ingestion accepts ANY metric name. zenmetrics alone emits fourteen columns
//! across six families, several carrying an implementation version in the name
//! (`cvvdp_imazen_v0_0_1`, `ssim2_imazen_iir_v3`), and a new backend or a
//! retuned kernel mints a new one without asking us. A fixed enum would mean a
//! code change per kernel retune, so there isn't one.
//!
//! Directions are the opposite: [`direction_of`] must recognise a metric to say
//! whether higher is better, and an unrecognised name gets
//! [`Direction::Unknown`]. That asymmetry is deliberate and load-bearing. A
//! stored value with no direction is still useful — it can be exported, joined,
//! eyeballed. A rank correlation with no direction is a coin flip on the SIGN,
//! and a sign error looks exactly like the finding this study is trying to
//! make: "the metric disagrees with humans on non-photo content" is what you
//! would report if you had simply subtracted in the wrong order. So the
//! analysis refuses what the store accepts.
//!
//! Direction rule, from zenmetrics' own statement of it
//! (`scripts/sweep/train_cvvdp_picker.py`): *butteraugli and dssim are
//! lower-better; everything else is higher-better.*

use crate::handlers::AppError;
use std::collections::BTreeMap;

/// Which way a metric points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// A bigger number means a better-looking image (ssim2, cvvdp, zensim…).
    HigherIsBetter,
    /// A bigger number means more visible damage (butteraugli, dssim).
    LowerIsBetter,
    /// Not recognised. Storable, exportable — never correlatable.
    Unknown,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::HigherIsBetter => "higher_is_better",
            Direction::LowerIsBetter => "lower_is_better",
            Direction::Unknown => "unknown",
        }
    }
}

/// A metric family we know how to interpret.
pub struct Family {
    /// Matched against the start of a normalised metric name.
    pub prefix: &'static str,
    pub direction: Direction,
    /// For the operator view. Short enough to sit in a table cell.
    pub blurb: &'static str,
}

/// The families zenmetrics emits, matched by prefix so version suffixes and
/// CPU/GPU namespacing come along for free.
///
/// Order matters: `butteraugli_max` and `butteraugli_pnorm3` both fall under
/// `butteraugli`, and `ssim2` must not swallow `ssim` were one ever added, so
/// longer prefixes are listed first and the first match wins.
pub const FAMILIES: &[Family] = &[
    Family {
        prefix: "butteraugli",
        direction: Direction::LowerIsBetter,
        blurb: "libjxl perceptual distance; ~1.0 is roughly the just-noticeable point",
    },
    Family {
        prefix: "ssim2",
        direction: Direction::HigherIsBetter,
        blurb: "SSIMULACRA2; 90 is about visually lossless, 70 high, 30 low",
    },
    Family {
        prefix: "ssimulacra2",
        direction: Direction::HigherIsBetter,
        blurb: "SSIMULACRA2, spelled out",
    },
    Family {
        prefix: "dssim",
        direction: Direction::LowerIsBetter,
        blurb: "structural dissimilarity; 0 is identical",
    },
    Family {
        prefix: "iwssim",
        direction: Direction::HigherIsBetter,
        blurb: "information-weighted SSIM in [0,1]; 1 is identical",
    },
    Family {
        prefix: "cvvdp",
        direction: Direction::HigherIsBetter,
        blurb: "ColorVideoVDP in JOD 0–10; 10 is imperceptible",
    },
    Family {
        prefix: "zensim",
        direction: Direction::HigherIsBetter,
        blurb: "zensim 0–100; note Profile-A identity is 97.69, not 100",
    },
];

/// Strip the decorations that carry no meaning for direction.
///
/// zenmetrics' sweep writer prefixes score columns with `score_`
/// (`sweep/run.rs`), and the same metric arrives with and without it depending
/// on whether the file came from the sweep path or the CLI. Lowercased because
/// a header is written by whoever wrote the exporter.
fn normalise(name: &str) -> String {
    let n = name.trim().to_ascii_lowercase();
    n.strip_prefix("score_").unwrap_or(&n).to_string()
}

/// Which way this metric points, by prefix match on its family.
pub fn direction_of(name: &str) -> Direction {
    let n = normalise(name);
    FAMILIES
        .iter()
        .find(|f| n.starts_with(f.prefix))
        .map(|f| f.direction)
        .unwrap_or(Direction::Unknown)
}

/// The family blurb, for the operator view. `None` when unrecognised.
pub fn blurb_of(name: &str) -> Option<&'static str> {
    let n = normalise(name);
    FAMILIES
        .iter()
        .find(|f| n.starts_with(f.prefix))
        .map(|f| f.blurb)
}

/// One ingested measurement.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricRow {
    pub encoding_id: String,
    pub metric: String,
    pub value: f64,
}

/// Column names accepted as "which encoding is this row about".
///
/// Several spellings because the producers differ: squintly's own exports say
/// `encoding_id`, zenmetrics sweep output says `encoding`, and a hand-made
/// join file often says `id`. Accepting all three costs nothing and removes a
/// reformatting step from the operator's path — the failure mode it replaces is
/// a file that ingests zero rows and says only "no encoding_id column".
const ID_COLUMNS: &[&str] = &["encoding_id", "encoding", "id"];

/// Columns that are metadata, never a metric, however they are spelled.
///
/// Without this, ingesting a sweep TSV would create metrics called `quality`
/// and `bytes` — numeric columns that parse fine and mean nothing as a quality
/// score. `bytes` in particular would correlate beautifully with human
/// judgement and be entirely an artefact of bigger files looking better.
const NON_METRIC_COLUMNS: &[&str] = &[
    "encoding_id",
    "encoding",
    "id",
    "source_hash",
    "source",
    "source_filename",
    "filename",
    "path",
    "codec",
    "quality",
    "effort",
    "bytes",
    "width",
    "height",
    "corpus",
    "stratum",
    "split",
];

fn is_metric_column(name: &str) -> bool {
    let n = normalise(name);
    !NON_METRIC_COLUMNS.iter().any(|c| n == *c)
}

/// Parse a wide TSV (or CSV) into long metric rows.
///
/// Wide in, long out: one column per metric is how every producer writes these
/// files, and one row per measurement is how they are stored.
///
/// Empty cells are SKIPPED, not stored as zero. A metric that failed to compute
/// for one encoding — cvvdp needs a GPU, iwssim needs `min(W,H) >= 176` — leaves
/// a blank, and storing that as 0.0 would put a "worst possible score" on an
/// encoding nobody measured. Same distinction the leaderboard's DEFAULT-0
/// incident turned on.
pub fn parse_delimited(text: &str, delimiter: u8) -> Result<Vec<MetricRow>, AppError> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_reader(text.as_bytes());

    let headers: Vec<String> = rdr
        .headers()
        .map_err(|e| AppError::BadRequest(format!("unreadable header row: {e}")))?
        .iter()
        .map(|h| h.trim().to_string())
        .collect();

    let id_at = headers
        .iter()
        .position(|h| ID_COLUMNS.contains(&normalise(h).as_str()))
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "no encoding id column: expected one of {ID_COLUMNS:?}, found {headers:?}"
            ))
        })?;

    let metric_at: Vec<usize> = (0..headers.len())
        .filter(|i| *i != id_at && is_metric_column(&headers[*i]))
        .collect();
    if metric_at.is_empty() {
        return Err(AppError::BadRequest(format!(
            "no metric columns: every column in {headers:?} is metadata"
        )));
    }

    let mut out = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| AppError::BadRequest(format!("unreadable row: {e}")))?;
        let Some(id) = rec.get(id_at).map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        for &i in &metric_at {
            let Some(cell) = rec.get(i).map(str::trim) else {
                continue;
            };
            // Blank is "not measured". Anything non-numeric that is not blank is
            // a malformed file and should be reported rather than dropped —
            // silently skipping it would ingest a partial file and call it done.
            if cell.is_empty() || cell.eq_ignore_ascii_case("nan") || cell == "-" {
                continue;
            }
            let value: f64 = cell.parse().map_err(|_| {
                AppError::BadRequest(format!(
                    "column `{}` row `{id}`: `{cell}` is not a number",
                    headers[i]
                ))
            })?;
            if !value.is_finite() {
                continue;
            }
            out.push(MetricRow {
                encoding_id: id.to_string(),
                metric: normalise(&headers[i]),
                value,
            });
        }
    }
    Ok(out)
}

/// Summary of an ingest, for the operator to check against what they sent.
#[derive(Debug, Default, serde::Serialize)]
pub struct IngestSummary {
    pub rows: usize,
    pub encodings: usize,
    /// Per metric: how many values landed, and which way it points. An
    /// `unknown` direction here is the operator's cue that the report will
    /// refuse to correlate it.
    pub metrics: Vec<MetricSummary>,
    /// Encoding ids in the file that no trial has ever referenced. Reported,
    /// not rejected — a metrics file legitimately covers a whole corpus while
    /// the study has only served part of it.
    pub unmatched_encodings: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct MetricSummary {
    pub metric: String,
    pub count: usize,
    pub direction: &'static str,
    pub min: f64,
    pub max: f64,
}

/// Fold parsed rows into the summary the operator sees.
pub fn summarise(rows: &[MetricRow]) -> IngestSummary {
    let mut by_metric: BTreeMap<&str, (usize, f64, f64)> = BTreeMap::new();
    let mut ids = std::collections::BTreeSet::new();
    for r in rows {
        ids.insert(r.encoding_id.as_str());
        let e = by_metric
            .entry(r.metric.as_str())
            .or_insert((0, f64::INFINITY, f64::NEG_INFINITY));
        e.0 += 1;
        e.1 = e.1.min(r.value);
        e.2 = e.2.max(r.value);
    }
    IngestSummary {
        rows: rows.len(),
        encodings: ids.len(),
        metrics: by_metric
            .into_iter()
            .map(|(metric, (count, min, max))| MetricSummary {
                metric: metric.to_string(),
                count,
                direction: direction_of(metric).as_str(),
                min,
                max,
            })
            .collect(),
        unmatched_encodings: 0,
    }
}

/// Parse a Parquet file into long metric rows.
///
/// Same wide-in, long-out contract as [`parse_delimited`], and the same
/// column rules — the id column may be spelled any of [`ID_COLUMNS`], metadata
/// columns are skipped, and a null is "not measured" rather than zero.
///
/// Parquet is here because that is what zenmetrics writes for anything large
/// (`~/work/claudehints/topics/parquet-vs-tsv.md`: Parquet+zstd above ~50 MB,
/// where `csv.DictReader` is 36x slower), so requiring a TSV would mean asking
/// the operator to convert a multi-GB sweep output by hand before it could be
/// ingested. It reads through parquet's ROW api rather than the arrow one, so
/// the whole arrow stack stays out of the build.
///
/// Numeric columns arrive as whatever width the writer chose — a score written
/// as float32 and one written as float64 are the same measurement — so every
/// numeric field is widened to f64. An integer-typed metric column is accepted
/// for the same reason: refusing it would reject a perfectly good file over the
/// writer's choice of physical type.
pub fn parse_parquet(bytes: axum::body::Bytes) -> Result<Vec<MetricRow>, AppError> {
    use parquet::file::reader::{FileReader, SerializedFileReader};
    use parquet::record::Field;

    let reader = SerializedFileReader::new(bytes)
        .map_err(|e| AppError::BadRequest(format!("not a readable parquet file: {e}")))?;
    let iter = reader
        .get_row_iter(None)
        .map_err(|e| AppError::BadRequest(format!("unreadable parquet rows: {e}")))?;

    let mut out = Vec::new();
    let mut saw_id_column = false;
    for row in iter {
        let row = row.map_err(|e| AppError::BadRequest(format!("bad parquet row: {e}")))?;

        // The id is read per row rather than resolved once from the schema:
        // `get_column_iter` yields (name, field) pairs anyway, so a second pass
        // over the same row costs nothing and avoids duplicating the schema
        // walk that would have to agree with it.
        let mut id: Option<String> = None;
        for (name, field) in row.get_column_iter() {
            if ID_COLUMNS.contains(&normalise(name).as_str()) {
                saw_id_column = true;
                if let Field::Str(v) = field {
                    id = Some(v.clone());
                } else if !matches!(field, Field::Null) {
                    // An id written as an integer is still an id.
                    id = Some(field.to_string());
                }
                break;
            }
        }
        let Some(id) = id.filter(|s| !s.is_empty()) else {
            continue;
        };

        for (name, field) in row.get_column_iter() {
            if !is_metric_column(name) {
                continue;
            }
            let value = match field {
                Field::Double(v) => *v,
                Field::Float(v) => *v as f64,
                Field::Int(v) => *v as f64,
                Field::Long(v) => *v as f64,
                Field::Short(v) => *v as f64,
                Field::UInt(v) => *v as f64,
                Field::ULong(v) => *v as f64,
                // Null is "this metric was not computed for this encoding",
                // which is normal — cvvdp needs a GPU, iwssim needs
                // min(W,H) >= 176. Storing it as 0.0 would put a
                // worst-possible score on an encoding nobody measured.
                Field::Null => continue,
                // A string column that is not an id is metadata we have no
                // rule for. Skipped rather than refused: a sweep parquet
                // carries plenty of those and none of them are metrics.
                _ => continue,
            };
            if !value.is_finite() {
                continue;
            }
            out.push(MetricRow {
                encoding_id: id.clone(),
                metric: normalise(name),
                value,
            });
        }
    }

    if !saw_id_column {
        return Err(AppError::BadRequest(format!(
            "no encoding id column: expected one of {ID_COLUMNS:?}"
        )));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directions_follow_zenmetrics_own_rule() {
        // "butteraugli + dssim are lower_better; everything else is
        // higher_better" — zenmetrics scripts/sweep/train_cvvdp_picker.py.
        for m in [
            "butteraugli_max",
            "butteraugli_pnorm3",
            "butteraugli_max_gpu",
        ] {
            assert_eq!(direction_of(m), Direction::LowerIsBetter, "{m}");
        }
        for m in ["dssim", "dssim_gpu"] {
            assert_eq!(direction_of(m), Direction::LowerIsBetter, "{m}");
        }
        for m in ["ssim2", "ssim2_gpu", "iwssim_gpu", "zensim", "zensim_gpu"] {
            assert_eq!(direction_of(m), Direction::HigherIsBetter, "{m}");
        }
    }

    #[test]
    fn versioned_and_prefixed_names_resolve_to_their_family() {
        // These are the real shapes zenmetrics writes — an implementation
        // version baked into the column name so CPU and GPU sidecars can be
        // joined without colliding. A direction table keyed on exact names
        // would return Unknown for every one of them, and the report would
        // then refuse to correlate the study's own headline metric.
        for m in [
            "cvvdp_imazen_v0_0_1",
            "cvvdp_cpu_imazen_v0_0_1",
            "score_cvvdp_imazen_v0_0_1",
        ] {
            assert_eq!(direction_of(m), Direction::HigherIsBetter, "{m}");
        }
        assert_eq!(
            direction_of("iwssim_cpu_imazen_v0_1_2"),
            Direction::HigherIsBetter
        );
        assert_eq!(
            direction_of("ssim2_imazen_iir_v3"),
            Direction::HigherIsBetter
        );
        assert_eq!(
            direction_of("score_butteraugli_pnorm3_gpu"),
            Direction::LowerIsBetter
        );
    }

    #[test]
    fn an_unknown_metric_is_unknown_rather_than_guessed() {
        // Storable, exportable, never correlatable. Defaulting to
        // higher-is-better would invert the sign for the next lower-is-better
        // metric anybody adds, and an inverted sign reads exactly like the
        // finding this study exists to make.
        assert_eq!(direction_of("lpips"), Direction::Unknown);
        assert_eq!(direction_of("some_new_2027_metric"), Direction::Unknown);
    }

    #[test]
    fn wide_tsv_becomes_long_rows() {
        let tsv = "encoding_id\tssim2\tbutteraugli_pnorm3\nenc1\t88.5\t1.2\nenc2\t70.25\t2.5\n";
        let rows = parse_delimited(tsv, b'\t').expect("parse");
        assert_eq!(rows.len(), 4);
        assert!(rows.contains(&MetricRow {
            encoding_id: "enc1".into(),
            metric: "ssim2".into(),
            value: 88.5
        }));
        assert!(rows.contains(&MetricRow {
            encoding_id: "enc2".into(),
            metric: "butteraugli_pnorm3".into(),
            value: 2.5
        }));
    }

    #[test]
    fn a_blank_cell_is_not_measured_rather_than_zero() {
        // cvvdp needs a GPU and iwssim needs min(W,H) >= 176, so gaps are
        // normal. Storing a gap as 0.0 puts a worst-possible score on an
        // encoding nobody measured, and every aggregate downstream inherits it.
        let tsv = "encoding_id\tssim2\tcvvdp_imazen_v0_0_1\nenc1\t88.5\t\nenc2\t70.0\t9.1\n";
        let rows = parse_delimited(tsv, b'\t').expect("parse");
        assert_eq!(rows.len(), 3);
        assert!(
            !rows
                .iter()
                .any(|r| r.encoding_id == "enc1" && r.metric.starts_with("cvvdp"))
        );
    }

    #[test]
    fn metadata_columns_do_not_become_metrics() {
        // `bytes` would correlate beautifully with human judgement and be
        // entirely an artefact of bigger files looking better.
        let tsv = "encoding_id\tcodec\tquality\tbytes\tssim2\nenc1\tjpegli\t80\t12345\t88.5\n";
        let rows = parse_delimited(tsv, b'\t').expect("parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].metric, "ssim2");
    }

    #[test]
    fn the_id_column_may_be_spelled_three_ways() {
        for id in ID_COLUMNS {
            let tsv = format!("{id}\tssim2\nenc1\t88.5\n");
            let rows = parse_delimited(&tsv, b'\t').expect("parse");
            assert_eq!(rows.len(), 1, "spelled {id}");
            assert_eq!(rows[0].encoding_id, "enc1");
        }
    }

    #[test]
    fn a_file_with_no_id_column_is_refused_by_name() {
        let err = parse_delimited("ssim2\tdssim\n88.5\t0.1\n", b'\t').unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("encoding"), "unhelpful message: {msg}");
    }

    #[test]
    fn a_non_numeric_cell_fails_the_whole_ingest() {
        // Skipping it would ingest a partial file and report success, which is
        // the failure mode where a truncated metrics run looks like a complete
        // one and every correlation is computed on whatever survived.
        let tsv = "encoding_id\tssim2\nenc1\tnot-a-number\n";
        assert!(parse_delimited(tsv, b'\t').is_err());
    }

    #[test]
    fn summary_reports_direction_so_unknowns_are_visible_before_analysis() {
        let rows = parse_delimited(
            "encoding_id\tssim2\tmystery\nenc1\t88.5\t3.0\nenc2\t70.0\t4.0\n",
            b'\t',
        )
        .expect("parse");
        let s = summarise(&rows);
        assert_eq!(s.encodings, 2);
        let mystery = s.metrics.iter().find(|m| m.metric == "mystery").unwrap();
        assert_eq!(mystery.direction, "unknown");
        let ssim2 = s.metrics.iter().find(|m| m.metric == "ssim2").unwrap();
        assert_eq!(ssim2.direction, "higher_is_better");
        assert_eq!(ssim2.min, 70.0);
        assert_eq!(ssim2.max, 88.5);
    }
}
