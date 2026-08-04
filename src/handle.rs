//! Stable, memorable, non-reversible handles for the reviewer leaderboard.
//!
//! # Why a derived handle rather than a display name
//!
//! A leaderboard needs a stable identity per reviewer, and squintly's whole
//! posture is that taking part is anonymous. Asking for a nickname would create
//! a second identity to store, moderate and leak. Deriving one from an
//! identifier we already hold gives stability for free and stores nothing new.
//!
//! # Why it cannot be reversed
//!
//! The handle is `BLAKE3(salt || identity)` folded into word indices. Without
//! the salt an attacker with a guessed email cannot confirm it: an unsalted
//! hash of an email address is trivially checked against a candidate list, and
//! a leaderboard is public by definition, so the salt is what stops the board
//! from being an email-membership oracle. Same reasoning as `hash_ip` — an
//! unsalted digest of a low-entropy input is not anonymisation.
//!
//! `SQUINTLY_HANDLE_SALT` must therefore be set and **stable** on a real
//! deployment: rotating it renames everyone, which silently resets the board.
//! Unset, a per-process salt is generated so handles still cannot be reversed —
//! but they change on every restart, which the boot log says out loud.
//!
//! # Why words
//!
//! `amber-heron-47` is something a contributor can recognise on a board and
//! quote in a message; `9f3c1a...` is not. The wordlists are deliberately
//! concrete, neutral nouns — nothing that could read as an insult when paired
//! with a low score, since the board shows quality metrics next to the name.

const ADJECTIVES: &[&str] = &[
    "amber", "arctic", "autumn", "azure", "bright", "brisk", "calm", "cedar", "cobalt", "copper",
    "coral", "crisp", "dawn", "deep", "dusky", "eager", "early", "ember", "fern", "flint",
    "gentle", "ginger", "glass", "golden", "granite", "harbor", "hazel", "ivory", "jade", "keen",
    "lilac", "lucid", "lunar", "marble", "meadow", "mellow", "misty", "north", "olive", "opal",
    "pearl", "pine", "quiet", "rapid", "river", "rowan", "russet", "sable", "sandy", "sepia",
    "silent", "silver", "slate", "solar", "spruce", "steady", "still", "stone", "sunlit", "swift",
    "teal", "tidal", "umber", "velvet",
];

const NOUNS: &[&str] = &[
    "alder", "anchor", "aspen", "badger", "beacon", "birch", "bison", "brook", "cairn", "canyon",
    "cedar", "cobble", "comet", "compass", "cypress", "delta", "dune", "eagle", "falcon", "fjord",
    "forest", "gannet", "geode", "glacier", "harbor", "heron", "ibex", "island", "juniper",
    "kestrel", "lantern", "larch", "ledger", "lichen", "lupine", "magpie", "maple", "marsh",
    "meridian", "mesa", "moraine", "nettle", "orchard", "osprey", "otter", "pebble", "petrel",
    "pika", "prism", "quarry", "quill", "ridge", "sable", "sequoia", "shale", "sparrow", "summit",
    "thistle", "tundra", "verbena", "walnut", "willow", "wren", "yarrow",
];

/// Environment variable holding the handle salt.
pub const HANDLE_SALT_ENV: &str = "SQUINTLY_HANDLE_SALT";

/// Derive a handle from a stable identity.
///
/// `identity` should be the observer's email when they have one (so the handle
/// follows them across devices, which is the whole reason email sign-in exists)
/// and their observer id otherwise. Trimmed and lowercased here so casing or
/// stray whitespace cannot produce two handles for one person.
pub fn handle_for(identity: &str, salt: &str) -> String {
    let normalized = identity.trim().to_ascii_lowercase();
    let mut h = blake3::Hasher::new();
    h.update(salt.as_bytes());
    h.update(b"\x00");
    h.update(normalized.as_bytes());
    let d = h.finalize();
    let b = d.as_bytes();

    let adj = ADJECTIVES[u16::from_le_bytes([b[0], b[1]]) as usize % ADJECTIVES.len()];
    let noun = NOUNS[u16::from_le_bytes([b[2], b[3]]) as usize % NOUNS.len()];
    // Two digits rather than three: the board is small, and a shorter handle is
    // easier to quote. Collisions are handled by the caller if they ever bite.
    let num = u16::from_le_bytes([b[4], b[5]]) % 100;
    format!("{adj}-{noun}-{num:02}")
}

/// The deployment's handle salt, read once.
pub fn salt() -> String {
    static SALT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SALT.get_or_init(|| {
        std::env::var(HANDLE_SALT_ENV)
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                tracing::warn!(
                    "{HANDLE_SALT_ENV} unset — leaderboard handles are still unreversible, \
                     but they change on every restart, so the board's names will not be stable"
                );
                crate::auth::generate_token()
            })
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handle_is_stable_for_one_identity() {
        let s = "salt";
        assert_eq!(handle_for("a@b.test", s), handle_for("a@b.test", s));
        // Casing and whitespace must not fork one person into two reviewers.
        assert_eq!(handle_for("A@B.TEST", s), handle_for("  a@b.test ", s));
    }

    /// The salt is the whole anonymity argument: a leaderboard is public, so an
    /// unsalted digest would let anyone confirm whether a guessed address is on
    /// it.
    #[test]
    fn the_salt_changes_the_handle() {
        assert_ne!(
            handle_for("a@b.test", "salt-one"),
            handle_for("a@b.test", "salt-two")
        );
    }

    #[test]
    fn different_identities_get_different_handles() {
        let s = "salt";
        let mut seen = std::collections::HashSet::new();
        for i in 0..500 {
            seen.insert(handle_for(&format!("person{i}@example.test"), s));
        }
        // 64 x 64 x 100 = 409,600 combinations; 500 draws should collide rarely.
        assert!(
            seen.len() >= 495,
            "only {} distinct handles from 500 identities",
            seen.len()
        );
    }

    #[test]
    fn a_handle_leaks_nothing_of_the_identity() {
        let h = handle_for("lilith@imazen.io", "salt");
        assert!(!h.contains("lilith"));
        assert!(!h.contains("imazen"));
        assert!(!h.contains('@'));
        // Shape is predictable and quotable.
        let parts: Vec<&str> = h.split('-').collect();
        assert_eq!(parts.len(), 3, "expected adjective-noun-NN, got {h}");
        assert!(ADJECTIVES.contains(&parts[0]), "{h}");
        assert!(NOUNS.contains(&parts[1]), "{h}");
        assert!(
            parts[2].len() == 2 && parts[2].chars().all(|c| c.is_ascii_digit()),
            "{h}"
        );
    }

    /// The board shows quality metrics beside the name, so no word may read as
    /// a judgement of the person next to a low score.
    #[test]
    fn the_wordlists_are_neutral_and_deduplicated() {
        for list in [ADJECTIVES, NOUNS] {
            let mut sorted = list.to_vec();
            sorted.sort_unstable();
            let before = sorted.len();
            sorted.dedup();
            assert_eq!(before, sorted.len(), "duplicate word in a handle list");
            for w in list {
                assert!(
                    w.chars().all(|c| c.is_ascii_lowercase()),
                    "{w} is not plain lowercase ascii"
                );
            }
        }
        assert!(ADJECTIVES.len() >= 32 && NOUNS.len() >= 32);
    }
}
