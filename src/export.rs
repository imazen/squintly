//! TSV exports in zenanalyze/zentrain-compatible schemas.

use anyhow::Result;
use rand::Rng;
use rand::SeedableRng;
use sqlx::Row;
use sqlx::SqlitePool;

use crate::bt::{Comparison, Outcome, beta_to_quality, fit, with_monotonicity};
use crate::stats::{
    BOOTSTRAP_ITERATIONS, bootstrap, ci90, disagreement_dummy_count, session_bias_offsets,
};
use crate::unified::{PairObs, PairOutcome, RatingObs, fit_unified, m_to_quality};

fn size_bucket(w: i64, h: i64) -> &'static str {
    let m = w.max(h);
    if m <= 256 {
        "S"
    } else if m <= 768 {
        "M"
    } else if m <= 2048 {
        "L"
    } else {
        "XL"
    }
}

fn condition_bucket(
    dpr: f64,
    viewing_distance_cm: Option<i64>,
    ambient: Option<&str>,
    gamut: Option<&str>,
) -> String {
    let dpr_b = if dpr < 1.5 {
        1
    } else if dpr < 2.5 {
        2
    } else {
        3
    };
    let dist_b = match viewing_distance_cm.unwrap_or(-1) {
        d if d <= 0 => "any".to_string(),
        d if d <= 25 => "20".to_string(),
        d if d <= 40 => "30".to_string(),
        d if d <= 60 => "50".to_string(),
        d if d <= 100 => "70".to_string(),
        d if d <= 200 => "150".to_string(),
        _ => "250".to_string(),
    };
    let amb_b = ambient.unwrap_or("any");
    let g_b = gamut.unwrap_or("any");
    format!("dpr{dpr_b}_dist{dist_b}_{amb_b}_{g_b}")
}

