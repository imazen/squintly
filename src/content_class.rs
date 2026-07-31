//! Whether a source is photographic, and which sources a study will accept.
//!
//! # Why this exists
//!
//! `ssim2-nonphoto` (imazen/squintly#4) asks whether SSIMULACRA2 ranks
//! *non-photo* content — screenshots, documents, line-art, charts, renders —
//! the way a human does. Until this module, the study constrained only the
//! trial *mix* (forced choice) and not the *content*, so it sampled the whole
//! corpus: of the 21 canonical strata, 8 are photographic, and the live corpus
//! carries 4 sources each, so roughly 38% of its trials were photos. Every one
//! of those is a valid pairwise judgement filed under a label that says it is
//! about non-photo content — the data does not look wrong, it looks like an
//! answer to a question nobody asked.
//!
//! # Where the classification comes from
//!
//! `coefficient::SourceMeta` carries no content type — only `corpus`, which for
//! the canonical corpus is the stratum name. The stratum *is* the
//! classification: `8000-lilith-mobile-screenshots` is not photographic and
//! `1400-lilith-nature` is. `scripts/build_demo_corpus.py::R2_STRATA` already
//! holds exactly this boolean per stratum; this is the same table on the Rust
//! side, and `strata_agree_with_the_corpus_builder` is the guard against the
//! two drifting apart.
//!
//! # Unknown strata are not eligible
//!
//! An unrecognised corpus resolves to [`ContentClass::Unknown`], which a
//! content-restricted study **refuses**. Defaulting the other way would mean a
//! stratum added to the builder but not here quietly enters the non-photo pool
//! — the same silent-mislabelling failure this module exists to fix, just
//! moved. Failing closed makes it a visible shortage of trials instead.

/// What kind of picture a source is, as far as the study cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentClass {
    /// A photograph: continuous tone, sensor noise, no hard synthetic edges.
    Photo,
    /// Screenshots, documents, scans, plots, line-art, clipart, 3-D renders —
    /// anything whose structure is synthetic. The regime where metrics tuned on
    /// photographic content are least trusted, which is the point of #4.
    NonPhoto,
    /// Not in the registry. Never silently admitted to a restricted study.
    Unknown,
}

/// Which sources a study will draw from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentFilter {
    /// No restriction — the general-purpose studies want the whole corpus.
    Any,
    /// Only sources whose stratum is registered non-photographic.
    NonPhotoOnly,
    /// Only sources whose stratum is registered photographic.
    PhotoOnly,
}

impl ContentFilter {
    pub fn accepts(&self, class: ContentClass) -> bool {
        match self {
            ContentFilter::Any => true,
            ContentFilter::NonPhotoOnly => class == ContentClass::NonPhoto,
            ContentFilter::PhotoOnly => class == ContentClass::Photo,
        }
    }

    /// Human-readable, for the 409 an operator sees when a filter empties the
    /// pool. A bare "no trials available" on a content-restricted study sends
    /// people looking at the sampler instead of at the corpus.
    pub fn describe(&self) -> &'static str {
        match self {
            ContentFilter::Any => "any content",
            ContentFilter::NonPhotoOnly => "non-photographic sources only",
            ContentFilter::PhotoOnly => "photographic sources only",
        }
    }
}

/// Stratum suffix → is it photographic.
///
/// Mirrors `scripts/build_demo_corpus.py::R2_STRATA` (the second tuple field).
/// Matched as a substring of the lowercased corpus because the live manifest
/// prefixes strata with the corpus name (`imazen26-8000-lilith-mobile-
/// screenshots`), exactly as `licensing::lookup` handles it.
const STRATA: &[(&str, ContentClass)] = &[
    ("1000-lilith-photos-general", ContentClass::Photo),
    ("1200-lilith-interiors", ContentClass::Photo),
    ("1400-lilith-nature", ContentClass::Photo),
    ("1600-lilith-food", ContentClass::Photo),
    ("2000-unsplash-people", ContentClass::Photo),
    // Renders are synthetic: hard edges, flat gradients, no sensor noise.
    ("2200-unsplash-renders", ContentClass::NonPhoto),
    ("2400-unsplash-textures", ContentClass::Photo),
    ("3000-art-institute-of-chicago-photos", ContentClass::Photo),
    ("3300-met-museum-photos", ContentClass::Photo),
    (
        "5000-national-park-service-brochures",
        ContentClass::NonPhoto,
    ),
    (
        "5200-epa-climate-impact-2021-report",
        ContentClass::NonPhoto,
    ),
    ("5300-noaa-hurricane-documents", ContentClass::NonPhoto),
    ("6000-lilith-scans-public-patents", ContentClass::NonPhoto),
    (
        "6600-ia-scans-manuscript-illustrations",
        ContentClass::NonPhoto,
    ),
    ("6800-ia-scans-manuscript-text", ContentClass::NonPhoto),
    ("7000-lilith-plots", ContentClass::NonPhoto),
    ("8000-lilith-mobile-screenshots", ContentClass::NonPhoto),
    ("8100-lilith-web-screenshots", ContentClass::NonPhoto),
    ("9000-lilith-ai-clipart", ContentClass::NonPhoto),
    ("9094-lilith-ai-illustrations", ContentClass::NonPhoto),
    ("9226-lilith-ai-products", ContentClass::NonPhoto),
];

