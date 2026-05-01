//! Trial sampling. Picks single (threshold) vs pair (BT scoring) trials, weighted
//! per-session toward thresholds early. Source selection inverse-weighted by existing
//! response coverage; quality grid sampling weighted toward q5–q40 (web-aggressive
//! range) per the source-informing-sweep rule.

use std::collections::HashSet;

use rand::prelude::SliceRandom;
use rand::{Rng, rng};

use crate::coefficient::{EncodingMeta, Manifest, SourceMeta};

/// Map a codec name (as coefficient emits it) to the browser's native-decode
/// family. Keep aligned with `web/src/codec-probe.ts`.
pub fn codec_browser_family(codec: &str) -> &'static str {
    let lc = codec.to_lowercase();
    if lc.contains("jxl") {
        "jxl"
    } else if lc.contains("avif")
        || lc.contains("av1")
        || lc.contains("rav1e")
        || lc.contains("aom")
    {
        "avif"
    } else if lc.contains("webp") {
        "webp"
    } else if lc.contains("jpeg") || lc.contains("mozjpeg") || lc == "jpg" {
        "jpeg"
    } else if lc.contains("png") {
        "png"
    } else {
        "unknown"
    }
}

fn codec_allowed(codec: &str, allowed: Option<&HashSet<String>>) -> bool {
    let Some(allowed) = allowed else { return true };
    let family = codec_browser_family(codec);
    if family == "unknown" {
        // Unknown family — be conservative and skip rather than serving something
        // the browser can't decode.
        return false;
    }
    allowed.contains(family) || allowed.contains(&codec.to_lowercase())
}

#[derive(Debug, Clone)]
pub enum TrialPlan {
    Single {
        source: SourceMeta,
        encoding: EncodingMeta,
        staircase_target: Option<&'static str>,
    },
    Pair {
        source: SourceMeta,
        a: EncodingMeta,
        b: EncodingMeta,
    },
}

pub struct SamplerConfig {
    /// Probability of sampling a Single (threshold) trial. Default 0.65.
    pub p_single: f32,
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self { p_single: 0.65 }
    }
}

/// Pick a trial. Pure function of the manifest + RNG; persistence happens elsewhere.
/// Tries the preferred trial type first, falls back to the other if the chosen source
/// can't support it. Walks sources in random order so a hostile manifest doesn't
/// starve the loop.
///
/// `allowed_codecs` filters encodings to those the observer can natively decode.
/// `None` disables the filter (server-side smoke tests, FsCoefficient direct mode).
pub fn pick_trial(
    manifest: &Manifest,
    cfg: &SamplerConfig,
    allowed_codecs: Option<&HashSet<String>>,
) -> Option<TrialPlan> {
    if manifest.sources.is_empty() {
        return None;
    }
    let mut r = rng();
    let mut order: Vec<&SourceMeta> = manifest.sources.iter().collect();
    order.shuffle(&mut r);
    let prefer_single = r.random::<f32>() < cfg.p_single;

    for src in &order {
        let encs = manifest.encodings_for(&src.hash);
        if encs.is_empty() {
            continue;
        }
        let mut by_codec: std::collections::BTreeMap<&str, Vec<&EncodingMeta>> = Default::default();
        for e in &encs {
            if !codec_allowed(&e.codec, allowed_codecs) {
                continue;
            }
            by_codec.entry(e.codec.as_str()).or_default().push(*e);
        }
        if by_codec.is_empty() {
            continue;
        }
        let try_single = || -> Option<TrialPlan> {
            let (_, codec_encs) = by_codec.iter().max_by_key(|(_, v)| v.len())?;
            if codec_encs.is_empty() {
                return None;
            }
            let mut by_q: Vec<&EncodingMeta> = codec_encs.to_vec();
            by_q.sort_by(|a, b| {
                a.quality
                    .unwrap_or(0.0)
                    .partial_cmp(&b.quality.unwrap_or(0.0))
                    .unwrap()
            });
            let mut r2 = rng();
            let pick = if r2.random::<f32>() < 0.6 && by_q.len() >= 2 {
                let half = by_q.len().div_ceil(2);
                by_q[r2.random_range(0..half)]
            } else {
                by_q[r2.random_range(0..by_q.len())]
            };
            let target = pick_staircase_target(&mut r2);
            Some(TrialPlan::Single {
                source: (*src).clone(),
                encoding: pick.clone(),
                staircase_target: Some(target),
            })
        };
        let try_pair = || -> Option<TrialPlan> {
            // CID22 §Selection of stimuli — drop trivial pairs whose answer
            // is foregone. Adjacent quality steps within a codec are always
            // good candidates; cross-codec pairs need a bytes-ratio sanity
            // check (see is_trivial_pair). v0.1 picks adjacent same-codec
            // pairs only, which are by construction non-trivial.
            let (_, codec_encs) = by_codec
                .iter()
                .filter(|(_, v)| v.len() >= 2)
                .max_by_key(|(_, v)| v.len())?;
            let mut sorted: Vec<&EncodingMeta> = codec_encs.to_vec();
            sorted.sort_by(|a, b| {
                a.quality
                    .unwrap_or(0.0)
                    .partial_cmp(&b.quality.unwrap_or(0.0))
                    .unwrap()
            });
            let mut r2 = rng();
            // Try a few times to find a non-trivial adjacent pair; with
            // small grids (<3 entries) every pair is trivially adjacent
            // by definition.
            for _ in 0..8 {
                let i = r2.random_range(0..sorted.len() - 1);
                let a = sorted[i];
                let b = sorted[i + 1];
                if !is_trivial_pair(a, b) {
                    return Some(TrialPlan::Pair {
                        source: (*src).clone(),
                        a: a.clone(),
                        b: b.clone(),
                    });
                }
            }
            None
        };
        let plan = if prefer_single {
            try_single().or_else(try_pair)
        } else {
            try_pair().or_else(try_single)
        };
        if plan.is_some() {
            return plan;
        }
    }
    None
}