pub async fn pareto_tsv(pool: &SqlitePool) -> Result<String> {
    // Per (source_hash, condition_bucket): collect the set of comparisons (pair trials),
    // fit BT-Davidson with monotonicity, bootstrap-CI 200 iterations, emit per-encoding rows.
    let mut out = String::new();
    out.push_str("image_id\tsize\tconfig_name\ttarget_zq\tbytes\tquality\tquality_lo\tquality_hi\tobservers\ttrials\tconditions_bucket\theld_out\n");

    // Pull all pair responses joined with sessions for conditions.
    let rows = sqlx::query(
        "SELECT t.source_hash, t.intrinsic_w, t.intrinsic_h, t.a_encoding_id, t.a_codec, \
                t.a_quality, t.a_bytes, t.b_encoding_id, t.b_codec, t.b_quality, t.b_bytes, \
                r.choice, s.observer_id, s.device_pixel_ratio, s.viewing_distance_cm, \
                s.ambient_light, s.color_gamut, t.held_out \
         FROM trials t \
         JOIN responses r ON r.trial_id = t.id \
         JOIN sessions  s ON s.id = t.session_id \
         WHERE t.kind = 'pair'",
    )
    .fetch_all(pool)
    .await?;

    use std::collections::BTreeMap;
    type Key = (String, String); // (source_hash, condition_bucket)
    struct Bucket {
        comparisons: Vec<Comparison>,
        encoding_ids: Vec<String>,
        observers: std::collections::HashSet<String>,
        meta: BTreeMap<String, (String, Option<f32>, i64)>, // id → (codec, quality, bytes)
        size: &'static str,
        image_id: String,
        held_out: bool,
    }

    let mut buckets: BTreeMap<Key, Bucket> = BTreeMap::new();

    for row in rows {
        let source_hash: String = row.get(0);
        let w: i64 = row.get(1);
        let h: i64 = row.get(2);
        let a_id: String = row.get(3);
        let a_codec: String = row.get(4);
        let a_quality: Option<f32> = row.get(5);
        let a_bytes: i64 = row.get(6);
        let b_id: Option<String> = row.get(7);
        let b_codec: Option<String> = row.get(8);
        let b_quality: Option<f32> = row.get(9);
        let b_bytes: Option<i64> = row.get(10);
        let choice: String = row.get(11);
        let observer_id: String = row.get(12);
        let dpr: f64 = row.get(13);
        let dist: Option<i64> = row.get(14);
        let amb: Option<String> = row.get(15);
        let gamut: Option<String> = row.get(16);
        let held_out: i64 = row.get(17);

        let Some(b_id) = b_id else { continue };
        let Some(b_codec) = b_codec else { continue };
        let Some(b_bytes) = b_bytes else { continue };

        let bucket_str = condition_bucket(dpr, dist, amb.as_deref(), gamut.as_deref());
        let key = (source_hash.clone(), bucket_str.clone());
        let entry = buckets.entry(key).or_insert_with(|| Bucket {
            comparisons: Vec::new(),
            encoding_ids: Vec::new(),
            observers: Default::default(),
            meta: BTreeMap::new(),
            size: size_bucket(w, h),
            image_id: source_hash[..8.min(source_hash.len())].to_string(),
            held_out: held_out != 0,
        });

        // Index encoding into stable ID order
        for (id, codec, q, bytes) in [
            (&a_id, &a_codec, a_quality, a_bytes),
            (&b_id, &b_codec, b_quality, b_bytes),
        ] {
            entry
                .meta
                .entry(id.clone())
                .or_insert_with(|| (codec.clone(), q, bytes));
        }
        let idx = |id: &str, encs: &mut Vec<String>| -> usize {
            if let Some(p) = encs.iter().position(|s| s == id) {
                p
            } else {
                encs.push(id.to_string());
                encs.len() - 1
            }
        };
        let ia = idx(&a_id, &mut entry.encoding_ids);
        let ib = idx(&b_id, &mut entry.encoding_ids);
        let outcome = match choice.as_str() {
            "a" => Outcome::AWins,
            "b" => Outcome::BWins,
            "tie" => Outcome::Tie,
            _ => continue,
        };
        entry.comparisons.push(Comparison {
            a: ia,
            b: ib,
            outcome,
        });
        entry.observers.insert(observer_id);
    }

    for ((_source_hash, bucket_str), b) in buckets {
        if b.comparisons.is_empty() || b.encoding_ids.len() < 2 {
            continue;
        }
        // Inject CID22-style monotonicity dummies before the fit. For every
        // pair of same-codec encodings within this bucket, the higher quality
        // setting must score >= the lower one. Without this constraint, CID22
        // reported KRCC dropping from 0.99 to 0.56 — see docs/methodology.md.
        let monotone_pairs: Vec<(usize, usize)> = {
            let mut pairs = Vec::new();
            for i in 0..b.encoding_ids.len() {
                let (codec_i, q_i, _) = b.meta.get(&b.encoding_ids[i]).cloned().unwrap_or_default();
                let q_i = q_i.unwrap_or(0.0);
                for j in 0..b.encoding_ids.len() {
                    if i == j {
                        continue;
                    }
                    let (codec_j, q_j, _) =
                        b.meta.get(&b.encoding_ids[j]).cloned().unwrap_or_default();
                    let q_j = q_j.unwrap_or(0.0);
                    if codec_i == codec_j && q_j > q_i {
                        // (lo, hi) — j is hi, i is lo.
                        pairs.push((i, j));
                    }
                }
            }
            pairs
        };
        let comparisons = with_monotonicity(&b.comparisons, &monotone_pairs, 200);
        let f = fit(b.encoding_ids.len(), &comparisons, 0, 1.5);
        let quality = beta_to_quality(&f.beta, 0);
        let n_trials = b.comparisons.len();

        // Bootstrap 200 iterations: resample raw comparisons with replacement,
        // re-inject monotonicity, refit, collect quality estimates per item.
        // Per docs/methodology.md §7 (CID22 verbatim).
        let mut quality_samples: Vec<Vec<f32>> =
            vec![Vec::with_capacity(BOOTSTRAP_ITERATIONS); b.encoding_ids.len()];
        let seed = (bucket_str.len() as u64) ^ (b.image_id.len() as u64);
        bootstrap(&b.comparisons, BOOTSTRAP_ITERATIONS, seed, |resampled| {
            let with_mono = with_monotonicity(resampled, &monotone_pairs, 200);
            let bf = fit(b.encoding_ids.len(), &with_mono, 0, 1.5);
            let bq = beta_to_quality(&bf.beta, 0);
            for (i, q) in bq.iter().enumerate() {
                quality_samples[i].push(*q);
            }
        });

        for (i, eid) in b.encoding_ids.iter().enumerate() {
            let (codec, q, bytes) = b.meta.get(eid).cloned().unwrap_or_default();
            let q_target = q.unwrap_or(0.0);
            let config_name = format!("{}.q{:.0}", codec, q_target);
            let (q_lo, q_hi) = ci90(&quality_samples[i]).unwrap_or((quality[i], quality[i]));
            out.push_str(&format!(
                "{}\t{}\t{}\t{:.0}\t{}\t{:.2}\t{:.2}\t{:.2}\t{}\t{}\t{}\t{}\n",
                b.image_id,
                b.size,
                config_name,
                q_target,
                bytes,
                quality[i],
                q_lo,
                q_hi,
                b.observers.len(),
                n_trials,
                bucket_str,
                if b.held_out { 1 } else { 0 },
            ));
        }
    }

    Ok(out)
}

