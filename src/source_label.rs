//! What to call the picture on screen.
//!
//! The trial header has one short line above a stimulus, and on a phone it is
//! the narrowest line in the app. It needs to identify the image well enough
//! that someone reporting "this one has a green band" can be understood — which
//! the corpus name alone never did, because a corpus holds dozens of pictures.
//!
//! The filename does identify it. But corpus and filename overlap heavily by
//! construction: `imazen26-6600-ia-scans-manuscript-illustrations` beside
//! `6605_scans-illustrations_haeckel-pitcher-plants_plate0062_...` repeats the
//! id prefix and "scans illustrations" twice in a row, which is exactly the
//! sort of duplication that pushes a header onto a second line.
//!
//! So the corpus contributes only the part of itself the filename does NOT
//! already say.

/// Split a name into comparable lowercase word tokens.
///
/// `-` and `_` are both separators here: the corpus uses hyphens and the
/// filename uses underscores for the same words, so a comparison that respects
/// the difference would find no overlap at all and defeat the whole point.
fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

/// Trim a source filename down to something readable in a header.
///
/// Drops the directory, the extension, the size-bucket suffix and the trailing
/// `WxH` dimensions — the dimensions are already reported separately, and at
/// header width they crowd out the descriptive part, which is the only bit a
/// person can act on.
pub fn short_filename(filename: &str) -> String {
    let base = filename.rsplit('/').next().unwrap_or(filename);
    let mut out = base.to_string();
    // The decorations interleave — `..._4988x7412.sdr__XL.png` is dimensions,
    // then an extension, then a size rung, then another extension — so one pass
    // in a fixed order cannot reach them all. Loop until nothing more comes off.
    loop {
        let before = out.len();

        // Trailing extension segment. Bounded and alphanumeric so a dotted part
        // of the name itself (`v1.2-draft`) is not eaten.
        if let Some((head, tail)) = out.rsplit_once('.') {
            if !tail.is_empty()
                && tail.len() <= 5
                && tail.chars().all(|c| c.is_ascii_alphanumeric())
            {
                out = head.to_string();
            }
        }
        // Size rung, e.g. `__XL`.
        if let Some((head, tail)) = out.rsplit_once("__") {
            if !tail.is_empty()
                && tail.len() <= 3
                && tail.chars().all(|c| c.is_ascii_alphanumeric())
            {
                out = head.to_string();
            }
        }
        // Trailing `_WxH`: reported separately, and at header width it crowds
        // out the descriptive part, which is the only bit a person can act on.
        if let Some((head, tail)) = out.rsplit_once('_') {
            let dims = tail.split_once('x').is_some_and(|(w, h)| {
                !w.is_empty()
                    && !h.is_empty()
                    && w.chars().all(|c| c.is_ascii_digit())
                    && h.chars().all(|c| c.is_ascii_digit())
            });
            if dims {
                out = head.to_string();
            }
        }

        if out.len() == before {
            return out;
        }
    }
}

/// The part of the corpus name the filename does not already carry.
///
/// Returns `None` when the filename says everything the corpus does — printing
/// it again would spend the header's scarcest resource on a repetition.
///
/// The numeric stratum id is always dropped: it leads the filename anyway, and
/// a bare number is not a thing a reader can use.
pub fn corpus_remainder(corpus: &str, filename: Option<&str>) -> Option<String> {
    let file_tokens = filename.map(tokens).unwrap_or_default();
    let mut kept: Vec<String> = Vec::new();
    for t in tokens(corpus) {
        // The dataset prefix and the stratum number identify the collection,
        // not the picture.
        if t == "imazen26" || t.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if file_tokens.iter().any(|f| f == &t) {
            continue;
        }
        if kept.contains(&t) {
            continue;
        }
        kept.push(t);
    }
    (!kept.is_empty()).then(|| kept.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real shape this exists for: corpus and filename repeat each other.
    #[test]
    fn the_corpus_contributes_only_what_the_filename_omits() {
        assert_eq!(
            corpus_remainder(
                "imazen26-6600-ia-scans-manuscript-illustrations",
                Some("6605_scans-illustrations_haeckel-pitcher-plants_plate0062_4988x7412.sdr__XL.png"),
            )
            .as_deref(),
            Some("ia manuscript"),
        );
        assert_eq!(
            corpus_remainder(
                "imazen26-5000-national-park-service-brochures",
                Some("5016_nps_grsm-grsm-area-map_color_p01_10182x5460.sdr__XL.png"),
            )
            .as_deref(),
            Some("national park service brochures"),
        );
    }

    /// Hyphen vs underscore must not defeat the overlap check — the corpus uses
    /// one and the filename the other for the very same words.
    #[test]
    fn separators_do_not_hide_an_overlap() {
        assert_eq!(
            corpus_remainder(
                "imazen26-1000-lilith-photos-general",
                Some("1000_general_red-car_4032x3024.sdr.png")
            )
            .as_deref(),
            Some("lilith photos"),
        );
    }

    #[test]
    fn nothing_left_to_say_yields_none() {
        assert_eq!(
            corpus_remainder(
                "imazen26-7000-lilith-plots",
                Some("7001_lilith_plots_800x600.png")
            ),
            None
        );
        // The stratum number alone is never worth a line.
        assert_eq!(corpus_remainder("imazen26-7000", Some("7001_x.png")), None);
    }

    #[test]
    fn a_missing_filename_keeps_the_whole_corpus() {
        assert_eq!(
            corpus_remainder("imazen26-2000-unsplash-people", None).as_deref(),
            Some("unsplash people"),
        );
    }

    #[test]
    fn the_filename_loses_its_extension_bucket_and_dimensions() {
        assert_eq!(
            short_filename(
                "6605_scans-illustrations_haeckel-pitcher-plants_plate0062_4988x7412.sdr__XL.png"
            ),
            "6605_scans-illustrations_haeckel-pitcher-plants_plate0062",
        );
        assert_eq!(
            short_filename(
                "1000_general_red-convertible-car_note9_20190514-100811_4032x3024.sdr.png"
            ),
            "1000_general_red-convertible-car_note9_20190514-100811",
        );
    }

    /// A name with none of the expected decoration must survive untouched — the
    /// trimming is opportunistic, not a parser that can reject input.
    #[test]
    fn an_undecorated_name_is_left_alone() {
        assert_eq!(
            short_filename("haeckel_0007_cephalopods"),
            "haeckel_0007_cephalopods"
        );
        assert_eq!(short_filename("a.png"), "a");
    }
}
