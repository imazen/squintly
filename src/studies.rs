//! Named studies, selectable at runtime.
//!
//! One deployment hosts several studies at once. They differ in what they
//! *measure*, so they differ in the trial stream they need — and mixing their
//! data would pool incompatible scales:
//!
//! * the pre-registered v0.2 crowd study wants the interleaved Type S / Type P
//!   mix from `docs/STUDY.md` §4.2 (single-stimulus staircases *and* pairwise);
//! * a rank-agreement study — validating SSIMULACRA2 as the non-photo oracle,
//!   imazen/squintly#4 — is a SROCC test on forced choice, where an ACR rating
//!   is a different quantity entirely.
//!
//! So the sampler config belongs to the study, not to the process. An observer
//! picks a study by name; the choice is stored on the session, and every trial
//! and response inherits it. That is also what lets the exports separate them:
//! without `study_id` on the row, two studies' responses are indistinguishable
//! after the fact.
//!
//! Studies are compiled in rather than configured, for the same reason the
//! license registry is: they are part of the pre-registration, and a typo in an
//! env var should not be able to invent one.

use serde::Serialize;

use crate::content_class::ContentFilter;
use crate::sampling::{PairingRule, SamplerConfig};

#[derive(Debug, Clone, Serialize)]
pub struct Study {
    /// Stable id. Stored on `sessions.study_id` and emitted in every export.
    pub id: &'static str,
    /// Shown in the picker.
    pub label: &'static str,
    /// One line explaining what the observer is contributing to.
    pub summary: &'static str,
    /// What the trial stream looks like, in plain words, so the picker can say
    /// so without the reader knowing what `p_single` is.
    pub trial_style: &'static str,
    /// Hidden from the public picker. Still selectable by id — for operator or
    /// single-observer runs that shouldn't be offered to drive-by visitors.
    pub unlisted: bool,
    /// Probability of re-serving a pair this observer already answered.
    ///
    /// **The control that makes the result interpretable.** Human-vs-ssim2
    /// SROCC means nothing on its own: if an observer agrees with *themselves*
    /// only 80% of the time on repeated pairs, ssim2 cannot exceed roughly
    /// that, and "ssim2 scored 0.7" reads completely differently against a
    /// ceiling of 0.95 than against one of 0.72. Repeats measure the ceiling
    /// directly, from the same observers, on the same content, in the same
    /// session — which is the only comparison that licenses a conclusion about
    /// the metric rather than about the data collection.
    pub p_repeat: f32,
    /// Whether an `excluded` participant disposition is *acted on* by default.
    ///
    /// The screens always run and are always recorded (see `exclusion.rs`);
    /// this only decides whether consumers drop the flagged data. It belongs to
    /// the study because the right answer depends on who is rating: an
    /// un-gated crowd wants the sieve, a handful of experts has no peer
    /// distribution to be an outlier against. `SQUINTLY_EXCLUSION` overrides.
    pub exclusion_default: bool,
    #[serde(skip)]
    pub sampler: SamplerConfig,
}

/// The default when a client doesn't name one. Overridable with
/// `SQUINTLY_DEFAULT_STUDY`.
///
/// `ssim2-nonphoto` while imazen/squintly#4 collects: validating SSIMULACRA2 as
/// the non-photo oracle is the live priority, and a judgment spent on a
/// photograph is one not spent on it. In code rather than an env var so the
/// intent travels with the repo and is covered by
/// `the_resolved_default_study_is_listed` — a default nobody can reach from the
/// picker is a configuration that only fails in front of a participant.
pub const DEFAULT_STUDY_ID: &str = "ssim2-nonphoto";