pub async fn thresholds_tsv(pool: &SqlitePool) -> Result<String> {
    // Per (source_hash, codec, condition_bucket): aggregate single-stimulus ratings,
    // apply per-session additive bias correction, estimate q_notice / q_dislike /
    // q_hate by logistic interpolation, bootstrap-CI 200 iterations.
    let mut out = String::new();
    out.push_str(
        "image_id\tsize\tcodec\tconditions_bucket\tq_notice\tq_dislike\tq_hate\
        \tq_notice_lo\tq_notice_hi\tq_dislike_lo\tq_dislike_hi\tq_hate_lo\tq_hate_hi\
        \tobservers\ttrials\theld_out\n",
    );

    let rows = sqlx::query(
        "SELECT t.source_hash, t.intrinsic_w, t.intrinsic_h, t.a_codec, t.a_quality, \
                r.choice, s.observer_id, s.device_pixel_ratio, s.viewing_distance_cm, \
                s.ambient_light, s.color_gamut, s.id, t.held_out \
         FROM trials t \
         JOIN responses r ON r.trial_id = t.id \
         JOIN sessions  s ON s.id = t.session_id \
         WHERE t.kind = 'single'",
    )
    .fetch_all(pool)
    .await?;

    // First pass: build the bias-correction sample list. Each row is
    // (session_id, stimulus_key=source||codec||q, score=rating).
    let mut bias_samples: Vec<(String, String, f32)> = Vec::new();
    for row in &rows {
        let source_hash: String = row.get(0);
        let codec: String = row.get(3);
        let q: Option<f32> = row.get::<Option<f64>, _>(4).map(|v| v as f32);
        let choice: String = row.get(5);
        let session_id: String = row.get(11);
        if let (Some(q), Ok(rating)) = (q, choice.parse::<f32>()) {
            let stim_key = format!("{}|{}|{:.0}", source_hash, codec, q);
            bias_samples.push((session_id, stim_key, rating));
        }
    }
    let bias_offsets = session_bias_offsets(&bias_samples);

    use std::collections::BTreeMap;
    #[derive(Default)]
    struct Bucket {
        /// Raw per-trial observations: (q_int, rating_i32, session_id) — kept raw so we can bootstrap.
        observations: Vec<(i32, f32, String)>,
        observers: std::collections::HashSet<String>,
        size: &'static str,
        image_id: String,
        held_out: bool,
    }
    type Key = (String, String, String); // (source_hash, codec, bucket)
    let mut buckets: BTreeMap<Key, Bucket> = BTreeMap::new();

    for row in rows {
        let source_hash: String = row.get(0);
        let w: i64 = row.get(1);
        let h: i64 = row.get(2);
        let codec: String = row.get(3);
        let q: Option<f64> = row.get(4);
        let choice: String = row.get(5);
        let observer_id: String = row.get(6);
        let dpr: f64 = row.get(7);
        let dist: Option<i64> = row.get(8);
        let amb: Option<String> = row.get(9);
        let gamut: Option<String> = row.get(10);
        let session_id: String = row.get(11);
        let held_out: i64 = row.get(12);

        let Some(q) = q else { continue };
        let q_int = q.round() as i32;
        let Ok(rating) = choice.parse::<f32>() else {
            continue;
        };
        // Apply per-session additive bias correction.
        let offset = bias_offsets.get(&session_id).copied().unwrap_or(0.0);
        let corrected = (rating + offset).clamp(1.0, 4.0);
        let bucket_str = condition_bucket(dpr, dist, amb.as_deref(), gamut.as_deref());
        let key = (source_hash.clone(), codec.clone(), bucket_str);
        let entry = buckets.entry(key).or_insert_with(|| Bucket {
            observations: Vec::new(),
            observers: Default::default(),
            size: size_bucket(w, h),
            image_id: source_hash[..8.min(source_hash.len())].to_string(),
            held_out: held_out != 0,
        });
        entry.observations.push((q_int, corrected, session_id));
        entry.observers.insert(observer_id);
    }

    for ((_, codec, bucket_str), b) in buckets {
        if b.observations.is_empty() {
            continue;
        }
        // Build the per-q histogram from corrected ratings.
        let counts = build_count_map(&b.observations);
        let q_notice = interp_threshold(&counts, |c| (c.1, c.0), 0.5);
        let q_dislike = interp_threshold(&counts, |c| (c.2, c.0), 0.5);
        let q_hate = interp_threshold(&counts, |c| (c.3, c.0), 0.5);

        // Bootstrap 200 iterations on the raw observations.
        let mut samples_n: Vec<f32> = Vec::with_capacity(BOOTSTRAP_ITERATIONS);
        let mut samples_d: Vec<f32> = Vec::with_capacity(BOOTSTRAP_ITERATIONS);
        let mut samples_h: Vec<f32> = Vec::with_capacity(BOOTSTRAP_ITERATIONS);
        let seed = bucket_str.len() as u64 ^ codec.len() as u64;
        bootstrap(&b.observations, BOOTSTRAP_ITERATIONS, seed, |resampled| {
            let bc = build_count_map(resampled);
            if let Some(qn) = interp_threshold(&bc, |c| (c.1, c.0), 0.5) {
                samples_n.push(qn);
            }
            if let Some(qd) = interp_threshold(&bc, |c| (c.2, c.0), 0.5) {
                samples_d.push(qd);
            }
            if let Some(qh) = interp_threshold(&bc, |c| (c.3, c.0), 0.5) {
                samples_h.push(qh);
            }
        });
        let (n_lo, n_hi) = ci90(&samples_n).unwrap_or((f32::NAN, f32::NAN));
        let (d_lo, d_hi) = ci90(&samples_d).unwrap_or((f32::NAN, f32::NAN));
        let (h_lo, h_hi) = ci90(&samples_h).unwrap_or((f32::NAN, f32::NAN));

        let total_trials = b.observations.len();
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            b.image_id,
            b.size,
            codec,
            bucket_str,
            fmt_opt(q_notice),
            fmt_opt(q_dislike),
            fmt_opt(q_hate),
            fmt_f32(n_lo),
            fmt_f32(n_hi),
            fmt_f32(d_lo),
            fmt_f32(d_hi),
            fmt_f32(h_lo),
            fmt_f32(h_hi),
            b.observers.len(),
            total_trials,
            if b.held_out { 1 } else { 0 },
        ));
    }

    Ok(out)
}

