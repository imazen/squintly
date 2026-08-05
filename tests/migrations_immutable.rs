//! Shipped migrations are immutable.
//!
//! sqlx checksums every migration file. Editing one that a deployment has
//! already applied — **including its comments** — makes every subsequent
//! startup fail with `migration N was previously applied but has been
//! modified`, and the failure is at boot, so the service simply does not come
//! back. That happened on 2026-07-31: a clarifying sentence was added to
//! 0017's header long after it had run in production, and the deploy crash-
//! looped until the file was restored byte-for-byte.
//!
//! Local `cargo test` never catches it, because a fresh in-memory database has
//! no `_sqlx_migrations` row to disagree with. This pins the checksums instead,
//! so editing a shipped migration fails here rather than in production.
//!
//! Adding a NEW migration is expected and does not touch these. If you must
//! change what a shipped one *did*, write a new migration that corrects it.

use std::collections::BTreeMap;

/// blake3 of each shipped migration, keyed by filename.
///
/// To add a migration: run this test, and paste the reported line for the new
/// file. Do NOT update an existing line to make a failure go away — that is the
/// bug this guards against.
fn expected() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        (
            "0001_init.sql",
            "d42f83437cb601cddd30f45282a9bd9ba0c402b0dcb4555dc51b1d49b9923b10",
        ),
        (
            "0002_grading.sql",
            "94c1cc40ed803076aed90f8a3d1b00f97e8cf06eb96bca445b20d4dd1c0651cc",
        ),
        (
            "0003_engagement.sql",
            "9b2740f6893f8ec7e0c9ba785616d630c6bdb37f0d59bb5353bdb2b4917ce238",
        ),
        (
            "0004_codec_support.sql",
            "2a7f0968e4c9eff0d31525b44ebd37ffac1b179d398631bb2526613c99e45340",
        ),
        (
            "0005_auth.sql",
            "5a9ffe7fec2eef4e2d60f1f23a90f6675699a2f921738a56bfa1ea754a5b7590",
        ),
        (
            "0006_v02_rigor.sql",
            "073d688e2540b56ae2e220a58c8300eadb3702f167d82adc7717bc03cdd144a6",
        ),
        (
            "0007_curator.sql",
            "04927c1d3728635b300df8f658111726ce0a8ac378890243a8b92c6dca3feb65",
        ),
        (
            "0008_suggestions.sql",
            "484c6712cdb5ff862d6384f2daf2435aa3037e4eb776a223643e9c75b19b1417",
        ),
        (
            "0009_curator_source_q.sql",
            "af2b3c29da476a7b5c5bd7453c6b351cb0fc8adb42b5406c057d2295779f95ee",
        ),
        (
            "0010_db_health.sql",
            "c9240aa27c5f4bdf84e077e0e024a51251677d322442bf4520084fdbcf9ef892",
        ),
        (
            "0011_manifest_snapshots.sql",
            "1c7db784d8e163630736c9d35697688b8a8e0d933372f6387c466b8534e0ae67",
        ),
        (
            "0012_pan_telemetry.sql",
            "3e0e9c4f3618f01ad8bc10cb438b70cc49cae1b4c86dd35870c126be9beae03a",
        ),
        (
            "0013_study_selection.sql",
            "0f1fe4016b35cfa88c95a0300fba1a3d0603668b45cab8e600096e821556349c",
        ),
        (
            "0014_zoom_factor.sql",
            "3becaaaceeba360fddd26599b75fc9904567d78c89cbf2a683b0fc1aa5ef82b4",
        ),
        (
            "0015_auth_sessions_and_rate_limit.sql",
            "6c3c9d02fe747bdc7909ff0235bad5898a11ab89a2a5ec19741099edae697493",
        ),
        (
            "0016_observer_dispositions.sql",
            "848a51d5d50767edf71a679ebb3a2c4eca9dc2faf722b84df2342273d0185671",
        ),
        (
            "0017_input_mode.sql",
            "7bc23a29a895a3a9ca685f80e500b95c777c54ce03bb83d99df2328c1893d503",
        ),
        (
            "0023_crowd_bt_eta.sql",
            "17ff976b51ffa8de9b627ff242085c320de91533ea03bdc029adc4d59e98b645",
        ),
        (
            "0022_trial_source_filename.sql",
            "cf08492af7d75a19ccf73aa1988cc7e19cdb15d84344747a72234c7071f2bc77",
        ),
        (
            "0021_cant_tell_hint.sql",
            "729a94e7880e87cdb482e0d13cb07589f274c490dfbd62f2caef9dc5bbd69882",
        ),
        (
            "0020_response_revisions.sql",
            "10b0c189be8b6c4c7c483d46fd4a8b7805d85337a488f16276b5c1cbc3e927dc",
        ),
        (
            "0019_view_dwell_and_controls.sql",
            "61422e60afa21a218e149cbe64f680b0226f7ac2cbba59736cb5924eab3677ef",
        ),
        (
            "0018_trial_content.sql",
            "bdad7a8ca81a5a952f43dc8599016475afe00eb6eb465baf2f9fc4391692ba89",
        ),
    ])
}

#[test]
fn shipped_migrations_are_unmodified() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");
    let mut actual: BTreeMap<String, String> = BTreeMap::new();
    for entry in std::fs::read_dir(dir).expect("migrations dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).expect("read migration");
        actual.insert(name, blake3::hash(&bytes).to_hex().to_string());
    }

    // Report the current hashes first: both failures below tell the reader to
    // re-run with this set, so printing it after the assertions would make that
    // advice useless — the panic gets there first.
    if std::env::var("SHOW_MIGRATION_HASHES").is_ok() {
        for (name, hash) in &actual {
            println!("        (\"{name}\", \"{hash}\"),");
        }
    }

    let exp = expected();
    let mut drifted = Vec::new();
    // An unpinned file is an unguarded file. Editing it would then take
    // production down at boot with nothing having failed here first.
    for name in actual.keys() {
        assert!(
            exp.contains_key(name.as_str()),
            "migration {name} is not pinned. Run with SHOW_MIGRATION_HASHES=1 and paste \
             its line into `expected()`."
        );
    }
    for (name, want) in &exp {
        if want.is_empty() {
            continue; // not yet pinned
        }
        match actual.get(*name) {
            Some(got) if got == want => {}
            Some(got) => drifted.push(format!(
                "{name}: expected {want}, found {got} — a shipped migration was edited. \
                 Restore it byte-for-byte and put the change in a NEW migration."
            )),
            None => drifted.push(format!("{name}: file is missing")),
        }
    }
    assert!(drifted.is_empty(), "{}", drifted.join("\n"));
}