pub const STUDIES: &[Study] = &[
    Study {
        id: "main",
        label: "Web image quality (main study)",
        summary: "Which compression artefacts do people actually notice on real phones? \
                  Trains zensim, an open-source perceptual quality metric.",
        trial_style: "A mix of single-image ratings and A/B comparisons.",
        // Single-stimulus staircases already re-measure the same conditions.
        p_repeat: 0.0,
        // Listed, but not the default.
        //
        // Unlisting it entirely (to force the non-photo focus) removed the
        // picker, and with it any way to move between projects at all. The
        // focus is carried by DEFAULT_STUDY_ID plus the content filter — a
        // default nobody chose away from is enough, and an operator who needs
        // photographic work should not have to edit an env var to get it.
        unlisted: false,
        // Anonymous, un-gated crowd. ch3-5 §4.4 calls correlation-to-peer-mean
        // "your first sieve" precisely for this regime; §4.3.2 notes the
        // screens barely move a *pre-screened* pipeline, which this is not.
        exclusion_default: true,
        sampler: SamplerConfig {
            p_single: 0.65,
            p_honeypot: 0.083,
            p_anchor: 0.30,
            // The crowd study wants the whole corpus; content coverage across
            // photo and non-photo is part of what it measures.
            content: ContentFilter::Any,
            pairing: PairingRule::AdjacentQuality,
            // Already controlled by single-stimulus honeypots and anchors.
            p_golden_pair: 0.0,
            pairwise_only: false,
        },
    },
    Study {
        id: "ssim2-nonphoto",
        label: "Non-photo oracle check (A/B only)",
        summary: "Does SSIMULACRA2 rank non-photo content — screenshots, documents, \
                  line-art, charts — the way a human does? Every trial is a forced choice, \
                  and every image is non-photographic.",
        trial_style: "A/B comparisons only. No star ratings.",
        // ~1 trial in 12 re-serves a pair already answered, giving the
        // intra-observer agreement ceiling this study's headline number has to
        // be read against.
        p_repeat: 0.08,
        unlisted: false,
        // A rank-agreement check run by a few careful observers. ch3-5 §4.6:
        // below ~15 subjects the modelling is under-identified, and with few
        // peers per stimulus the reference distribution the BT.500 band needs
        // is noise. Screening still runs and is recorded; it just isn't acted
        // on unless someone asks for it.
        exclusion_default: false,
        sampler: SamplerConfig {
            // Forced choice only. `p_single: 0.0` alone would NOT be enough —
            // see SamplerConfig::pairwise_only for why (fallback to single, and
            // single-stimulus honeypots/anchors injected ahead of the draw).
            p_single: 0.0,
            p_honeypot: 0.0,
            p_anchor: 0.0,
            // The name is a claim about the data. Without this the study drew
            // from all 21 canonical strata, 8 of them photographic, so ~38% of
            // its trials were photos — valid judgements filed under a label
            // that says they are about non-photo content.
            content: ContentFilter::NonPhotoOnly,
            pairing: PairingRule::AdjacentQuality,
            // ~1 trial in 12, matching the single-stimulus honeypot rate this
            // study cannot use. Without it nothing here distinguishes a careful
            // observer from a careless one — the honeypot and anchor rates are
            // both zero, because both build single-stimulus trials.
            p_golden_pair: 0.083,
            pairwise_only: true,
        },
    },
    Study {
        id: "zensr-dejpeg",
        label: "JPEG artifact removal (zensr)",
        summary: "Does removing JPEG artifacts actually get closer to the original, \
                  or only score better? Each pair is one JPEG against zensr's restored \
                  version of that exact file.",
        trial_style: "A/B comparisons only. No star ratings.",
        p_repeat: 0.08,
        // Few, careful observers rather than an un-gated crowd, same as the
        // other forced-choice study: with few peers per stimulus the reference
        // distribution the screens need is noise (zenpapers ch3-5 §4.6).
        exclusion_default: false,
        // UNLISTED UNTIL THE CORPUS HAS RESTORATIONS.
        //
        // `RestorationVsBaseline` needs encodings whose codec starts with
        // `zensr-`, produced by running `zensr-zenjpeg::restore_jpeg` over the
        // corpus's JPEG rungs. None exist yet — the dejpeg weights are not in
        // the zensr tree, on /mnt/v, or in any R2 bucket (checked 2026-08-01).
        // Listing it now would put visitors on a study that can only 409.
        unlisted: true,
        sampler: SamplerConfig {
            p_single: 0.0,
            p_honeypot: 0.0,
            p_anchor: 0.0,
            // Artifact removal matters on photographs and on graphics alike,
            // and zensr routes them differently (`dejpeg9_gfxycc` exists for
            // graphics), so both belong in the pool. Narrow to
            // `NonPhotoOnly` if the graphics route is what needs answering.
            content: ContentFilter::Any,
            // The whole point: a restoration judged against its own input at
            // the same quality, not against a different compression level.
            pairing: PairingRule::RestorationVsBaseline {
                restored_prefix: "zensr",
            },
            // Same attention check. A golden here falls back to a restoration
            // pair when the source's ladder is too narrow to be unambiguous.
            p_golden_pair: 0.083,
            // Forced choice. "Closer to the original" is the question that
            // matters and it is not the same as "looks better": artifact
            // removal can invent plausible detail that was never there, which
            // reads as an improvement on a preference test and as a fidelity
            // failure on a reference one.
            pairwise_only: true,
        },
    },
];

pub fn by_id(id: &str) -> Option<&'static Study> {
    STUDIES.iter().find(|s| s.id == id)
}

/// Studies offered in the picker.
pub fn listed() -> Vec<&'static Study> {
    STUDIES.iter().filter(|s| !s.unlisted).collect()
}

