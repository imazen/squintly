//! TSV exports in zenanalyze/zentrain-compatible schemas.

use anyhow::Result;
use sqlx::Row;
use sqlx::SqlitePool;

use crate::bt::{Comparison, Outcome, beta_to_quality, fit, with_monotonicity};

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
    // fit BT-Davidson, and emit per-encoding rows.
    let mut out = String::new();
    out.push_str("image_id\tsize\tconfig_name\ttarget_zq\tbytes\tquality\tobservers\ttrials\tconditions_bucket\n");

    // Pull all pair responses joined with sessions for conditions.
    let rows = sqlx::query(
        "SELECT t.source_hash, t.intrinsic_w, t.intrinsic_h, t.a_encoding_id, t.a_codec, \
                t.a_quality, t.a_bytes, t.b_encoding_id, t.b_codec, t.b_quality, t.b_bytes, \
                r.choice, s.observer_id, s.device_pixel_ratio, s.viewing_distance_cm, \
                s.ambient_light, s.color_gamut \
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
        for (i, eid) in b.encoding_ids.iter().enumerate() {
            let (codec, q, bytes) = b.meta.get(eid).cloned().unwrap_or_default();
            let q_target = q.unwrap_or(0.0);
            let config_name = format!("{}.q{:.0}", codec, q_target);
            out.push_str(&format!(
                "{}\t{}\t{}\t{:.0}\t{}\t{:.2}\t{}\t{}\t{}\n",
                b.image_id,
                b.size,
                config_name,
                q_target,
                bytes,
                quality[i],
                b.observers.len(),
                n_trials,
                bucket_str,
            ));
        }
    }

    Ok(out)
}

pub async fn thresholds_tsv(pool: &SqlitePool) -> Result<String> {
    // Per (source_hash, codec, condition_bucket): aggregate single-stimulus ratings,
    // estimate q_notice / q_dislike / q_hate by logistic interpolation.
    let mut out = String::new();
    out.push_str(
        "image_id\tsize\tcodec\tconditions_bucket\tq_notice\tq_dislike\tq_hate\tobservers\ttrials\n",
    );

    let rows = sqlx::query(
        "SELECT t.source_hash, t.intrinsic_w, t.intrinsic_h, t.a_codec, t.a_quality, \
                r.choice, s.observer_id, s.device_pixel_ratio, s.viewing_distance_cm, \
                s.ambient_light, s.color_gamut \
         FROM trials t \
         JOIN responses r ON r.trial_id = t.id \
         JOIN sessions  s ON s.id = t.session_id \
         WHERE t.kind = 'single'",
    )
    .fetch_all(pool)
    .await?;

    use std::collections::BTreeMap;
    #[derive(Default)]
    struct Bucket {
        // q → (count, sum_meets_notice, sum_meets_dislike, sum_meets_hate)
        per_q: BTreeMap<i32, (i32, i32, i32, i32)>,
        observers: std::collections::HashSet<String>,
        size: &'static str,
        image_id: String,
    }
    type Key = (String, String, String); // (source_hash, codec, bucket)
    let mut buckets: BTreeMap<Key, Bucket> = BTreeMap::new();

    for row in rows {
        let source_hash: String = row.get(0);
        let w: i64 = row.get(1);
        let h: i64 = row.get(2);
        let codec: String = row.get(3);
        let q: Option<f32> = row.get(4);
        let choice: String = row.get(5);
        let observer_id: String = row.get(6);
        let dpr: f64 = row.get(7);
        let dist: Option<i64> = row.get(8);
        let amb: Option<String> = row.get(9);
        let gamut: Option<String> = row.get(10);

        let Some(q) = q else { continue };
        let q_int = q.round() as i32;
        let bucket_str = condition_bucket(dpr, dist, amb.as_deref(), gamut.as_deref());
        let key = (source_hash.clone(), codec.clone(), bucket_str);
        let entry = buckets.entry(key).or_insert_with(|| Bucket {
            per_q: Default::default(),
            observers: Default::default(),
            size: size_bucket(w, h),
            image_id: source_hash[..8.min(source_hash.len())].to_string(),
        });
        let counts = entry.per_q.entry(q_int).or_insert((0, 0, 0, 0));
        counts.0 += 1;
        let rating: i32 = choice.parse().unwrap_or(0);
        if rating >= 2 {
            counts.1 += 1;
        }
        if rating >= 3 {
            counts.2 += 1;
        }
        if rating >= 4 {
            counts.3 += 1;
        }
        entry.observers.insert(observer_id);
    }

    for ((_, codec, bucket_str), b) in buckets {
        let qs: Vec<i32> = b.per_q.keys().copied().collect();
        if qs.is_empty() {
            continue;
        }
        let q_notice = interp_threshold(&b.per_q, |c| (c.1, c.0), 0.5);
        let q_dislike = interp_threshold(&b.per_q, |c| (c.2, c.0), 0.5);
        let q_hate = interp_threshold(&b.per_q, |c| (c.3, c.0), 0.5);
        let total_trials: i32 = b.per_q.values().map(|c| c.0).sum();
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            b.image_id,
            b.size,
            codec,
            bucket_str,
            fmt_opt(q_notice),
            fmt_opt(q_dislike),
            fmt_opt(q_hate),
            b.observers.len(),
            total_trials,
        ));
    }

    Ok(out)
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
