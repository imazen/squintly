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
use crate::sampling::SamplerConfig;

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
pub const DEFAULT_STUDY_ID: &str = "main";

pub const STUDIES: &[Study] = &[
    Study {
        id: "main",
        label: "Web image quality (main study)",
        summary: "Which compression artefacts do people actually notice on real phones? \
                  Trains zensim, an open-source perceptual quality metric.",
        trial_style: "A mix of single-image ratings and A/B comparisons.",
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

    #[test]
    fn default_study_exists_and_is_listed() {
        let d = by_id(DEFAULT_STUDY_ID).expect("default study must exist");
        assert!(!d.unlisted, "the default study must be offerable");
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
