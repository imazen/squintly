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

/// Choose one codec's encodings at random, weighted by how many quality rungs
/// it carries.
///
/// This replaces `by_codec.iter().max_by_key(|(_, v)| v.len())`. `by_codec` is
/// a BTreeMap, so on a *balanced* ladder — every codec with the same number of
/// rungs, i.e. what a well-formed corpus looks like — every codec ties and
/// `max_by_key` returns the last maximum in key order. The sampler then served
/// the alphabetically-last codec for every single trial (measured: 27/27
/// `libwebp` on imazen-26), silently voiding every cross-codec comparison.
///
/// Weighting by rung count keeps the original preference for the
/// better-sampled ladder while degenerating to uniform when they're equal.
fn choose_codec<'a, 'b>(
    by_codec: &'b std::collections::BTreeMap<&'a str, Vec<&'a EncodingMeta>>,
    min_encodings: usize,
    r: &mut impl Rng,
) -> Option<&'b Vec<&'a EncodingMeta>> {
    let eligible: Vec<&Vec<&EncodingMeta>> = by_codec
        .values()
        .filter(|v| v.len() >= min_encodings)
        .collect();
    if eligible.is_empty() {
        return None;
    }
    let total: usize = eligible.iter().map(|v| v.len()).sum();
    let mut pick = r.random_range(0..total);
    for v in eligible {
        if pick < v.len() {
            return Some(v);
        }
        pick -= v.len();
    }
    None
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
        is_golden: bool,
        expected_choice: Option<String>,
        held_out: bool,
    },
    Pair {
        source: SourceMeta,
        a: EncodingMeta,
        b: EncodingMeta,
        is_golden: bool,
        expected_choice: Option<String>,
        held_out: bool,
    },
}

pub struct SamplerConfig {
    /// Probability of sampling a Single (threshold) trial. Default 0.65.
    pub p_single: f32,
    /// Probability of overriding the random pick with a honeypot trial. CID22
    /// uses 2 of 30 = 0.067; we use 1 in 12 ≈ 0.083 because phone sessions
    /// are shorter and we want denser anchor coverage.
    pub p_honeypot: f32,
    /// Probability of overriding with an anchor (non-golden) trial when the
    /// source has registered anchors. CID22 ≈ 30% of session slots reserved.
    pub p_anchor: f32,
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            p_single: 0.65,
            p_honeypot: 0.083,
            p_anchor: 0.30,
        }
    }
}

/// In-memory pool of anchor/honeypot trials, loaded from `corpus_anchors`
/// at server start (and refreshed alongside the manifest).
#[derive(Debug, Clone, Default)]
pub struct AnchorPool {
    pub anchors: Vec<AnchorEntry>,
    pub honeypots: Vec<AnchorEntry>,
}

#[derive(Debug, Clone)]
pub struct AnchorEntry {
    pub source_hash: String,
    pub encoding_id: String,
    pub codec: String,
    pub quality: f32,
    pub expected_choice: Option<String>,
}

impl AnchorPool {
    pub fn anchors_for(&self, source_hash: &str) -> Vec<&AnchorEntry> {
        self.anchors
            .iter()
            .filter(|a| a.source_hash == source_hash)
            .collect()
    }
    pub fn honeypots_for(&self, source_hash: &str) -> Vec<&AnchorEntry> {
        self.honeypots
            .iter()
            .filter(|h| h.source_hash == source_hash)
            .collect()
    }
}

/// Source-flag lookup for the held-out validation set discipline.
#[derive(Debug, Clone, Default)]
pub struct SourceFlagMap {
    pub held_out: std::collections::HashSet<String>,
}

impl SourceFlagMap {
    pub fn is_held_out(&self, source_hash: &str) -> bool {
        self.held_out.contains(source_hash)
    }
}

