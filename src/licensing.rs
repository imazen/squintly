//! Per-corpus license registry.
//!
//! Squintly never claims to know the license of every individual image — that
//! information is generally not in corpus-builder's manifest. What we *do*
//! know is the *corpus* an image came from, and corpora carry policies. This
//! module surfaces the policy so the curator and rating UIs can show honest
//! attribution and downstream consumers (training pipelines) can filter by
//! license posture.
//!
//! When per-image attribution exists in the manifest (`license_id`,
//! `license_url`), the database row carries it; the registry below is the
//! fallback when the manifest only tells us the corpus.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct LicensePolicy {
    /// Stable id used in DB rows and TSV exports.
    pub id: &'static str,
    /// Human-readable label.
    pub label: &'static str,
    /// SPDX-style identifier or "mixed" / "research-only".
    pub spdx_or_status: &'static str,
    /// One-sentence policy text shown in UI badges.
    pub summary: &'static str,
    /// Permanent reference URL for full terms.
    pub terms_url: &'static str,
    /// Whether redistributing the original bytes is permitted.
    pub redistribute_bytes: bool,
    /// Whether commercial training is permitted.
    pub commercial_training: bool,
    /// Whether attribution is required when redistributing.
    pub attribution_required: bool,
}

/// Look up policy by corpus name (matches corpus-builder's `corpus`/`source_label`
/// columns). Returns `MIXED_RESEARCH` if unknown.
pub fn lookup(corpus: &str) -> &'static LicensePolicy {
    let lower = corpus.to_ascii_lowercase();
    for entry in REGISTRY {
        for prefix in entry.match_prefixes {
            if lower.starts_with(prefix) {
                return entry.policy;
            }
        }
    }
    &MIXED_RESEARCH
}

/// Get a policy by its stable id (used when DB rows persist `license_id`).
pub fn by_id(id: &str) -> &'static LicensePolicy {
    for p in ALL_POLICIES {
        if p.id == id {
            return p;
        }
    }
    &MIXED_RESEARCH
}

struct Entry {
    match_prefixes: &'static [&'static str],
    policy: &'static LicensePolicy,
}

pub const UNSPLASH: LicensePolicy = LicensePolicy {
    id: "unsplash",
    label: "Unsplash License",
    spdx_or_status: "Unsplash-License",
    summary: "Free to use. No attribution required, but credit appreciated. No selling unmodified.",
    terms_url: "https://unsplash.com/license",
    redistribute_bytes: true,
    commercial_training: true,
    attribution_required: false,
};

pub const WIKIMEDIA: LicensePolicy = LicensePolicy {
    id: "wikimedia-mixed",
    label: "Wikimedia Commons (mixed)",
    spdx_or_status: "mixed-CC",
    summary: "Mixed CC-BY / CC-BY-SA / CC0 — per-image attribution required when known.",
    terms_url: "https://commons.wikimedia.org/wiki/Commons:Reusing_content_outside_Wikimedia",
    redistribute_bytes: true,
    commercial_training: true,
    attribution_required: true,
};

pub const CC_INDEX: LicensePolicy = LicensePolicy {
    id: "common-crawl",
    label: "Common Crawl Index",
    spdx_or_status: "research-fair-use",
    summary: "Crawled web images. Research/fair-use only. Do not redistribute bytes.",
    terms_url: "https://commoncrawl.org/terms-of-use",
    redistribute_bytes: false,
    commercial_training: false,
    attribution_required: true,
};

pub const FLICKR_PHOTO: LicensePolicy = LicensePolicy {
    id: "flickr-mixed",
    label: "Flickr (mixed CC)",
    spdx_or_status: "mixed-CC",
    summary: "Per-photo CC license — see source URL for terms.",
    terms_url: "https://www.flickr.com/creativecommons/",
    redistribute_bytes: true,
    commercial_training: false,
    attribution_required: true,
};

pub const GITHUB_REPRO: LicensePolicy = LicensePolicy {
    id: "github-issues",
    label: "GitHub issue repros",
    spdx_or_status: "research-fair-use",
    summary: "Bug-report attachments scraped from public issues. Research only.",
    terms_url: "https://docs.github.com/en/site-policy/github-terms",
    redistribute_bytes: false,
    commercial_training: false,
    attribution_required: true,
};

pub const GENERATED_BUILT: LicensePolicy = LicensePolicy {
    id: "generated-built",
    label: "Built corpus (re-encoded)",
    spdx_or_status: "derived-mixed",
    summary: "Re-encoded variants of the source corpora. Inherits source license.",
    terms_url: "https://github.com/imazen/squintly/blob/main/docs/HANDOFF.md#19-reading-list",
    redistribute_bytes: false,
    commercial_training: false,
    attribution_required: true,
};

pub const MIXED_RESEARCH: LicensePolicy = LicensePolicy {
    id: "mixed-research",
    label: "Mixed (research only)",
    spdx_or_status: "research-fair-use",
    summary: "Mixed-provenance research corpus. Treat as fair-use; do not redistribute.",
    terms_url: "https://github.com/imazen/squintly/blob/main/CLAUDE.md",
    redistribute_bytes: false,
    commercial_training: false,
    attribution_required: true,
};

const ALL_POLICIES: &[&LicensePolicy] = &[
    &UNSPLASH,
    &WIKIMEDIA,
    &CC_INDEX,
    &FLICKR_PHOTO,
    &GITHUB_REPRO,
    &GENERATED_BUILT,
    &MIXED_RESEARCH,
];

const REGISTRY: &[Entry] = &[
    Entry {
        match_prefixes: &["unsplash"],
        policy: &UNSPLASH,
    },
    Entry {
        match_prefixes: &["wikimedia", "wide-gamut", "wikimedia-webshapes"],
        policy: &WIKIMEDIA,
    },
    Entry {
        match_prefixes: &["cc-index", "scraping/cc", "common-crawl"],
        policy: &CC_INDEX,
    },
    Entry {
        match_prefixes: &["flickr"],
        policy: &FLICKR_PHOTO,
    },
    Entry {
        match_prefixes: &["github-issues", "repro-images"],
        policy: &GITHUB_REPRO,
    },
    Entry {
        match_prefixes: &["corpus/", "built", "generated", "png-", "gif-", "apng"],
        policy: &GENERATED_BUILT,
    },
];

/// All known policies, for the welcome-screen credits panel.
pub fn all_policies() -> &'static [&'static LicensePolicy] {
    ALL_POLICIES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsplash_corpus_resolves() {
        let p = lookup("unsplash-webp");
        assert_eq!(p.id, "unsplash");
        assert!(p.redistribute_bytes);
    }

    #[test]
    fn wikimedia_resolves() {
        let p = lookup("wikimedia-webshapes");
        assert_eq!(p.id, "wikimedia-mixed");
    }

    #[test]
    fn unknown_falls_back_to_mixed_research() {
        let p = lookup("never-heard-of-this-corpus");
        assert_eq!(p.id, "mixed-research");
        assert!(!p.redistribute_bytes);
    }

    #[test]
    fn lookup_is_case_insensitive() {
        let p = lookup("UNSPLASH-WEBP-extra");
        assert_eq!(p.id, "unsplash");
    }

    #[test]
    fn by_id_round_trip() {
        let p = lookup("unsplash");
        let q = by_id(p.id);
        assert_eq!(q.id, p.id);
    }
}
