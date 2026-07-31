//! The non-photo study, checked against the shape of the real corpus.
//!
//! The unit tests in `content_class` use a synthetic manifest. This one asserts
//! the classification against the corpus values the live deployment actually
//! serves, so an upstream rename (the `imazen26-` prefix changing, a stratum
//! renumbered) surfaces here rather than as photographs appearing in a study
//! whose name says they cannot.

use squintly::content_class::{ContentClass, ContentFilter, classify};

/// Exactly what `GET /api/manifest` returns today — measured 2026-07-31 against
/// `codec-corpus/squintly/demo-corpus/imazen26-v2`: 21 strata, 4 sources each.
const LIVE_CORPORA: &[&str] = &[
    "imazen26-1000-lilith-photos-general",
    "imazen26-1200-lilith-interiors",
    "imazen26-1400-lilith-nature",
    "imazen26-1600-lilith-food",
    "imazen26-2000-unsplash-people",
    "imazen26-2200-unsplash-renders",
    "imazen26-2400-unsplash-textures",
    "imazen26-3000-art-institute-of-chicago-photos",
    "imazen26-3300-met-museum-photos",
    "imazen26-5000-national-park-service-brochures",
    "imazen26-5200-epa-climate-impact-2021-report",
    "imazen26-5300-noaa-hurricane-documents",
    "imazen26-6000-lilith-scans-public-patents",
    "imazen26-6600-ia-scans-manuscript-illustrations",
    "imazen26-6800-ia-scans-manuscript-text",
    "imazen26-7000-lilith-plots",
    "imazen26-8000-lilith-mobile-screenshots",
    "imazen26-8100-lilith-web-screenshots",
    "imazen26-9000-lilith-ai-clipart",
    "imazen26-9094-lilith-ai-illustrations",
    "imazen26-9226-lilith-ai-products",
];

#[test]
fn every_live_corpus_value_is_classifiable() {
    let unknown: Vec<&str> = LIVE_CORPORA
        .iter()
        .copied()
        .filter(|c| classify(Some(c)) == ContentClass::Unknown)
        .collect();
    assert!(
        unknown.is_empty(),
        "these live corpus values classify as Unknown, so a content-restricted \
         study silently drops them: {unknown:?}"
    );
}

#[test]
fn the_live_corpus_splits_the_way_the_builder_says() {
    let photo: Vec<&str> = LIVE_CORPORA
        .iter()
        .copied()
        .filter(|c| classify(Some(c)) == ContentClass::Photo)
        .collect();
    let non_photo: Vec<&str> = LIVE_CORPORA
        .iter()
        .copied()
        .filter(|c| classify(Some(c)) == ContentClass::NonPhoto)
        .collect();

    // 8 photographic strata out of 21 is what made roughly 38% of the study's
    // trials photographs before the filter existed.
    assert_eq!(photo.len(), 8, "photo strata: {photo:?}");
    assert_eq!(non_photo.len(), 13, "non-photo strata: {non_photo:?}");

    // Spot-check the two whose names do NOT give the answer away — those are
    // where a misclassification would be least visible on review.
    assert_eq!(
        classify(Some("imazen26-2200-unsplash-renders")),
        ContentClass::NonPhoto,
        "renders are synthetic despite sitting in the unsplash group"
    );
    assert_eq!(
        classify(Some("imazen26-2400-unsplash-textures")),
        ContentClass::Photo,
        "photographed textures are still photographs"
    );
}

#[test]
fn a_non_photo_study_accepts_only_the_non_photo_half() {
    let accepted = LIVE_CORPORA
        .iter()
        .filter(|c| ContentFilter::NonPhotoOnly.accepts(classify(Some(c))))
        .count();
    assert_eq!(accepted, 13);
    // And the pool has to be big enough to actually run imazen/squintly#4 on.
    assert!(
        accepted * 4 >= 40,
        "only {} sources would be eligible",
        accepted * 4
    );
}