/// Pick a trial. Pure function of the manifest + RNG; persistence happens elsewhere.
/// Tries the preferred trial type first, falls back to the other if the chosen source
/// can't support it. Walks sources in random order so a hostile manifest doesn't
/// starve the loop.
///
/// `allowed_codecs` filters encodings to those the observer can natively decode.
/// `None` disables the filter (server-side smoke tests, FsCoefficient direct mode).
///
/// `anchors` and `flags` are optional; when present, the sampler will mix in
/// anchor and honeypot trials per `cfg.p_anchor` / `cfg.p_honeypot`.
pub fn pick_trial(
    manifest: &Manifest,
    cfg: &SamplerConfig,
    allowed_codecs: Option<&HashSet<String>>,
    anchors: Option<&AnchorPool>,
    flags: Option<&SourceFlagMap>,
) -> Option<TrialPlan> {
    if manifest.sources.is_empty() {
        return None;
    }
    let mut r = rng();

    // First chance: honeypot. If the dice roll says so AND we have honeypots
    // for some manifest source, return one immediately.
    if let Some(pool) = anchors {
        if !pool.honeypots.is_empty() && r.random::<f32>() < cfg.p_honeypot {
            if let Some(plan) = pick_honeypot(manifest, pool, allowed_codecs, flags, &mut r) {
                return Some(plan);
            }
        }
    }

    // Second chance: anchor (non-golden). Same idea, lower probability.
    if let Some(pool) = anchors {
        if !pool.anchors.is_empty() && r.random::<f32>() < cfg.p_anchor {
            if let Some(plan) = pick_anchor(manifest, pool, allowed_codecs, flags, &mut r) {
                return Some(plan);
            }
        }
    }

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
        let held_out_src = flags.map(|f| f.is_held_out(&src.hash)).unwrap_or(false);
        let try_single = || -> Option<TrialPlan> {
            let mut r_codec = rng();
            let codec_encs = choose_codec(&by_codec, 1, &mut r_codec)?;
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
                is_golden: false,
                expected_choice: None,
                held_out: held_out_src,
            })
        };
        let try_pair = || -> Option<TrialPlan> {
            // CID22 §Selection of stimuli — drop trivial pairs whose answer
            // is foregone. Adjacent quality steps within a codec are always
            // good candidates; cross-codec pairs need a bytes-ratio sanity
            // check (see is_trivial_pair). v0.1 picks adjacent same-codec
            // pairs only, which are by construction non-trivial.
            let mut r_codec = rng();
            let codec_encs = choose_codec(&by_codec, 2, &mut r_codec)?;
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
                        is_golden: false,
                        expected_choice: None,
                        held_out: held_out_src,
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

/// Build a honeypot trial: a single-stimulus trial whose `expected_choice`
/// is known (typically reference rated `1` imperceptible, or ~q5 mozjpeg
/// rated `4` hate).
fn pick_honeypot<R: Rng + ?Sized>(
    manifest: &Manifest,
    pool: &AnchorPool,
    allowed_codecs: Option<&HashSet<String>>,
    flags: Option<&SourceFlagMap>,
    r: &mut R,
) -> Option<TrialPlan> {
    let candidates: Vec<&AnchorEntry> = pool
        .honeypots
        .iter()
        .filter(|h| codec_allowed(&h.codec, allowed_codecs))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let pick = candidates[r.random_range(0..candidates.len())];
    let source = manifest.source(&pick.source_hash)?.clone();
    let encoding = manifest.encoding(&pick.encoding_id)?.clone();
    let held_out = flags
        .map(|f| f.is_held_out(&pick.source_hash))
        .unwrap_or(false);
    Some(TrialPlan::Single {
        source,
        encoding,
        staircase_target: None,
        is_golden: true,
        expected_choice: pick.expected_choice.clone(),
        held_out,
    })
}

/// Build an anchor (non-golden) single trial against one of the source's
/// canonical (codec, quality) anchors. Anchors are drawn from
/// `corpus_anchors` with role='anchor' and serve as scale-calibration
/// reference points for the offline pipeline.
fn pick_anchor<R: Rng + ?Sized>(
    manifest: &Manifest,
    pool: &AnchorPool,
    allowed_codecs: Option<&HashSet<String>>,
    flags: Option<&SourceFlagMap>,
    r: &mut R,
) -> Option<TrialPlan> {
    let candidates: Vec<&AnchorEntry> = pool
        .anchors
        .iter()
        .filter(|a| codec_allowed(&a.codec, allowed_codecs))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let pick = candidates[r.random_range(0..candidates.len())];
    let source = manifest.source(&pick.source_hash)?.clone();
    let encoding = manifest.encoding(&pick.encoding_id)?.clone();
    let held_out = flags
        .map(|f| f.is_held_out(&pick.source_hash))
        .unwrap_or(false);
    Some(TrialPlan::Single {
        source,
        encoding,
        staircase_target: None,
        is_golden: false,
        expected_choice: None,
        held_out,
    })
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

/// Minimum number of usable pair observations on a source before ASAP EIG
/// kicks in. Below this we keep the random adjacent pair — the BT posterior
/// is dominated by the Gaussian prior and EIG is approximately uniform, so
/// adding fit-then-pick overhead buys nothing.
pub const ASAP_MIN_OBS: usize = 8;

/// ASAP EIG-based pair selection over a sorted-by-quality encoding list for
/// one (source, codec). Inputs:
///
/// - `sorted_encodings` — same-codec encodings of a single source, sorted by
///   ascending `quality` (so adjacency = nearest-quality neighbours).
/// - `comparisons` — pair observations indexed against `sorted_encodings`
///   positions.
///
/// Behaviour:
///
/// 1. Fit BT-Davidson (anchor = highest-quality index) with σ_prior = 1.0 —
///    matches pwcmp's standalone-paper recommendation. Fit is sync and fast
///    (≤ low-hundreds of iterations on the few-thousand-comparisons-per-source
///    regime we expect).
/// 2. Build the candidate set = adjacent `(i, i+1)` pairs filtered through
///    `is_trivial_pair` (mirrors `pick_trial::try_pair`).
/// 3. Pick the candidate maximising `asap::eig` under the fitted β with σ =
///    1.0 (the natural BT log-strength unit; matches the Gaussian prior).
///
/// Returns `None` when too few observations, when fewer than two encodings,
/// or when no candidate survives the trivial-pair filter — caller falls
/// back to the random adjacent pair from `pick_trial::try_pair`.
pub fn select_pair_with_eig(
    sorted_encodings: &[&crate::coefficient::EncodingMeta],
    comparisons: &[crate::bt::Comparison],
    min_obs: usize,
) -> Option<(usize, usize)> {
    if sorted_encodings.len() < 2 || comparisons.len() < min_obs {
        return None;
    }
    // Reference / highest-quality encoding is the BT anchor (β = 0). We
    // sorted ascending, so it's the last index.
    let anchor = sorted_encodings.len() - 1;
    let fit = crate::bt::fit(sorted_encodings.len(), comparisons, anchor, 1.0);

    let mut cands: Vec<(usize, usize)> = Vec::new();
    for i in 0..sorted_encodings.len() - 1 {
        if !is_trivial_pair(sorted_encodings[i], sorted_encodings[i + 1]) {
            cands.push((i, i + 1));
        }
    }
    if cands.is_empty() {
        return None;
    }
    let mut r = rng();
    crate::asap::pick_max_eig(&fit.beta, 1.0, &cands, &mut r)
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
    fn select_pair_with_eig_targets_undecided_neighbours() {
        use crate::bt::{Comparison, Outcome};

        // Five same-codec encodings at q ∈ {20, 40, 60, 80, 95}. Bytes scale
        // monotonically so no pair is trivial.
        let make = |id: &str, q: f32, b: u64| crate::coefficient::EncodingMeta {
            id: id.into(),
            source_hash: "h".into(),
            codec: "mozjpeg".into(),
            quality: Some(q),
            effort: None,
            bytes: b,
        };
        let encs = vec![
            make("q20", 20.0, 4_000),
            make("q40", 40.0, 8_000),
            make("q60", 60.0, 14_000),
            make("q80", 80.0, 22_000),
            make("q95", 95.0, 40_000),
        ];
        let sorted: Vec<&crate::coefficient::EncodingMeta> = encs.iter().collect();

        // Observations: (q20, q40) and (q60, q80) are well-resolved (high
        // index always wins), but (q40, q60) is split — that's the EIG-max
        // pair we want ASAP to surface.
        let mut comps: Vec<Comparison> = Vec::new();
        for _ in 0..10 {
            comps.push(Comparison {
                a: 0,
                b: 1,
                outcome: Outcome::BWins,
            });
            comps.push(Comparison {
                a: 2,
                b: 3,
                outcome: Outcome::BWins,
            });
        }
        for _ in 0..5 {
            comps.push(Comparison {
                a: 1,
                b: 2,
                outcome: Outcome::AWins,
            });
            comps.push(Comparison {
                a: 1,
                b: 2,
                outcome: Outcome::BWins,
            });
        }
        let pick = select_pair_with_eig(&sorted, &comps, ASAP_MIN_OBS);
        assert_eq!(
            pick,
            Some((1, 2)),
            "ASAP should target the undecided neighbour pair"
        );
    }

    #[test]
    fn select_pair_with_eig_returns_none_under_min_obs() {
        let e = |id: &str, q: f32| crate::coefficient::EncodingMeta {
            id: id.into(),
            source_hash: "h".into(),
            codec: "mozjpeg".into(),
            quality: Some(q),
            effort: None,
            bytes: 10_000,
        };
        let encs = [e("a", 40.0), e("b", 60.0)];
        let sorted: Vec<&crate::coefficient::EncodingMeta> = encs.iter().collect();
        assert!(select_pair_with_eig(&sorted, &[], ASAP_MIN_OBS).is_none());
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
            if let Some(plan) = pick_trial(
                &manifest,
                &SamplerConfig::default(),
                Some(&allowed),
                None,
                None,
            ) {
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

    /// Regression: the sampler must spread trials across every codec the
    /// observer can decode, not lock onto one.
    ///
    /// `by_codec` is a BTreeMap, and codec selection used
    /// `max_by_key(|(_, v)| v.len())`. On a *balanced* ladder — every codec
    /// carrying the same number of quality rungs, which is what a well-formed
    /// corpus looks like — every codec ties, and `max_by_key` returns the LAST
    /// maximum in iteration order, i.e. the alphabetically-last codec, every
    /// single time. Measured against the imazen-26 corpus before the fix:
    /// 27 of 27 served trials were `libwebp`, and libavif / libjpeg-turbo /
    /// jpegli were never shown at all. That silently voids every cross-codec
    /// comparison the study exists to make.
    #[test]
    fn pick_trial_spreads_across_codecs_on_a_balanced_ladder() {
        use crate::coefficient::{EncodingMeta, Manifest, SourceMeta};
        use std::collections::HashSet;

        let codecs = ["jpegli", "libavif", "libjpeg-turbo", "libwebp"];
        let mut encodings = Vec::new();
        for c in codecs {
            for (i, q) in [15.0f32, 30.0, 45.0, 60.0, 80.0, 92.0].iter().enumerate() {
                encodings.push(EncodingMeta {
                    id: format!("{c}-{i}"),
                    source_hash: "h".into(),
                    codec: c.into(),
                    quality: Some(*q),
                    effort: None,
                    bytes: 1000 + (i as u64) * 500,
                });
            }
        }
        let manifest = Manifest {
            sources: vec![SourceMeta {
                hash: "h".into(),
                width: 1024,
                height: 768,
                size_bytes: 0,
                corpus: None,
                filename: None,
            }],
            encodings,
        };
        let mut allowed = HashSet::new();
        for f in ["jpeg", "webp", "avif"] {
            allowed.insert(f.to_string());
        }

        let mut seen: HashSet<String> = HashSet::new();
        for _ in 0..300 {
            if let Some(plan) = pick_trial(
                &manifest,
                &SamplerConfig::default(),
                Some(&allowed),
                None,
                None,
            ) {
                match plan {
                    TrialPlan::Single { encoding, .. } => {
                        seen.insert(encoding.codec.clone());
                    }
                    TrialPlan::Pair { a, b, .. } => {
                        seen.insert(a.codec.clone());
                        seen.insert(b.codec.clone());
                    }
                }
            }
        }
        assert_eq!(
            seen.len(),
            4,
            "expected all 4 decodable codecs across 300 trials, saw {seen:?}"
        );
    }

}