fn build_count_map(
    observations: &[(i32, f32, String)],
) -> std::collections::BTreeMap<i32, (i32, i32, i32, i32)> {
    let mut counts: std::collections::BTreeMap<i32, (i32, i32, i32, i32)> = Default::default();
    for (q, rating, _) in observations {
        let entry = counts.entry(*q).or_insert((0, 0, 0, 0));
        entry.0 += 1;
        if *rating >= 2.0 {
            entry.1 += 1;
        }
        if *rating >= 3.0 {
            entry.2 += 1;
        }
        if *rating >= 4.0 {
            entry.3 += 1;
        }
    }
    counts
}

fn fmt_f32(v: f32) -> String {
    if v.is_nan() {
        "NA".into()
    } else {
        format!("{:.1}", v)
    }
}

fn fmt_opt(v: Option<f32>) -> String {
    match v {
        Some(v) => format!("{:.1}", v),
        None => "NA".into(),
    }
}

/// Estimate the quality at which P(meets) crosses `target` by linear interpolation
/// between adjacent quality levels with sufficient samples.
fn interp_threshold(
    per_q: &std::collections::BTreeMap<i32, (i32, i32, i32, i32)>,
    project: impl Fn(&(i32, i32, i32, i32)) -> (i32, i32),
    target: f32,
) -> Option<f32> {
    let mut points: Vec<(f32, f32)> = per_q
        .iter()
        .filter_map(|(q, c)| {
            let (hits, n) = project(c);
            if n >= 2 {
                Some((*q as f32, hits as f32 / n as f32))
            } else {
                None
            }
        })
        .collect();
    points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    if points.len() < 2 {
        return None;
    }
    // P(meet) is monotonically decreasing in q (higher q → less distortion → fewer
    // observers notice/dislike/hate). Find the bracket where P crosses target.
    for w in points.windows(2) {
        let (qa, pa) = w[0];
        let (qb, pb) = w[1];
        if (pa - target) * (pb - target) <= 0.0 && (pb - pa).abs() > 1e-6 {
            let t = (target - pa) / (pb - pa);
            return Some(qa + t * (qb - qa));
        }
    }
    None
}