/// Classify a corpus/stratum string. `None` (a source with no corpus at all)
/// is [`ContentClass::Unknown`], not a default.
pub fn classify(corpus: Option<&str>) -> ContentClass {
    let Some(corpus) = corpus else {
        return ContentClass::Unknown;
    };
    let lower = corpus.to_ascii_lowercase();
    for (stratum, class) in STRATA {
        if lower.contains(stratum) {
            return *class;
        }
    }
    ContentClass::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The live manifest prefixes the stratum with the corpus name, so matching
    /// has to survive that. This is the exact shape `GET /api/manifest` returns.
    #[test]
    fn classifies_live_manifest_corpus_names() {
        assert_eq!(
            classify(Some("imazen26-8000-lilith-mobile-screenshots")),
            ContentClass::NonPhoto
        );
        assert_eq!(
            classify(Some("imazen26-1400-lilith-nature")),
            ContentClass::Photo
        );
        // Bare stratum names too, for stores that don't prefix.
        assert_eq!(classify(Some("7000-lilith-plots")), ContentClass::NonPhoto);
        assert_eq!(
            classify(Some("IMAZEN26-1600-LILITH-FOOD")),
            ContentClass::Photo
        );
    }

    /// The failure this module exists to prevent, in miniature: an unregistered
    /// stratum must not slip into a non-photo study.
    #[test]
    fn unknown_corpora_are_not_admitted_to_a_restricted_study() {
        for c in [None, Some("some-new-stratum"), Some(""), Some("kodak")] {
            assert_eq!(classify(c), ContentClass::Unknown, "{c:?}");
            assert!(
                !ContentFilter::NonPhotoOnly.accepts(classify(c)),
                "{c:?} must not be admitted"
            );
            assert!(!ContentFilter::PhotoOnly.accepts(classify(c)));
            // ...but an unrestricted study still takes it.
            assert!(ContentFilter::Any.accepts(classify(c)));
        }
    }

    #[test]
    fn filters_accept_what_they_say() {
        assert!(ContentFilter::NonPhotoOnly.accepts(ContentClass::NonPhoto));
        assert!(!ContentFilter::NonPhotoOnly.accepts(ContentClass::Photo));
        assert!(ContentFilter::PhotoOnly.accepts(ContentClass::Photo));
        assert!(!ContentFilter::PhotoOnly.accepts(ContentClass::NonPhoto));
        assert!(ContentFilter::Any.accepts(ContentClass::Photo));
        assert!(ContentFilter::Any.accepts(ContentClass::NonPhoto));
    }

    /// Drift guard, the counterpart to
    /// `licensing::every_v3_stratum_has_a_real_policy`.
    ///
    /// A stratum present in the corpus builder but missing here classifies as
    /// `Unknown` and silently vanishes from the non-photo pool — a study that
    /// quietly narrows is as wrong as one that quietly widens. The expected
    /// booleans are transcribed from `R2_STRATA`'s second tuple field.
    #[test]
    fn strata_agree_with_the_corpus_builder() {
        // (stratum, is_photo) — keep in sync with R2_STRATA.
        let expected: &[(&str, bool)] = &[
            ("1000-lilith-photos-general", true),
            ("1200-lilith-interiors", true),
            ("1400-lilith-nature", true),
            ("1600-lilith-food", true),
            ("2000-unsplash-people", true),
            ("2200-unsplash-renders", false),
            ("2400-unsplash-textures", true),
            ("3000-art-institute-of-chicago-photos", true),
            ("3300-met-museum-photos", true),
            ("5000-national-park-service-brochures", false),
            ("5200-epa-climate-impact-2021-report", false),
            ("5300-noaa-hurricane-documents", false),
            ("6000-lilith-scans-public-patents", false),
            ("6600-ia-scans-manuscript-illustrations", false),
            ("6800-ia-scans-manuscript-text", false),
            ("7000-lilith-plots", false),
            ("8000-lilith-mobile-screenshots", false),
            ("8100-lilith-web-screenshots", false),
            ("9000-lilith-ai-clipart", false),
            ("9094-lilith-ai-illustrations", false),
            ("9226-lilith-ai-products", false),
        ];
        assert_eq!(
            expected.len(),
            STRATA.len(),
            "STRATA and the corpus builder disagree on how many strata exist"
        );
        for (stratum, is_photo) in expected {
            let got = classify(Some(&format!("imazen26-{stratum}")));
            let want = if *is_photo {
                ContentClass::Photo
            } else {
                ContentClass::NonPhoto
            };
            assert_eq!(
                got, want,
                "stratum {stratum}: builder says is_photo={is_photo}, registry says {got:?}"
            );
        }
    }

    /// The non-photo pool has to be big enough to run a study on. If a future
    /// corpus change tipped most strata into `Photo`, #4 would be starved and
    /// the only symptom would be 409s.
    #[test]
    fn the_non_photo_pool_is_the_larger_half() {
        let non_photo = STRATA
            .iter()
            .filter(|(_, c)| *c == ContentClass::NonPhoto)
            .count();
        assert!(
            non_photo >= STRATA.len() / 2,
            "only {non_photo} of {} strata are non-photo",
            STRATA.len()
        );
    }
}