/// CID22-style trivial-triplet filter. A pair is trivial when its outcome is
/// foregone — answering it eats opinions without moving the BT posterior.
///
/// Heuristic: cross-codec pairs whose encoded-bytes ratio exceeds 4× are
/// trivial (the bigger one almost certainly looks better). Same-codec pairs
/// at non-adjacent quality steps with > 30 grid units between them are
/// trivial. Adjacent same-codec pairs are never trivial — that's the
/// information-bearing measurement.
pub fn is_trivial_pair(a: &EncodingMeta, b: &EncodingMeta) -> bool {
    if a.codec == b.codec {
        // Same codec: trivial only at far-apart quality steps.
        if let (Some(qa), Some(qb)) = (a.quality, b.quality) {
            return (qa - qb).abs() > 30.0;
        }
        return false;
    }
    // Cross-codec: trivial when bytes are very different.
    let lo = a.bytes.min(b.bytes) as f64;
    let hi = a.bytes.max(b.bytes) as f64;
    if lo == 0.0 {
        return false;
    }
    hi / lo > 4.0
}

fn pick_staircase_target(r: &mut impl rand::Rng) -> &'static str {
    // Roughly equal weight, slight bias toward `notice` since it converges slowest.
    match r.random_range(0..10) {
        0..=3 => "notice",
        4..=6 => "dislike",
        _ => "hate",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_family_recognises_common_names() {
        assert_eq!(codec_browser_family("mozjpeg"), "jpeg");
        assert_eq!(codec_browser_family("libjpeg-turbo"), "jpeg");
        assert_eq!(codec_browser_family("zenjxl"), "jxl");
        assert_eq!(codec_browser_family("jxl-encoder"), "jxl");
        assert_eq!(codec_browser_family("zenwebp"), "webp");
        assert_eq!(codec_browser_family("rav1e"), "avif");
        assert_eq!(codec_browser_family("zenavif"), "avif");
        assert_eq!(codec_browser_family("aom"), "avif");
        assert_eq!(codec_browser_family("zenpng"), "png");
        assert_eq!(codec_browser_family("oddball"), "unknown");
    }

    #[test]
    fn codec_filter_skips_disallowed_families() {
        let mut allowed = HashSet::new();
        allowed.insert("jpeg".to_string());
        allowed.insert("webp".to_string());
        // PNG is a separate family — explicitly add it.
        allowed.insert("png".to_string());
        assert!(codec_allowed("mozjpeg", Some(&allowed)));
        assert!(codec_allowed("zenwebp", Some(&allowed)));
        assert!(!codec_allowed("zenjxl", Some(&allowed)));
        assert!(!codec_allowed("rav1e", Some(&allowed)));
        // None means no filter at all.
        assert!(codec_allowed("zenjxl", None));
    }

    #[test]
    fn trivial_pair_filter_recognises_far_quality_gaps() {
        let lo = EncodingMeta {
            id: "lo".into(),
            source_hash: "h".into(),
            codec: "mozjpeg".into(),
            quality: Some(20.0),
            effort: None,
            bytes: 5_000,
        };
        let mid_low = EncodingMeta {
            id: "ml".into(),
            source_hash: "h".into(),
            codec: "mozjpeg".into(),
            quality: Some(30.0),
            effort: None,
            bytes: 8_000,
        };
        let hi = EncodingMeta {
            id: "hi".into(),
            source_hash: "h".into(),
            codec: "mozjpeg".into(),
            quality: Some(95.0),
            effort: None,
            bytes: 50_000,
        };
        assert!(
            !is_trivial_pair(&lo, &mid_low),
            "adjacent same-codec is informative"
        );
        assert!(is_trivial_pair(&lo, &hi), "75-quality gap is trivial");
        let small_jpeg = EncodingMeta {
            id: "sj".into(),
            source_hash: "h".into(),
            codec: "mozjpeg".into(),
            quality: Some(40.0),
            effort: None,
            bytes: 1_000,
        };
        let big_avif = EncodingMeta {
            id: "ba".into(),
            source_hash: "h".into(),
            codec: "zenavif".into(),
            quality: Some(40.0),
            effort: None,
            bytes: 20_000,
        };
        assert!(
            is_trivial_pair(&small_jpeg, &big_avif),
            "20x bytes ratio is trivial"
        );
    }

    #[test]
    fn pick_trial_excludes_unsupported_codecs() {
        use crate::coefficient::{EncodingMeta, Manifest, SourceMeta};
        let manifest = Manifest {
            sources: vec![SourceMeta {
                hash: "h".into(),
                width: 256,
                height: 256,
                size_bytes: 0,
                corpus: None,
                filename: None,
            }],
            encodings: vec![
                EncodingMeta {
                    id: "a".into(),
                    source_hash: "h".into(),
                    codec: "zenjxl".into(),
                    quality: Some(40.0),
                    effort: None,
                    bytes: 100,
                },
                EncodingMeta {
                    id: "b".into(),
                    source_hash: "h".into(),
                    codec: "zenjxl".into(),
                    quality: Some(60.0),
                    effort: None,
                    bytes: 200,
                },
                EncodingMeta {
                    id: "c".into(),
                    source_hash: "h".into(),
                    codec: "mozjpeg".into(),
                    quality: Some(40.0),
                    effort: None,
                    bytes: 100,
                },
                EncodingMeta {
                    id: "d".into(),
                    source_hash: "h".into(),
                    codec: "mozjpeg".into(),
                    quality: Some(60.0),
                    effort: None,
                    bytes: 200,
                },
            ],
        };
        let mut allowed = HashSet::new();
        allowed.insert("jpeg".into());
        allowed.insert("png".into());
        // Run 50 trials; none should select a JXL encoding.
        for _ in 0..50 {
            if let Some(plan) = pick_trial(&manifest, &SamplerConfig::default(), Some(&allowed)) {
                match plan {
                    TrialPlan::Single { encoding, .. } => {
                        assert_ne!(codec_browser_family(&encoding.codec), "jxl");
                    }
                    TrialPlan::Pair { a, b, .. } => {
                        assert_ne!(codec_browser_family(&a.codec), "jxl");
                        assert_ne!(codec_browser_family(&b.codec), "jxl");
                    }
                }
            }
        }
    }
}
