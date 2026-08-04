//! Trial sampling. Picks single (threshold) vs pair (BT scoring) trials, weighted
//! per-session toward thresholds early. Source selection inverse-weighted by existing
//! response coverage; quality grid sampling weighted toward q5–q40 (web-aggressive
//! range) per the source-informing-sweep rule.

use std::collections::HashSet;

use rand::prelude::SliceRandom;
use rand::{Rng, rng};

use crate::coefficient::{EncodingMeta, Manifest, SourceMeta};
use crate::content_class::{ContentFilter, classify};

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

/// Does the manifest source behind this hash satisfy the study's content
/// restriction? A hash the manifest doesn't know cannot be classified, so it is
/// refused under any restriction — same fail-closed rule as an unknown stratum.
fn source_passes_content(manifest: &Manifest, source_hash: &str, content: ContentFilter) -> bool {
    match manifest.sources.iter().find(|s| s.hash == source_hash) {
        Some(s) => content.accepts(classify(s.corpus.as_deref())),
        None => content == ContentFilter::Any,
    }
}

/// Randomise which encoding lands in slot A and which in slot B.
///
/// **Position counterbalancing.** `try_pair` picks two adjacent rungs from a
/// quality-sorted list as `(sorted[i], sorted[i + 1])`, so slot B held the
/// higher-quality encoding on *every* pair trial — measured 60/60 against the
/// live deployment. In a 2AFC asking "which is closer to the original", that
/// makes B the correct answer every time: an observer who notices scores
/// perfectly without looking, and every response conflates a judgement about
/// quality with a preference for a side. Neither the Bradley-Terry fit nor a
/// SROCC against a metric can separate the two after the fact.
///
/// zenpapers `ch3-5_sampling_screening_cis.md` §4.6 names this directly — a
/// suspected side-biased UI calls for "explicit position-counterbalancing"
/// (JPEG XL CfP) before any of the per-subject modelling is meaningful.
///
/// Applied at one choke point in `handlers::next_trial`, after every path that
/// can build a pair (including the ASAP override), so no route can skip it.
/// `expected_choice` is flipped with the slots — a golden pair whose answer is
/// recorded as "a" is answered "b" once the encodings trade places, and not
/// flipping it would turn counterbalancing into a honeypot that fails everyone.
pub fn counterbalance_pair<R: Rng + ?Sized>(plan: TrialPlan, r: &mut R) -> TrialPlan {
    let TrialPlan::Pair {
        source,
        a,
        b,
        is_golden,
        expected_choice,
        held_out,
    } = plan
    else {
        return plan;
    };
    if r.random::<bool>() {
        return TrialPlan::Pair {
            source,
            a,
            b,
            is_golden,
            expected_choice,
            held_out,
        };
    }
    TrialPlan::Pair {
        source,
        a: b,
        b: a,
        is_golden,
        expected_choice: expected_choice.map(|c| match c.as_str() {
            "a" => "b".to_string(),
            "b" => "a".to_string(),
            // "tie" and anything else is position-free.
            _ => c,
        }),
        held_out,
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

/// How a pair trial's two arms are chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingRule {
    /// Two adjacent quality rungs of one codec. The codec-comparison shape:
    /// "which of these two encodes is closer to the original".
    AdjacentQuality,
    /// A restored encode against the baseline it was restored *from*, at the
    /// same quality — the artifact-removal shape.
    ///
    /// Adjacent-quality pairing cannot express this. It picks two rungs within
    /// one codec, so it would never put `mozjpeg q30` beside
    /// `zensr-dejpeg7 q30`, which is the only comparison that answers "did the
    /// restoration help". Matching on quality is the point: a restoration is
    /// judged against its own input, not against a different compression level.
    RestorationVsBaseline {
        /// Codec-name prefix identifying the restored arm.
        restored_prefix: &'static str,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct SamplerConfig {
    /// Probability of sampling a Single (threshold) trial. Default 0.65.
    ///
    /// This is the *main* study's mix (docs/STUDY.md §4.2 Type S 70% early /
    /// 50% late). It is NOT "2AFC by default" — squintly's own CLAUDE.md
    /// claimed that for a while and it was simply untrue of the code.
    pub p_single: f32,
    /// Probability of overriding the random pick with a honeypot trial. CID22
    /// uses 2 of 30 = 0.067; we use 1 in 12 ≈ 0.083 because phone sessions
    /// are shorter and we want denser anchor coverage.
    pub p_honeypot: f32,
    /// Probability of overriding with an anchor (non-golden) trial when the
    /// source has registered anchors. CID22 ≈ 30% of session slots reserved.
    pub p_anchor: f32,
    /// Which sources the study will draw from.
    ///
    /// The trial *mix* and the *content* are separate axes and both belong to
    /// the study. `ssim2-nonphoto` constrained only the mix for a while, so it
    /// served forced-choice trials over the whole corpus — including the 8
    /// photographic strata — under a label that says it is about non-photo
    /// content. See `content_class`.
    pub content: ContentFilter,
    /// How a pair's two arms are chosen. See [`PairingRule`].
    pub pairing: PairingRule,
    /// Probability of serving a **golden pair**: two encodings far enough apart
    /// that the answer is not in doubt, with `expected_choice` set.
    ///
    /// The rank-agreement study had NO controls. `p_honeypot` and `p_anchor`
    /// are zero there and had to be — both build single-stimulus trials, which
    /// a forced-choice study excludes — so nothing in it could tell a careless
    /// observer from a careful one. `is_trivial_pair` is the existing predicate
    /// for "the answer is obvious", used to *exclude* such pairs from
    /// measurement; deliberately serving a few of them is exactly an attention
    /// check, and `grading.rs` already flags `golden_fail`.
    pub p_golden_pair: f32,
    /// Serve **only** pairwise (2AFC) trials, never single-stimulus ratings.
    ///
    /// For a rank-agreement study — e.g. validating SSIMULACRA2 as the
    /// non-photo oracle (imazen/squintly#4) — forced choice is the measurement
    /// and an ACR rating is a different quantity. Mixing them silently would
    /// leave the analysis pooling two scales.
    ///
    /// This is strict on purpose: `pick_trial` normally falls back
    /// (`try_pair().or_else(try_single)`), and honeypots and anchors are both
    /// single-stimulus, so *without* this flag a "pairwise" run still emits
    /// ratings whenever a source has no non-trivial adjacent pair. Under this
    /// flag the sampler returns `None` instead — a clean 409 the operator can
    /// see, rather than contaminated data nobody notices.
    pub pairwise_only: bool,
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            p_single: 0.65,
            p_honeypot: 0.083,
            p_anchor: 0.30,
            content: ContentFilter::Any,
            pairing: PairingRule::AdjacentQuality,
            p_golden_pair: 0.0,
            pairwise_only: false,
        }
    }
}