/// Resolve the deployment's default study.
///
/// `SQUINTLY_PAIRWISE_ONLY=1` is kept as an alias for selecting the
/// forced-choice study: it shipped as the documented way to run the #4
/// validation, and DEPLOY.md plus a comment on that issue both reference it.
/// Studies are the real mechanism now; this only picks the default.
pub fn default_study() -> &'static Study {
    if let Ok(id) = std::env::var("SQUINTLY_DEFAULT_STUDY") {
        let id = id.trim();
        match by_id(id) {
            Some(s) => return s,
            None => tracing::warn!(
                requested = id,
                known = ?STUDIES.iter().map(|s| s.id).collect::<Vec<_>>(),
                "SQUINTLY_DEFAULT_STUDY names no known study; falling back"
            ),
        }
    }
    if std::env::var("SQUINTLY_PAIRWISE_ONLY")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        if let Some(s) = by_id("ssim2-nonphoto") {
            return s;
        }
    }
    by_id(DEFAULT_STUDY_ID).expect("DEFAULT_STUDY_ID must exist in STUDIES")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever a deployment defaults to must be offerable, or a visitor who
    /// opens the picker cannot get back to it.
    #[test]
    fn the_resolved_default_study_is_listed() {
        assert!(
            !default_study().unlisted,
            "the resolved default study ({}) is hidden from the picker",
            default_study().id
        );
    }

    /// The live priority is imazen/squintly#4, so a visitor who names no study
    /// lands on the non-photo validation rather than on general photo work.
    /// If this is deliberately reverted, delete the test with it — do not
    /// "fix" it by editing the expected id while `main` stays unlisted.
    #[test]
    fn the_compiled_default_is_the_non_photo_study() {
        assert_eq!(DEFAULT_STUDY_ID, "ssim2-nonphoto");
        let d = by_id(DEFAULT_STUDY_ID).expect("default must exist");
        assert_eq!(d.sampler.content, ContentFilter::NonPhotoOnly);
    }

    /// A study that cannot serve a trial must not be offered. `zensr-dejpeg`
    /// needs restored encodings that do not exist yet, so listing it would put
    /// visitors on a study that can only 409.
    #[test]
    fn a_study_needing_a_corpus_it_lacks_is_not_listed() {
        let z = by_id("zensr-dejpeg").expect("study must exist");
        assert!(
            z.unlisted,
            "zensr-dejpeg has no restored encodings in the corpus yet; keep it unlisted \
             until it can actually serve a trial"
        );
        assert!(matches!(
            z.sampler.pairing,
            crate::sampling::PairingRule::RestorationVsBaseline { .. }
        ));
    }

    /// Switching between projects has to be possible. Unlisting everything but
    /// one study hid the picker entirely, which removed the only way to move
    /// between them.
    #[test]
    fn more_than_one_study_is_offered_so_the_picker_exists() {
        assert!(
            listed().len() >= 2,
            "only {} study listed — the picker hides itself and nothing can be switched to",
            listed().len()
        );
    }

    /// At least one study has to be reachable by a drive-by visitor.
    #[test]
    fn something_is_always_offered() {
        assert!(!listed().is_empty(), "every study is unlisted");
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<&str> = STUDIES.iter().map(|s| s.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate study id");
    }

    /// The forced-choice study must actually be forced-choice. Setting
    /// `p_single: 0.0` is not sufficient on its own — the sampler falls back to
    /// a single when no non-trivial pair exists, and honeypots/anchors are
    /// themselves single-stimulus.
    /// The study's name is a claim about its data. It drew from the whole
    /// corpus — photographic strata included — while calling itself
    /// "non-photo", which produces valid judgements filed under the wrong
    /// question.
    #[test]
    fn the_non_photo_study_actually_restricts_content() {
        let s = by_id("ssim2-nonphoto").expect("study must exist");
        assert_eq!(
            s.sampler.content,
            ContentFilter::NonPhotoOnly,
            "a study named nonphoto must filter content, not just the trial mix"
        );
    }

    /// Any study whose id or label claims a content type must back it with a
    /// filter. Catches the next one added by copy-paste.
    #[test]
    fn studies_claiming_a_content_type_restrict_it() {
        for st in STUDIES {
            let claims_nonphoto = st.id.contains("nonphoto")
                || st.label.to_lowercase().contains("non-photo")
                || st.summary.to_lowercase().contains("non-photo");
            if claims_nonphoto {
                assert_eq!(
                    st.sampler.content,
                    ContentFilter::NonPhotoOnly,
                    "study {} claims non-photo content but does not filter for it",
                    st.id
                );
            }
        }
    }

    #[test]
    fn rank_agreement_study_is_strictly_pairwise() {
        let s = by_id("ssim2-nonphoto").expect("study must exist");
        assert!(
            s.sampler.pairwise_only,
            "must set pairwise_only, not just p_single"
        );
        assert_eq!(s.sampler.p_single, 0.0);
        assert_eq!(s.sampler.p_honeypot, 0.0, "honeypots are single-stimulus");
        assert_eq!(s.sampler.p_anchor, 0.0, "anchors are single-stimulus");
    }

    #[test]
    fn unknown_default_study_env_falls_back_rather_than_panicking() {
        // SAFETY: one test owning this var; cargo runs tests in parallel threads.
        unsafe { std::env::set_var("SQUINTLY_DEFAULT_STUDY", "no-such-study") };
        assert_eq!(default_study().id, DEFAULT_STUDY_ID);
        unsafe { std::env::set_var("SQUINTLY_DEFAULT_STUDY", "ssim2-nonphoto") };
        assert_eq!(default_study().id, "ssim2-nonphoto");
        unsafe { std::env::remove_var("SQUINTLY_DEFAULT_STUDY") };
    }
}