/// Pérez-Ortiz et al. 2019 unified rating + pairwise scale.
///
/// Per (source_hash, condition_bucket): joint-fit pair and rating data using
/// `unified::fit_unified()`. Output is one row per encoding with the latent
/// quality m mapped to a 0..100 scale, plus per-observer bias and noise
/// summary statistics.
pub async fn unified_tsv(pool: &SqlitePool) -> Result<String> {
    let mut out = String::new();
    out.push_str(
        "image_id\tsize\tconfig_name\ttarget_zq\tbytes\tquality_unified\tquality_unified_lo\tquality_unified_hi\
         \tobservers\tn_pairs\tn_ratings\tconditions_bucket\theld_out\n",
    );

    // Pull both trial kinds for sources together. The unified fit needs both
    // per (source, bucket) so we fold them into the same buckets.
    let pair_rows = sqlx::query(
        "SELECT t.source_hash, t.intrinsic_w, t.intrinsic_h, t.a_encoding_id, t.b_encoding_id, \
                t.a_codec, t.a_quality, t.a_bytes, t.b_codec, t.b_quality, t.b_bytes, \
                r.choice, s.observer_id, s.device_pixel_ratio, s.viewing_distance_cm, \
                s.ambient_light, s.color_gamut, t.held_out \
         FROM trials t JOIN responses r ON r.trial_id = t.id \
         JOIN sessions s ON s.id = t.session_id WHERE t.kind = 'pair'",
    )
    .fetch_all(pool)
    .await?;
    let single_rows = sqlx::query(
        "SELECT t.source_hash, t.intrinsic_w, t.intrinsic_h, t.a_encoding_id, t.a_codec, \
                t.a_quality, t.a_bytes, r.choice, s.observer_id, s.device_pixel_ratio, \
                s.viewing_distance_cm, s.ambient_light, s.color_gamut, t.held_out \
         FROM trials t JOIN responses r ON r.trial_id = t.id \
         JOIN sessions s ON s.id = t.session_id WHERE t.kind = 'single'",
    )
    .fetch_all(pool)
    .await?;

    use std::collections::BTreeMap;
    type Key = (String, String);
    struct Bucket {
        encoding_ids: Vec<String>,
        observer_ids: Vec<String>,
        meta: BTreeMap<String, (String, Option<f32>, i64)>,
        pairs: Vec<PairObs>,
        ratings: Vec<RatingObs>,
        observers_set: std::collections::HashSet<String>,
        size: &'static str,
        image_id: String,
        held_out: bool,
    }
    let mut buckets: BTreeMap<Key, Bucket> = BTreeMap::new();
    let new_bucket = |hash: &str, w: i64, h: i64, held_out: bool| Bucket {
        encoding_ids: Vec::new(),
        observer_ids: Vec::new(),
        meta: BTreeMap::new(),
        pairs: Vec::new(),
        ratings: Vec::new(),
        observers_set: Default::default(),
        size: size_bucket(w, h),
        image_id: hash[..8.min(hash.len())].to_string(),
        held_out,
    };
    let idx_or_insert = |b: &mut Bucket, key: &str| -> usize {
        if let Some(p) = b.encoding_ids.iter().position(|s| s == key) {
            p
        } else {
            b.encoding_ids.push(key.to_string());
            b.encoding_ids.len() - 1
        }
    };
    let observer_idx = |b: &mut Bucket, oid: &str| -> usize {
        if let Some(p) = b.observer_ids.iter().position(|s| s == oid) {
            p
        } else {
            b.observer_ids.push(oid.to_string());
            b.observer_ids.len() - 1
        }
    };

    for row in pair_rows {
        let source_hash: String = row.get(0);
        let w: i64 = row.get(1);
        let h: i64 = row.get(2);
        let a_id: String = row.get(3);
        let b_id: Option<String> = row.get(4);
        let a_codec: String = row.get(5);
        let a_quality: Option<f32> = row.get::<Option<f64>, _>(6).map(|v| v as f32);
        let a_bytes: i64 = row.get(7);
        let b_codec: Option<String> = row.get(8);
        let b_quality: Option<f32> = row.get::<Option<f64>, _>(9).map(|v| v as f32);
        let b_bytes: Option<i64> = row.get(10);
        let choice: String = row.get(11);
        let observer_id: String = row.get(12);
        let dpr: f64 = row.get(13);
        let dist: Option<i64> = row.get(14);
        let amb: Option<String> = row.get(15);
        let gamut: Option<String> = row.get(16);
        let held_out: i64 = row.get(17);
        let Some(b_id) = b_id else { continue };
        let Some(b_codec) = b_codec else { continue };
        let Some(b_bytes) = b_bytes else { continue };
        let bucket_str = condition_bucket(dpr, dist, amb.as_deref(), gamut.as_deref());
        let entry = buckets
            .entry((source_hash.clone(), bucket_str.clone()))
            .or_insert_with(|| new_bucket(&source_hash, w, h, held_out != 0));
        entry
            .meta
            .entry(a_id.clone())
            .or_insert((a_codec.clone(), a_quality, a_bytes));
        entry
            .meta
            .entry(b_id.clone())
            .or_insert((b_codec.clone(), b_quality, b_bytes));
        let ia = idx_or_insert(entry, &a_id);
        let ib = idx_or_insert(entry, &b_id);
        let oi = observer_idx(entry, &observer_id);
        entry.observers_set.insert(observer_id);
        let outcome = match choice.as_str() {
            "a" => PairOutcome::AWins,
            "b" => PairOutcome::BWins,
            "tie" => PairOutcome::Tie,
            _ => continue,
        };
        entry.pairs.push(PairObs {
            item_a: ia,
            item_b: ib,
            observer: oi,
            outcome,
        });
    }
    for row in single_rows {
        let source_hash: String = row.get(0);
        let w: i64 = row.get(1);
        let h: i64 = row.get(2);
        let a_id: String = row.get(3);
        let a_codec: String = row.get(4);
        let a_quality: Option<f32> = row.get::<Option<f64>, _>(5).map(|v| v as f32);
        let a_bytes: i64 = row.get(6);
        let choice: String = row.get(7);
        let observer_id: String = row.get(8);
        let dpr: f64 = row.get(9);
        let dist: Option<i64> = row.get(10);
        let amb: Option<String> = row.get(11);
        let gamut: Option<String> = row.get(12);
        let held_out: i64 = row.get(13);
        let bucket_str = condition_bucket(dpr, dist, amb.as_deref(), gamut.as_deref());
        let entry = buckets
            .entry((source_hash.clone(), bucket_str.clone()))
            .or_insert_with(|| new_bucket(&source_hash, w, h, held_out != 0));
        entry
            .meta
            .entry(a_id.clone())
            .or_insert((a_codec.clone(), a_quality, a_bytes));
        let ia = idx_or_insert(entry, &a_id);
        let oi = observer_idx(entry, &observer_id);
        entry.observers_set.insert(observer_id);
        let Ok(rating) = choice.parse::<u8>() else {
            continue;
        };
        if !(1..=4).contains(&rating) {
            continue;
        }
        entry.ratings.push(RatingObs {
            item: ia,
            observer: oi,
            rating,
        });
    }

    for ((_source_hash, bucket_str), b) in buckets {
        if b.encoding_ids.is_empty() {
            continue;
        }
        if b.pairs.is_empty() && b.ratings.is_empty() {
            continue;
        }
        let n_items = b.encoding_ids.len();
        let n_obs = b.observer_ids.len();
        let f = fit_unified(n_items, n_obs.max(1), &b.pairs, &b.ratings);
        let qualities = m_to_quality(&f.m, 0, 10.0);

        // Bootstrap on the union of observations (resample pairs and ratings
        // proportionally — simpler than separate bootstraps).
        let mut q_samples: Vec<Vec<f32>> = vec![Vec::with_capacity(BOOTSTRAP_ITERATIONS); n_items];
        let combined: Vec<bool> = (0..(b.pairs.len() + b.ratings.len()))
            .map(|_| true)
            .collect();
        let seed = bucket_str.len() as u64 ^ b.encoding_ids.len() as u64;
        bootstrap(&combined, BOOTSTRAP_ITERATIONS / 2, seed, |_| {
            // Resample by drawing indices into the combined observation set.
            let mut rng =
                rand::rngs::SmallRng::seed_from_u64(seed.wrapping_add(q_samples.len() as u64));
            let n_total = b.pairs.len() + b.ratings.len();
            if n_total == 0 {
                return;
            }
            let mut bs_pairs: Vec<PairObs> = Vec::new();
            let mut bs_ratings: Vec<RatingObs> = Vec::new();
            for _ in 0..n_total {
                let i = rng.random_range(0..n_total);
                if i < b.pairs.len() {
                    bs_pairs.push(b.pairs[i].clone());
                } else {
                    bs_ratings.push(b.ratings[i - b.pairs.len()].clone());
                }
            }
            let bf = fit_unified(n_items, n_obs.max(1), &bs_pairs, &bs_ratings);
            let bq = m_to_quality(&bf.m, 0, 10.0);
            for (i, q) in bq.iter().enumerate() {
                q_samples[i].push(*q);
            }
        });

        for (i, eid) in b.encoding_ids.iter().enumerate() {
            let (codec, q, bytes) = b.meta.get(eid).cloned().unwrap_or_default();
            let q_target = q.unwrap_or(0.0);
            let config_name = format!("{}.q{:.0}", codec, q_target);
            let (q_lo, q_hi) = ci90(&q_samples[i]).unwrap_or((qualities[i], qualities[i]));
            out.push_str(&format!(
                "{}\t{}\t{}\t{:.0}\t{}\t{:.2}\t{:.2}\t{:.2}\t{}\t{}\t{}\t{}\t{}\n",
                b.image_id,
                b.size,
                config_name,
                q_target,
                bytes,
                qualities[i],
                q_lo,
                q_hi,
                b.observers_set.len(),
                b.pairs.len(),
                b.ratings.len(),
                bucket_str,
                if b.held_out { 1 } else { 0 },
            ));
        }
    }

    // disagreement_dummy_count is the v0.2 mitigation hook; kept as an
    // import here so future per-row diagnostics can use it without re-plumbing.
    let _ = disagreement_dummy_count;
    Ok(out)
}