impl SamplerConfig {
    /// Read the trial mix from the environment, once, at startup.
    ///
    /// Lets a validation run be configured without a rebuild:
    ///   SQUINTLY_PAIRWISE_ONLY=1   — pure 2AFC, no ratings at all
    ///   SQUINTLY_P_SINGLE=0.2      — mostly pairwise, ratings still allowed
    /// Out-of-range probabilities are clamped rather than rejected; an
    /// unparseable value keeps the default and warns.
    pub fn from_env() -> Self {
        let d = Self::default();
        let prob = |key: &str, default: f32| -> f32 {
            match std::env::var(key) {
                Ok(v) => match v.trim().parse::<f32>() {
                    Ok(p) => p.clamp(0.0, 1.0),
                    Err(_) => {
                        tracing::warn!(key, value = %v, "unparseable probability; using default");
                        default
                    }
                },
                Err(_) => default,
            }
        };
        let pairwise_only = std::env::var("SQUINTLY_PAIRWISE_ONLY")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        Self {
            // Pairwise-only implies no single-stimulus trials at all; keep
            // p_single consistent so the two settings can't disagree.
            p_single: if pairwise_only {
                0.0
            } else {
                prob("SQUINTLY_P_SINGLE", d.p_single)
            },
            p_honeypot: prob("SQUINTLY_P_HONEYPOT", d.p_honeypot),
            p_anchor: prob("SQUINTLY_P_ANCHOR", d.p_anchor),
            content: d.content,
            pairing: d.pairing,
            p_golden_pair: d.p_golden_pair,
            pairwise_only,
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
    // Resolved once per trial, before any path consults it — honeypots and
    // anchors filter on content too, so leaving them the unresolved `Mixed`
    // would silently ignore the fraction. (Caught by the debug_assert in
    // `ContentFilter::accepts`.)
    let content = cfg.content.resolve_for_draw(r.random::<f32>());

    // Honeypots and anchors are both single-stimulus (see pick_honeypot /
    // pick_anchor), so a pairwise-only run must not inject them — they would
    // be exactly the ACR ratings the mode exists to exclude.
    let inject_singles = !cfg.pairwise_only;

    // First chance: honeypot. If the dice roll says so AND we have honeypots
    // for some manifest source, return one immediately.
    if let Some(pool) = anchors.filter(|_| inject_singles) {
        if !pool.honeypots.is_empty() && r.random::<f32>() < cfg.p_honeypot {
            if let Some(plan) =
                pick_honeypot(manifest, pool, allowed_codecs, flags, content, &mut r)
            {
                return Some(plan);
            }
        }
    }

    // Second chance: anchor (non-golden). Same idea, lower probability.
    if let Some(pool) = anchors.filter(|_| inject_singles) {
        if !pool.anchors.is_empty() && r.random::<f32>() < cfg.p_anchor {
            if let Some(plan) = pick_anchor(manifest, pool, allowed_codecs, flags, content, &mut r)
            {
                return Some(plan);
            }
        }
    }

    // Content restriction applies to the *whole* draw, honeypots and anchors
    // included — an anchor from a photographic stratum is still a photo trial
    // filed under a non-photo study.
    let eligible = |f: ContentFilter| -> Vec<&SourceMeta> {
        manifest
            .sources
            .iter()
            .filter(|s| f.accepts(classify(s.corpus.as_deref())))
            .collect()
    };
    let mut order: Vec<&SourceMeta> = eligible(content);
    // A mixed study states a preferred RATIO, not a requirement that the corpus
    // hold both classes. When the drawn class is absent, serve the other rather
    // than refusing a quarter of the time — an intermittent 409 whose frequency
    // tracks a probability is a genuinely baffling thing to debug.
    //
    // This does not weaken the "unknown strata are refused" rule: both classes
    // here are ones the study already declares it draws from.
    if order.is_empty() {
        if let ContentFilter::Mixed { .. } = cfg.content {
            let other = if content == ContentFilter::PhotoOnly {
                ContentFilter::NonPhotoOnly
            } else {
                ContentFilter::PhotoOnly
            };
            order = eligible(other);
        }
    }
    if order.is_empty() {
        // No fallback to the unrestricted pool. Serving a photo here is exactly
        // the bug this filter exists to fix; a caller seeing None reports a
        // clear shortage instead.
        return None;
    }
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
        // Pair a restored encode against the baseline it was restored from,
        // at the same quality. Returns `None` when this source has no such
        // pair — the caller moves on to the next source rather than falling
        // back to an adjacent-quality pair, which would silently answer a
        // different question.
        let try_restoration = |restored_prefix: &str| -> Option<TrialPlan> {
            let all: Vec<&EncodingMeta> = by_codec.values().flatten().copied().collect();
            let mut cands: Vec<(&EncodingMeta, &EncodingMeta)> = Vec::new();
            for r in all.iter().filter(|e| e.codec.starts_with(restored_prefix)) {
                for b in all.iter().filter(|e| !e.codec.starts_with(restored_prefix)) {
                    // Same quality is what makes this a restoration comparison
                    // rather than a compression-level comparison.
                    let (Some(rq), Some(bq)) = (r.quality, b.quality) else {
                        continue;
                    };
                    if (rq - bq).abs() < f32::EPSILON {
                        cands.push((b, r));
                    }
                }
            }
            if cands.is_empty() {
                return None;
            }
            let mut r2 = rng();
            let (base, restored) = cands[r2.random_range(0..cands.len())];
            Some(TrialPlan::Pair {
                source: (*src).clone(),
                // Slot order here is arbitrary and does not survive anyway:
                // `counterbalance_pair` randomises it at the choke point.
                a: base.clone(),
                b: restored.clone(),
                is_golden: false,
                expected_choice: None,
                held_out: held_out_src,
            })
        };

        // A pair whose answer is not in doubt. `is_trivial_pair` is the
        // project's existing definition of "obvious"; measurement excludes
        // these, so serving one is a pure attention check.
        let try_golden_pair = || -> Option<TrialPlan> {
            let mut r_codec = rng();
            let codec_encs = choose_codec(&by_codec, 2, &mut r_codec)?;
            let mut sorted: Vec<&EncodingMeta> = codec_encs.to_vec();
            sorted.sort_by(|a, b| {
                a.quality
                    .unwrap_or(0.0)
                    .partial_cmp(&b.quality.unwrap_or(0.0))
                    .unwrap()
            });
            let lo = *sorted.first()?;
            let hi = *sorted.last()?;
            if !is_trivial_pair(lo, hi) {
                // This source's ladder is too narrow to be unambiguous. Better
                // no control than one whose "correct" answer is arguable.
                return None;
            }
            Some(TrialPlan::Pair {
                source: (*src).clone(),
                a: lo.clone(),
                b: hi.clone(),
                is_golden: true,
                // The higher-quality arm is the one closer to the original.
                // `counterbalance_pair` flips this with the slots.
                expected_choice: Some("b".to_string()),
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
        // The fallback is what makes a "pairwise" run leak ratings: a source
        // with no non-trivial adjacent pair silently yields a single. Under
        // pairwise_only there is no fallback — we move on to the next source,
        // and if none can produce a pair the caller gets a clean 409.
        let pair_fn = || match cfg.pairing {
            _ if rng().random::<f32>() < cfg.p_golden_pair => {
                // Fall through to a real pair when this source cannot make an
                // unambiguous one, rather than skipping the trial.
                try_golden_pair().or_else(|| match cfg.pairing {
                    PairingRule::AdjacentQuality => try_pair(),
                    PairingRule::RestorationVsBaseline { restored_prefix } => {
                        try_restoration(restored_prefix)
                    }
                })
            }
            PairingRule::AdjacentQuality => try_pair(),
            PairingRule::RestorationVsBaseline { restored_prefix } => {
                try_restoration(restored_prefix)
            }
        };
        let plan = if cfg.pairwise_only {
            pair_fn()
        } else if prefer_single {
            try_single().or_else(pair_fn)
        } else {
            pair_fn().or_else(try_single)
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
    content: ContentFilter,
    r: &mut R,
) -> Option<TrialPlan> {
    // A honeypot from a photographic stratum is still a photo trial, so the
    // content restriction has to reach here too — not just the general draw.
    let candidates: Vec<&AnchorEntry> = pool
        .honeypots
        .iter()
        .filter(|h| codec_allowed(&h.codec, allowed_codecs))
        .filter(|h| source_passes_content(manifest, &h.source_hash, content))
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
    content: ContentFilter,
    r: &mut R,
) -> Option<TrialPlan> {
    let candidates: Vec<&AnchorEntry> = pool
        .anchors
        .iter()
        .filter(|a| codec_allowed(&a.codec, allowed_codecs))
        .filter(|a| source_passes_content(manifest, &a.source_hash, content))
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
        let encs = [
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

    /// A pairwise-only run must never emit a single-stimulus trial.
    ///
    /// The default path is `try_pair().or_else(try_single)`, and honeypots and
    /// anchors are themselves single-stimulus, so "set p_single = 0" is NOT
    /// enough — a source with no non-trivial adjacent pair still yields a
    /// rating. For a rank-agreement study (imazen/squintly#4) that silently
    /// mixes two different scales into one analysis.
    /// The bug this filter fixes: `ssim2-nonphoto` drew from the whole corpus,
    /// so photographic strata appeared in a study whose name says they cannot.
    #[test]
    fn a_non_photo_study_never_draws_a_photographic_source() {
        // A manifest shaped like the live one: photo and non-photo strata mixed.
        let strata = [
            ("imazen26-1400-lilith-nature", true),
            ("imazen26-2000-unsplash-people", true),
            ("imazen26-3300-met-museum-photos", true),
            ("imazen26-8000-lilith-mobile-screenshots", false),
            ("imazen26-7000-lilith-plots", false),
            ("imazen26-6800-ia-scans-manuscript-text", false),
        ];
        let mut sources = Vec::new();
        let mut encodings = Vec::new();
        for (i, (corpus, _)) in strata.iter().enumerate() {
            let hash = format!("src{i}");
            sources.push(SourceMeta {
                hash: hash.clone(),
                width: 512,
                height: 512,
                size_bytes: 100_000,
                corpus: Some((*corpus).to_string()),
                filename: Some(format!("{corpus}.png")),
            });
            for (j, q) in [20.0f32, 40.0, 60.0, 80.0].iter().enumerate() {
                encodings.push(EncodingMeta {
                    id: format!("{hash}-e{j}"),
                    source_hash: hash.clone(),
                    codec: "mozjpeg".into(),
                    quality: Some(*q),
                    effort: None,
                    bytes: (5_000 * (j + 1)) as u64,
                });
            }
        }
        let manifest = Manifest { sources, encodings };

        let cfg = SamplerConfig {
            p_single: 0.0,
            p_honeypot: 0.0,
            p_anchor: 0.0,
            content: ContentFilter::NonPhotoOnly,
            pairing: PairingRule::AdjacentQuality,
            p_golden_pair: 0.0,
            pairwise_only: true,
        };

        let photo: std::collections::HashSet<&str> = strata
            .iter()
            .filter(|(_, is_photo)| *is_photo)
            .map(|(c, _)| *c)
            .collect();

        let mut seen = std::collections::HashSet::new();
        for _ in 0..400 {
            let plan = pick_trial(&manifest, &cfg, None, None, None).expect("pool is non-empty");
            let corpus = match &plan {
                TrialPlan::Single { source, .. } => source.corpus.clone(),
                TrialPlan::Pair { source, .. } => source.corpus.clone(),
            }
            .unwrap();
            assert!(
                !photo.contains(corpus.as_str()),
                "non-photo study served a photographic stratum: {corpus}"
            );
            seen.insert(corpus);
        }
        // ...and it must reach all three non-photo strata, not collapse onto one.
        assert_eq!(
            seen.len(),
            3,
            "expected every non-photo stratum, saw {seen:?}"
        );
    }

    /// An unregistered stratum must not slip into a restricted study, and an
    /// empty pool must refuse rather than quietly fall back to the full corpus
    /// — falling back is precisely the original bug.
    #[test]
    fn an_unclassifiable_pool_refuses_rather_than_falling_back() {
        let manifest = Manifest {
            sources: vec![SourceMeta {
                hash: "s1".into(),
                width: 512,
                height: 512,
                size_bytes: 1000,
                corpus: Some("some-stratum-added-later".into()),
                filename: None,
            }],
            encodings: (0..4)
                .map(|j| EncodingMeta {
                    id: format!("e{j}"),
                    source_hash: "s1".into(),
                    codec: "mozjpeg".into(),
                    quality: Some(20.0 * (j + 1) as f32),
                    effort: None,
                    bytes: 5_000 * (j + 1),
                })
                .collect(),
        };
        let restricted = SamplerConfig {
            content: ContentFilter::NonPhotoOnly,
            ..SamplerConfig::default()
        };
        assert!(
            pick_trial(&manifest, &restricted, None, None, None).is_none(),
            "an unknown stratum must not be admitted to a restricted study"
        );
        // The same manifest still serves an unrestricted study.
        assert!(pick_trial(&manifest, &SamplerConfig::default(), None, None, None).is_some());
    }

    /// The bug: `try_pair` returns `(sorted[i], sorted[i+1])` from a
    /// quality-ascending list, so slot B was the better image on every trial —
    /// 60/60 measured live. In a "which is closer to the original" task that
    /// makes the answer constant, and no downstream fit can separate a quality
    /// judgement from a side preference after the fact.
    /// Artifact removal asks a different question from codec comparison, and
    /// adjacent-quality pairing cannot express it: it picks two rungs within
    /// one codec, so it would never put `mozjpeg q30` beside its own restored
    /// output. Matching on quality is what makes the pair a restoration
    /// comparison rather than a compression-level one.
    #[test]
    fn restoration_pairing_matches_a_restored_encode_to_its_own_input() {
        let src = SourceMeta {
            hash: "s1".into(),
            width: 512,
            height: 512,
            size_bytes: 1000,
            corpus: Some("imazen26-7000-lilith-plots".into()),
            filename: None,
        };
        let enc = |id: &str, codec: &str, q: f32| EncodingMeta {
            id: id.into(),
            source_hash: "s1".into(),
            codec: codec.into(),
            quality: Some(q),
            effort: None,
            bytes: (q as u64) * 100,
        };
        let manifest = Manifest {
            sources: vec![src],
            encodings: vec![
                enc("m30", "mozjpeg", 30.0),
                enc("m60", "mozjpeg", 60.0),
                enc("z30", "zensr-dejpeg7", 30.0),
                enc("z60", "zensr-dejpeg7", 60.0),
            ],
        };
        let cfg = SamplerConfig {
            p_single: 0.0,
            p_honeypot: 0.0,
            p_anchor: 0.0,
            content: ContentFilter::Any,
            pairing: PairingRule::RestorationVsBaseline {
                restored_prefix: "zensr",
            },
            p_golden_pair: 0.0,
            pairwise_only: true,
        };

        let mut seen = std::collections::HashSet::new();
        for _ in 0..300 {
            let plan = pick_trial(&manifest, &cfg, None, None, None).expect("a pair exists");
            let TrialPlan::Pair { a, b, .. } = plan else {
                panic!("restoration study must never emit a single");
            };
            let restored: Vec<&EncodingMeta> = [&a, &b]
                .into_iter()
                .filter(|e| e.codec.starts_with("zensr"))
                .collect();
            assert_eq!(
                restored.len(),
                1,
                "exactly one arm must be the restoration, got {} and {}",
                a.codec,
                b.codec
            );
            assert_eq!(
                a.quality, b.quality,
                "a restoration is judged against its own input, so the qualities must match: \
                 {a:?} vs {b:?}"
            );
            let mut ids = [a.id.clone(), b.id.clone()];
            ids.sort();
            seen.insert(ids.join("+"));
        }
        // Both matched pairs should appear; neither cross-quality pair should.
        assert!(
            seen.contains("m30+z30"),
            "expected the q30 pair, saw {seen:?}"
        );
        assert!(
            seen.contains("m60+z60"),
            "expected the q60 pair, saw {seen:?}"
        );
        assert_eq!(
            seen.len(),
            2,
            "only quality-matched pairs are valid: {seen:?}"
        );
    }

    /// A corpus with no restorations must yield nothing rather than quietly
    /// falling back to an adjacent-quality pair — that would answer a
    /// different question under the artifact-removal label.
    #[test]
    fn restoration_pairing_refuses_rather_than_falling_back() {
        let src = SourceMeta {
            hash: "s1".into(),
            width: 512,
            height: 512,
            size_bytes: 1000,
            corpus: Some("imazen26-7000-lilith-plots".into()),
            filename: None,
        };
        let enc = |id: &str, q: f32| EncodingMeta {
            id: id.into(),
            source_hash: "s1".into(),
            codec: "mozjpeg".into(),
            quality: Some(q),
            effort: None,
            bytes: 1000,
        };
        let manifest = Manifest {
            sources: vec![src],
            encodings: vec![
                enc("a", 20.0),
                enc("b", 40.0),
                enc("c", 60.0),
                enc("d", 80.0),
            ],
        };
        let cfg = SamplerConfig {
            p_single: 0.0,
            p_honeypot: 0.0,
            p_anchor: 0.0,
            content: ContentFilter::Any,
            pairing: PairingRule::RestorationVsBaseline {
                restored_prefix: "zensr",
            },
            p_golden_pair: 0.0,
            pairwise_only: true,
        };
        assert!(
            pick_trial(&manifest, &cfg, None, None, None).is_none(),
            "no restorations in the corpus must mean no trial, not a codec pair"
        );
        // The same manifest still serves the ordinary adjacent-quality study.
        let normal = SamplerConfig {
            pairwise_only: true,
            p_single: 0.0,
            ..SamplerConfig::default()
        };
        assert!(pick_trial(&manifest, &normal, None, None, None).is_some());
    }

    /// The rank-agreement study had no controls at all: `p_honeypot` and
    /// `p_anchor` are zero there and must be, because both build
    /// single-stimulus trials that a forced-choice study excludes. Nothing in
    /// it could tell a careful observer from a careless one.
    #[test]
    fn golden_pairs_are_unambiguous_and_carry_the_right_answer() {
        let src = SourceMeta {
            hash: "s1".into(),
            width: 512,
            height: 512,
            size_bytes: 1000,
            corpus: Some("imazen26-7000-lilith-plots".into()),
            filename: None,
        };
        let enc = |id: &str, q: f32| EncodingMeta {
            id: id.into(),
            source_hash: "s1".into(),
            codec: "mozjpeg".into(),
            quality: Some(q),
            effort: None,
            bytes: (q as u64) * 200,
        };
        // A wide ladder, so the extremes are unambiguous.
        let manifest = Manifest {
            sources: vec![src],
            encodings: vec![enc("q10", 10.0), enc("q40", 40.0), enc("q95", 95.0)],
        };
        let cfg = SamplerConfig {
            p_single: 0.0,
            p_honeypot: 0.0,
            p_anchor: 0.0,
            content: ContentFilter::Any,
            pairing: PairingRule::AdjacentQuality,
            p_golden_pair: 1.0,
            pairwise_only: true,
        };

        let mut goldens = 0;
        for _ in 0..200 {
            let plan = pick_trial(&manifest, &cfg, None, None, None).expect("a pair exists");
            let TrialPlan::Pair {
                a,
                b,
                is_golden,
                expected_choice,
                ..
            } = plan
            else {
                panic!("forced-choice study must never emit a single");
            };
            if !is_golden {
                continue;
            }
            goldens += 1;
            assert!(
                is_trivial_pair(&a, &b),
                "a control whose answer is arguable is not a control: {a:?} vs {b:?}"
            );
            // The expected answer must name the slot actually holding the
            // better encoding — counterbalancing flips both together.
            let better = if a.quality.unwrap() > b.quality.unwrap() {
                "a"
            } else {
                "b"
            };
            assert_eq!(
                expected_choice.as_deref(),
                Some(better),
                "expected_choice must follow the encoding, not the slot"
            );
        }
        assert!(goldens > 0, "p_golden_pair = 1.0 produced no goldens");
    }

    /// A source whose ladder is too narrow to be unambiguous must fall back to
    /// a real measurement pair rather than serve a control whose "correct"
    /// answer is arguable.
    #[test]
    fn a_narrow_ladder_yields_no_golden_rather_than_a_dubious_one() {
        let src = SourceMeta {
            hash: "s1".into(),
            width: 512,
            height: 512,
            size_bytes: 1000,
            corpus: Some("imazen26-7000-lilith-plots".into()),
            filename: None,
        };
        let enc = |id: &str, q: f32| EncodingMeta {
            id: id.into(),
            source_hash: "s1".into(),
            codec: "mozjpeg".into(),
            quality: Some(q),
            effort: None,
            bytes: 10_000,
        };
        // 20 points apart: under the 30-point trivial threshold.
        let manifest = Manifest {
            sources: vec![src],
            encodings: vec![enc("q40", 40.0), enc("q50", 50.0), enc("q60", 60.0)],
        };
        let cfg = SamplerConfig {
            p_single: 0.0,
            p_honeypot: 0.0,
            p_anchor: 0.0,
            content: ContentFilter::Any,
            pairing: PairingRule::AdjacentQuality,
            p_golden_pair: 1.0,
            pairwise_only: true,
        };
        for _ in 0..100 {
            let plan = pick_trial(&manifest, &cfg, None, None, None).expect("a pair exists");
            let TrialPlan::Pair { is_golden, .. } = plan else {
                panic!("expected a pair");
            };
            assert!(
                !is_golden,
                "no pair here is unambiguous, so none may be a control"
            );
        }
    }

    /// A photographic control run as a separate study confounds content with
    /// session — fatigue, lighting, screen and adaptation all differ between
    /// sessions. Interleaving makes the comparison within-session by
    /// construction, which is the only thing that licenses attributing a gap
    /// to the content rather than the sitting.
    #[test]
    fn a_mixed_study_interleaves_both_classes_at_the_declared_ratio() {
        let mk = |hash: &str, corpus: &str| SourceMeta {
            hash: hash.into(),
            width: 512,
            height: 512,
            size_bytes: 1000,
            corpus: Some(corpus.into()),
            filename: None,
        };
        let enc = |src: &str, id: &str, q: f32| EncodingMeta {
            id: id.into(),
            source_hash: src.into(),
            codec: "mozjpeg".into(),
            quality: Some(q),
            effort: None,
            bytes: (q as u64) * 100,
        };
        let mut sources = Vec::new();
        let mut encodings = Vec::new();
        for (h, c) in [
            ("np1", "imazen26-7000-lilith-plots"),
            ("np2", "imazen26-8100-lilith-web-screenshots"),
            ("ph1", "imazen26-1400-lilith-nature"),
            ("ph2", "imazen26-1600-lilith-food"),
        ] {
            sources.push(mk(h, c));
            for (i, q) in [20.0f32, 40.0, 60.0, 80.0].iter().enumerate() {
                encodings.push(enc(h, &format!("{h}-e{i}"), *q));
            }
        }
        let manifest = Manifest { sources, encodings };
        let cfg = SamplerConfig {
            p_single: 0.0,
            p_honeypot: 0.0,
            p_anchor: 0.0,
            content: ContentFilter::Mixed {
                photo_fraction: 0.25,
            },
            pairing: PairingRule::AdjacentQuality,
            p_golden_pair: 0.0,
            pairwise_only: true,
        };

        let mut photo = 0usize;
        const N: usize = 4000;
        for _ in 0..N {
            let plan = pick_trial(&manifest, &cfg, None, None, None).expect("a pair exists");
            let corpus = match &plan {
                TrialPlan::Single { source, .. } | TrialPlan::Pair { source, .. } => {
                    source.corpus.clone().unwrap()
                }
            };
            if classify(Some(&corpus)) == crate::content_class::ContentClass::Photo {
                photo += 1;
            }
        }
        let frac = photo as f64 / N as f64;
        // Binomial(4000, 0.25): +/-0.03 is far outside sampling noise.
        assert!(
            (0.22..=0.28).contains(&frac),
            "photo share {:.3}, expected ~0.25 — a session must interleave, not commit",
            frac
        );
    }

    /// Resolution happens per DRAW. Deciding once per session would make each
    /// session entirely one class, which is exactly the confound interleaving
    /// removes.
    #[test]
    fn a_mixed_filter_resolves_per_draw_not_once() {
        let m = ContentFilter::Mixed {
            photo_fraction: 0.25,
        };
        assert_eq!(m.resolve_for_draw(0.0), ContentFilter::PhotoOnly);
        assert_eq!(m.resolve_for_draw(0.24), ContentFilter::PhotoOnly);
        assert_eq!(m.resolve_for_draw(0.25), ContentFilter::NonPhotoOnly);
        assert_eq!(m.resolve_for_draw(0.99), ContentFilter::NonPhotoOnly);
        // A concrete filter is unaffected by the roll.
        for roll in [0.0, 0.5, 1.0] {
            assert_eq!(
                ContentFilter::NonPhotoOnly.resolve_for_draw(roll),
                ContentFilter::NonPhotoOnly
            );
            assert_eq!(
                ContentFilter::Any.resolve_for_draw(roll),
                ContentFilter::Any
            );
        }
    }

    #[test]
    fn pair_slots_are_counterbalanced() {
        let src = SourceMeta {
            hash: "s1".into(),
            width: 512,
            height: 512,
            size_bytes: 1000,
            corpus: Some("imazen26-7000-lilith-plots".into()),
            filename: None,
        };
        let enc = |id: &str, q: f32| EncodingMeta {
            id: id.into(),
            source_hash: "s1".into(),
            codec: "mozjpeg".into(),
            quality: Some(q),
            effort: None,
            bytes: (q as u64) * 100,
        };

        let mut r = rng();
        let mut b_better = 0;
        const N: usize = 2000;
        for _ in 0..N {
            let plan = TrialPlan::Pair {
                source: src.clone(),
                a: enc("low", 30.0),
                b: enc("high", 60.0),
                is_golden: false,
                expected_choice: None,
                held_out: false,
            };
            if let TrialPlan::Pair { a, b, .. } = counterbalance_pair(plan, &mut r) {
                assert_ne!(a.id, b.id, "counterbalancing must not duplicate a side");
                if b.quality.unwrap() > a.quality.unwrap() {
                    b_better += 1;
                }
            }
        }
        // Binomial(2000, 0.5): ±5% is ~4.5 sigma, so this is tight without
        // being flaky.
        let frac = b_better as f64 / N as f64;
        assert!(
            (0.45..=0.55).contains(&frac),
            "better image landed in B {:.1}% of the time; expected ~50%",
            frac * 100.0
        );
    }

    /// A golden pair records which side is correct. Swapping the encodings
    /// without swapping the answer would turn counterbalancing into a honeypot
    /// that every honest observer fails.
    #[test]
    fn counterbalancing_flips_the_expected_answer_with_the_slots() {
        let src = SourceMeta {
            hash: "s1".into(),
            width: 512,
            height: 512,
            size_bytes: 1000,
            corpus: None,
            filename: None,
        };
        let enc = |id: &str, q: f32| EncodingMeta {
            id: id.into(),
            source_hash: "s1".into(),
            codec: "mozjpeg".into(),
            quality: Some(q),
            effort: None,
            bytes: 1000,
        };
        let mut r = rng();
        let mut swapped = 0;
        let mut kept = 0;
        for _ in 0..500 {
            let plan = TrialPlan::Pair {
                source: src.clone(),
                a: enc("worse", 30.0),
                b: enc("better", 60.0),
                is_golden: true,
                // The better image is "b" in the unswapped layout.
                expected_choice: Some("b".to_string()),
                held_out: false,
            };
            // Slot A identifies the layout on its own; B is implied.
            let TrialPlan::Pair {
                a, expected_choice, ..
            } = counterbalance_pair(plan, &mut r)
            else {
                unreachable!()
            };
            // Whatever the layout, the expected answer must name the slot that
            // actually holds the better encoding.
            let better_slot = if a.id == "better" { "a" } else { "b" };
            assert_eq!(
                expected_choice.as_deref(),
                Some(better_slot),
                "expected_choice must follow the encoding, not the slot"
            );
            if a.id == "better" {
                swapped += 1;
            } else {
                kept += 1;
            }
        }
        assert!(swapped > 0 && kept > 0, "both layouts must occur");
    }

    /// "tie" names no side, so it must survive a swap untouched.
    #[test]
    fn a_tie_expectation_is_position_free() {
        let src = SourceMeta {
            hash: "s1".into(),
            width: 1,
            height: 1,
            size_bytes: 1,
            corpus: None,
            filename: None,
        };
        let enc = |id: &str| EncodingMeta {
            id: id.into(),
            source_hash: "s1".into(),
            codec: "mozjpeg".into(),
            quality: Some(50.0),
            effort: None,
            bytes: 1,
        };
        let mut r = rng();
        for _ in 0..50 {
            let plan = TrialPlan::Pair {
                source: src.clone(),
                a: enc("x"),
                b: enc("y"),
                is_golden: true,
                expected_choice: Some("tie".to_string()),
                held_out: false,
            };
            let TrialPlan::Pair {
                expected_choice, ..
            } = counterbalance_pair(plan, &mut r)
            else {
                unreachable!()
            };
            assert_eq!(expected_choice.as_deref(), Some("tie"));
        }
    }

    #[test]
    fn pairwise_only_never_emits_a_single() {
        use crate::coefficient::{EncodingMeta, Manifest, SourceMeta};

        // A source whose encodings cannot form a non-trivial pair (one rung),
        // alongside one that can. The single-rung source is exactly the case
        // that used to fall back to a rating.
        let mut encodings = vec![EncodingMeta {
            id: "lonely".into(),
            source_hash: "thin".into(),
            codec: "libwebp".into(),
            quality: Some(50.0),
            effort: None,
            bytes: 5_000,
        }];
        for (i, q) in [15.0f32, 30.0, 45.0, 60.0].iter().enumerate() {
            encodings.push(EncodingMeta {
                id: format!("rich-{i}"),
                source_hash: "rich".into(),
                codec: "libwebp".into(),
                quality: Some(*q),
                effort: None,
                bytes: 4_000 + (i as u64) * 900,
            });
        }
        let src = |h: &str| SourceMeta {
            hash: h.into(),
            width: 1024,
            height: 768,
            size_bytes: 0,
            corpus: None,
            filename: None,
        };
        let manifest = Manifest {
            sources: vec![src("thin"), src("rich")],
            encodings,
        };
        // Honeypots present and at probability 1.0: without the guard these
        // would be returned immediately, and every one of them is a Single.
        let pool = AnchorPool {
            anchors: vec![],
            honeypots: vec![AnchorEntry {
                source_hash: "rich".into(),
                encoding_id: "rich-0".into(),
                codec: "libwebp".into(),
                quality: 15.0,
                expected_choice: Some("4".into()),
            }],
        };
        let cfg = SamplerConfig {
            p_single: 0.0,
            p_honeypot: 1.0,
            p_anchor: 1.0,
            content: ContentFilter::Any,
            pairing: PairingRule::AdjacentQuality,
            p_golden_pair: 0.0,
            pairwise_only: true,
        };
        let mut allowed = HashSet::new();
        allowed.insert("webp".to_string());

        let mut pairs = 0;
        for _ in 0..300 {
            match pick_trial(&manifest, &cfg, Some(&allowed), Some(&pool), None) {
                Some(TrialPlan::Pair { .. }) => pairs += 1,
                Some(TrialPlan::Single { .. }) => {
                    panic!("pairwise_only emitted a single-stimulus trial")
                }
                None => {}
            }
        }
        assert!(pairs > 0, "expected some pairs to be servable");
    }

    /// Without the flag, the same manifest *does* produce singles — proving the
    /// test above is exercising the guard and not a manifest that simply can't
    /// make one.
    #[test]
    fn default_mix_still_emits_singles() {
        use crate::coefficient::{EncodingMeta, Manifest, SourceMeta};
        let encodings = vec![EncodingMeta {
            id: "lonely".into(),
            source_hash: "thin".into(),
            codec: "libwebp".into(),
            quality: Some(50.0),
            effort: None,
            bytes: 5_000,
        }];
        let manifest = Manifest {
            sources: vec![SourceMeta {
                hash: "thin".into(),
                width: 1024,
                height: 768,
                size_bytes: 0,
                corpus: None,
                filename: None,
            }],
            encodings,
        };
        let mut allowed = HashSet::new();
        allowed.insert("webp".to_string());
        let singles = (0..50)
            .filter_map(|_| {
                pick_trial(
                    &manifest,
                    &SamplerConfig::default(),
                    Some(&allowed),
                    None,
                    None,
                )
            })
            .filter(|p| matches!(p, TrialPlan::Single { .. }))
            .count();
        assert!(
            singles > 0,
            "a one-rung source should fall back to a single"
        );
    }

    #[test]
    fn from_env_pairwise_only_forces_p_single_to_zero() {
        // SAFETY: single test owning these vars; cargo runs tests in parallel
        // threads so they must not be split across tests.
        unsafe { std::env::set_var("SQUINTLY_PAIRWISE_ONLY", "1") };
        unsafe { std::env::set_var("SQUINTLY_P_SINGLE", "0.9") };
        let c = SamplerConfig::from_env();
        assert!(c.pairwise_only);
        assert_eq!(c.p_single, 0.0, "pairwise_only must override p_single");
        unsafe { std::env::remove_var("SQUINTLY_PAIRWISE_ONLY") };
        unsafe { std::env::set_var("SQUINTLY_P_SINGLE", "1.7") };
        assert_eq!(SamplerConfig::from_env().p_single, 1.0, "clamped to [0,1]");
        unsafe { std::env::set_var("SQUINTLY_P_SINGLE", "nonsense") };
        assert_eq!(
            SamplerConfig::from_env().p_single,
            SamplerConfig::default().p_single,
            "unparseable keeps the default"
        );
        unsafe { std::env::remove_var("SQUINTLY_P_SINGLE") };
    }
}
