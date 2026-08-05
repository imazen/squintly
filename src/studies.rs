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
//! # Measuring a metric's efficacy on one content class *relative to* another
//!
//! "Is SSIMULACRA2 good at non-photo content?" has no answer on its own — a
//! correlation of 0.7 is only interpretable against something. Two comparisons
//! are available and only one of them is valid:
//!
//! * **Against a published photographic number** (CID22, KADID, TID). Invalid.
//!   Different observers, different UI, different pair selection, different
//!   protocol. Any gap could be the instrument rather than the content.
//! * **Against a photographic arm of *this* instrument.** Valid. Same
//!   observers, same screen, same forced-choice protocol, same sampler, same
//!   counterbalancing — differing only in the content class drawn.
//!
//! `ssim2-photo-control` is that arm: byte-for-byte the same `SamplerConfig` as
//! `ssim2-nonphoto` apart from `ContentFilter::PhotoOnly`. Keeping them
//! identical is not tidiness, it is the entire experimental control; a
//! difference in any other field would confound the comparison it exists to
//! make (guarded by `the_photo_arm_differs_only_in_content`).
//!
//! **And the comparison is of efficiencies, not raw correlations.** Humans may
//! simply be noisier on one class: if self-agreement on repeated pairs is 0.95
//! on photographs and 0.75 on non-photo, a lower ssim2 correlation on non-photo
//! could be entirely human noise and say nothing about the metric. `p_repeat`
//! measures that ceiling per class, so the statistic to compare is
//! `ρ / ceiling` — how much of the achievable agreement the metric captured —
//! not `ρ` itself.
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
    /// Two words, for the corner of the trial screen.
    ///
    /// The full `label` is a sentence — fine in a picker, useless in the one
    /// line of chrome above a stimulus on a phone. This is what an observer
    /// glances at to know which study they are in, so it has to survive being
    /// read sideways: two words, no parentheses, no jargon.
    pub short_name: &'static str,
    /// One line explaining what the observer is contributing to.
    pub summary: &'static str,
    /// What the trial stream looks like, in plain words, so the picker can say
    /// so without the reader knowing what `p_single` is.
    pub trial_style: &'static str,
    /// Hidden from the public picker. Still selectable by id — for operator or
    /// single-observer runs that shouldn't be offered to drive-by visitors.
    pub unlisted: bool,
    /// How many responses this study needs before its result means anything.
    ///
    /// Two numbers, not one, because they answer different questions.
    /// `min_viable_ratings` is the point below which the fit is not worth
    /// reporting at all — a rank correlation from a handful of pairs is noise
    /// with a decimal point on it. `ideal_ratings` is where the confidence
    /// interval is tight enough that the study can be called done and people
    /// can stop.
    ///
    /// Published so an observer can see what they are contributing toward, and
    /// so "is there enough data yet?" is answered by the study's own
    /// pre-registered number rather than by whoever is looking at the dashboard.
    pub min_viable_ratings: u32,
    pub ideal_ratings: u32,
    /// The one an observer gets without choosing.
    ///
    /// Exactly one study carries this; `default_study` asserts it. A default
    /// that is merely "whichever is listed first" moves the moment a study is
    /// added or reordered, and every session recorded under the old one becomes
    /// hard to interpret after the fact.
    pub is_default: bool,
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
        short_name: "Web quality",
        min_viable_ratings: 600,
        ideal_ratings: 3000,
        is_default: false,
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
        label: "Non-photo oracle check (A/B, with photo control)",
        short_name: "Non-photo check",
        min_viable_ratings: 900,
        ideal_ratings: 4500,
        is_default: true,
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
            // Mostly non-photo, with ~1 trial in 4 photographic as a
            // WITHIN-SESSION control. A separate photo study confounds content
            // with session — fatigue, lighting, screen and adaptation all
            // differ between sessions — so the comparison the control exists
            // to license would not be clean. See `ContentFilter::Mixed`.
            content: ContentFilter::Mixed {
                photo_fraction: 0.25,
            },
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
        id: "ssim2-photo-control",
        label: "Photo control arm (A/B only)",
        short_name: "Photo control",
        min_viable_ratings: 400,
        ideal_ratings: 2000,
        is_default: false,
        summary: "The same forced-choice comparison on photographs. Run alongside the \
                  non-photo study so its result has something to be measured against.",
        trial_style: "A/B comparisons only. No star ratings.",
        p_repeat: 0.08,
        exclusion_default: false,
        // UNLISTED: superseded by interleaving.
        //
        // Run as a separate study, this arm's data comes from different
        // sessions than the non-photo data it is meant to control for, so
        // content is confounded with session. `ssim2-nonphoto` now interleaves
        // a photographic minority instead, which is within-session and
        // within-observer by construction.
        //
        // Kept selectable by id: a dedicated photographic run is still a
        // legitimate thing to want, and retiring the id would break sessions
        // that reference it.
        unlisted: true,
        sampler: SamplerConfig {
            // IDENTICAL to `ssim2-nonphoto` in every respect except the content
            // filter. That is the whole design — see the note below.
            p_single: 0.0,
            p_honeypot: 0.0,
            p_anchor: 0.0,
            content: ContentFilter::PhotoOnly,
            pairing: PairingRule::AdjacentQuality,
            p_golden_pair: 0.083,
            pairwise_only: true,
        },
    },
    Study {
        id: "zensr-dejpeg",
        label: "JPEG artifact removal (zensr)",
        short_name: "Artifact removal",
        min_viable_ratings: 400,
        ideal_ratings: 2000,
        is_default: false,
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
        // Majority non-photo, with a declared photographic control minority.
        match d.sampler.content {
            ContentFilter::Mixed { photo_fraction } => assert!(photo_fraction < 0.5),
            other => panic!("expected a mixed draw, got {other:?}"),
        }
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

    /// The photo arm is a control, so it must differ from the study it controls
    /// for in EXACTLY one respect. A difference in any other field would
    /// confound the comparison it exists to make.
    #[test]
    fn the_photo_arm_differs_only_in_content() {
        let np = by_id("ssim2-nonphoto").expect("study");
        let ph = by_id("ssim2-photo-control").expect("study");
        assert_eq!(ph.sampler.content, ContentFilter::PhotoOnly);
        assert!(matches!(np.sampler.content, ContentFilter::Mixed { .. }));
        // Everything else identical.
        assert_eq!(ph.sampler.p_single, np.sampler.p_single);
        assert_eq!(ph.sampler.p_honeypot, np.sampler.p_honeypot);
        assert_eq!(ph.sampler.p_anchor, np.sampler.p_anchor);
        assert_eq!(ph.sampler.p_golden_pair, np.sampler.p_golden_pair);
        assert_eq!(ph.sampler.pairwise_only, np.sampler.pairwise_only);
        assert_eq!(ph.sampler.pairing, np.sampler.pairing);
        // The ceiling has to be measurable on both arms, or the efficiencies
        // cannot be compared at all.
        assert_eq!(ph.p_repeat, np.p_repeat);
        assert!(
            ph.p_repeat > 0.0,
            "no repeats means no ceiling means no comparison"
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
        // It must restrict content — the original bug was that it constrained
        // only the trial mix and drew from the whole corpus. A declared
        // minority of photographs as a within-session control is not that:
        // those trials are tagged `content_class` and are the control arm.
        match s.sampler.content {
            ContentFilter::Mixed { photo_fraction } => assert!(
                photo_fraction < 0.5,
                "a study named nonphoto must be majority non-photo, got {photo_fraction}"
            ),
            other => panic!("expected a mixed draw with a photo control, got {other:?}"),
        }
    }

    /// Any study whose id or label claims a content type must back it with a
    /// filter. Catches the next one added by copy-paste.
    #[test]
    fn studies_claiming_a_content_type_restrict_it() {
        // Checked against the id and the LABEL, not the summary. The summary is
        // prose and may legitimately mention another study — the photo control
        // arm's summary says "run alongside the non-photo study", which a naive
        // substring match read as a claim to *be* one.
        for st in STUDIES {
            let name = format!("{} {}", st.id, st.label).to_lowercase();
            let claims_nonphoto = name.contains("nonphoto") || name.contains("non-photo");
            if claims_nonphoto {
                // `Mixed` counts: it is majority non-photo, with a declared
                // photographic minority as a within-session control, and the
                // label says so.
                let ok = matches!(
                    st.sampler.content,
                    ContentFilter::NonPhotoOnly | ContentFilter::Mixed { .. }
                );
                assert!(
                    ok,
                    "study {} claims non-photo content but does not filter for it ({:?})",
                    st.id, st.sampler.content
                );
                if let ContentFilter::Mixed { photo_fraction } = st.sampler.content {
                    assert!(
                        photo_fraction < 0.5,
                        "study {} is named non-photo but draws {:.0}% photographs",
                        st.id,
                        photo_fraction * 100.0
                    );
                    assert!(
                        st.label.to_lowercase().contains("photo control"),
                        "study {} mixes in photographs; the label must say so",
                        st.id
                    );
                }
            } else if name.contains("photo") {
                assert_eq!(
                    st.sampler.content,
                    ContentFilter::PhotoOnly,
                    "study {} claims photographic content but does not filter for it",
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

#[cfg(test)]
mod default_flag_tests {
    use super::*;

    /// Exactly one study is the default, and it is the one `DEFAULT_STUDY_ID`
    /// names. Two sources of truth for "which study does someone get without
    /// choosing" is how sessions end up recorded under a study nobody meant to
    /// run — and neither the flag nor the constant announces the disagreement.
    #[test]
    fn exactly_one_study_is_the_default_and_it_matches_the_id() {
        let flagged: Vec<&str> = STUDIES
            .iter()
            .filter(|s| s.is_default)
            .map(|s| s.id)
            .collect();
        assert_eq!(
            flagged.len(),
            1,
            "expected exactly one is_default study, found {flagged:?}"
        );
        assert_eq!(flagged[0], DEFAULT_STUDY_ID);
    }

    /// The default has to be reachable without typing an id, or "default" means
    /// nothing to the person it exists for.
    #[test]
    fn the_default_is_listed() {
        let d = by_id(DEFAULT_STUDY_ID).expect("default study exists");
        assert!(!d.unlisted, "the default study must appear in the picker");
    }

    /// Two words, checked here as well as in the e2e suite so a new study
    /// cannot ship with a corner label that wraps or is empty.
    #[test]
    fn every_study_has_a_two_word_short_name() {
        for s in STUDIES {
            let words: Vec<&str> = s.short_name.split_whitespace().collect();
            assert!(
                !words.is_empty() && words.len() <= 2,
                "{}: short_name {:?} must be one or two words",
                s.id,
                s.short_name
            );
        }
    }
}

#[cfg(test)]
mod target_tests {
    use super::*;

    /// Both numbers must be present and ordered, or "how far along is this
    /// study?" has no answer and a progress bar would be drawing a lie.
    #[test]
    fn every_study_states_what_it_needs() {
        for s in STUDIES {
            assert!(
                s.min_viable_ratings > 0,
                "{}: min_viable_ratings must be set",
                s.id
            );
            assert!(
                s.ideal_ratings > s.min_viable_ratings,
                "{}: ideal ({}) must exceed minimum viable ({})",
                s.id,
                s.ideal_ratings,
                s.min_viable_ratings
            );
        }
    }

    /// The forced-choice study needs the most: a 2AFC answer carries less
    /// information than a 4-tier rating, and its repeat arm spends responses on
    /// measuring the observer rather than the metric.
    #[test]
    fn the_pairwise_study_asks_for_more_than_the_rating_study() {
        let pairwise = by_id("ssim2-nonphoto").expect("study exists");
        let mixed = by_id("main").expect("study exists");
        assert!(pairwise.min_viable_ratings > mixed.min_viable_ratings);
    }
}