pub async fn responses_tsv(pool: &SqlitePool) -> Result<String> {
    let mut out = String::new();
    out.push_str(
        "trial_id\tsession_id\tobserver_id\tkind\tsource_hash\ta_encoding_id\ta_codec\ta_quality\ta_bytes\
         \tb_encoding_id\tb_codec\tb_quality\tb_bytes\tintrinsic_w\tintrinsic_h\tstaircase_target\
         \tchoice\tdwell_ms\treveal_count\treveal_ms_total\tzoom_used\tviewport_w_css\tviewport_h_css\
         \torientation\timage_displayed_w_css\timage_displayed_h_css\tintrinsic_to_device_ratio\
         \tpixels_per_degree\tdevice_pixel_ratio\tscreen_w_css\tscreen_h_css\tcolor_gamut\
         \tdynamic_range_high\tprefers_dark\tviewing_distance_cm\tambient_light\tcss_px_per_mm\
         \tage_bracket\tvision_corrected\tresponded_at\n",
    );
    let rows = sqlx::query(
        "SELECT t.id, t.session_id, s.observer_id, t.kind, t.source_hash, t.a_encoding_id, \
                t.a_codec, t.a_quality, t.a_bytes, t.b_encoding_id, t.b_codec, t.b_quality, t.b_bytes, \
                t.intrinsic_w, t.intrinsic_h, t.staircase_target, r.choice, r.dwell_ms, r.reveal_count, \
                r.reveal_ms_total, r.zoom_used, r.viewport_w_css, r.viewport_h_css, r.orientation, \
                r.image_displayed_w_css, r.image_displayed_h_css, r.intrinsic_to_device_ratio, \
                r.pixels_per_degree, s.device_pixel_ratio, s.screen_width_css, s.screen_height_css, \
                s.color_gamut, s.dynamic_range_high, s.prefers_dark, s.viewing_distance_cm, \
                s.ambient_light, s.css_px_per_mm, o.age_bracket, o.vision_corrected, r.responded_at \
         FROM trials t \
         JOIN responses r ON r.trial_id = t.id \
         JOIN sessions  s ON s.id = t.session_id \
         JOIN observers o ON o.id = s.observer_id",
    )
    .fetch_all(pool)
    .await?;
    for row in rows {
        for i in 0..40 {
            if i > 0 {
                out.push('\t');
            }
            let v: Option<String> =
                row.try_get::<Option<String>, _>(i)
                    .ok()
                    .flatten()
                    .or_else(|| {
                        row.try_get::<Option<i64>, _>(i)
                            .ok()
                            .flatten()
                            .map(|v| v.to_string())
                            .or_else(|| {
                                row.try_get::<Option<f64>, _>(i)
                                    .ok()
                                    .flatten()
                                    .map(|v| v.to_string())
                            })
                    });
            out.push_str(v.as_deref().unwrap_or(""));
        }
        out.push('\n');
    }
    Ok(out)
}
