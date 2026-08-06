//! Bout detection and the debrief prompt, against a real schema.
//!
//! The rule this pins: a debrief is keyed on a BOUT — a contiguous run of
//! answers — not on a `sessions` row, because almost nobody closes a session
//! and the row is opened by a page load rather than by a person deciding to
//! start.

use anyhow::Result;
use sqlx::sqlite::SqlitePoolOptions;
use squintly::db::now_ms;
use squintly::debrief::{BOUT_GAP_MS, MIN_BOUT_RESPONSES, SubmitDebrief, pending, submit};

async fn pool() -> Result<sqlx::SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// Seed `n` answers ending `ago_ms` before now, one every two minutes.
async fn seed(pool: &sqlx::SqlitePool, observer: &str, n: usize, ago_ms: i64) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO observers (id, created_at) VALUES (?, 0)")
        .bind(observer)
        .execute(pool)
        .await?;
    let sess = format!("s-{observer}-{ago_ms}");
    sqlx::query(
        "INSERT OR IGNORE INTO sessions (id, observer_id, started_at, device_pixel_ratio, \
         screen_width_css, screen_height_css, study_id) \
         VALUES (?, ?, 0, 3.0, 390, 844, 'ssim2-nonphoto')",
    )
    .bind(&sess)
    .bind(observer)
    .execute(pool)
    .await?;
    let end = now_ms() - ago_ms;
    for i in 0..n {
        let trial = format!("{sess}-t{i}");
        sqlx::query(
            "INSERT INTO trials (id, session_id, kind, source_hash, a_encoding_id, b_encoding_id, \
             a_codec, b_codec, intrinsic_w, intrinsic_h, served_at) \
             VALUES (?, ?, 'pair', 'src', 'e1', 'e2', 'jpegli', 'jpegli', 256, 256, 0)",
        )
        .bind(&trial)
        .bind(&sess)
        .execute(pool)
        .await?;
        // Newest last, two minutes apart.
        let at = end - ((n - 1 - i) as i64) * 2 * 60 * 1000;
        sqlx::query(
            "INSERT INTO responses (trial_id, choice, dwell_ms, reveal_count, reveal_ms_total, \
             zoom_used, viewport_w_css, viewport_h_css, orientation, image_displayed_w_css, \
             image_displayed_h_css, intrinsic_to_device_ratio, responded_at) \
             VALUES (?, 'a', 1000, 1, 0, 0, 390, 844, 'portrait', 256.0, 256.0, 1.0, ?)",
        )
        .bind(&trial)
        .bind(at)
        .execute(pool)
        .await?;
    }
    Ok(())
}

#[tokio::test]
async fn a_finished_bout_is_offered_on_a_return_visit() -> Result<()> {
    let pool = pool().await?;
    seed(&pool, "obs1", 12, 3 * BOUT_GAP_MS).await?;
    let p = pending(&pool, "obs1", false)
        .await?
        .expect("a bout to ask about");
    assert_eq!(p.bout.responses, 12);
    assert_eq!(p.bout.comparisons, 12);
    assert!(!p.reasons.is_empty(), "the prompt needs its reason list");
    Ok(())
}

#[tokio::test]
async fn a_bout_still_in_progress_is_not_offered_on_a_return_visit() -> Result<()> {
    // Prompting about work somebody is still doing is a mid-session
    // interruption, which is the one thing this design exists to avoid.
    let pool = pool().await?;
    seed(&pool, "obs1", 12, 60 * 1000).await?;
    assert!(pending(&pool, "obs1", false).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn the_same_bout_is_offered_when_the_observer_signs_off_deliberately() -> Result<()> {
    // `ending` is what distinguishes "they clicked End session" from "they came
    // back later" — the first is immediate and the second is recall, and only
    // the first may ask about the run they are still in.
    let pool = pool().await?;
    seed(&pool, "obs1", 12, 60 * 1000).await?;
    let p = pending(&pool, "obs1", true)
        .await?
        .expect("offered at sign-off");
    assert_eq!(p.bout.responses, 12);
    Ok(())
}

#[tokio::test]
async fn a_bout_too_short_to_have_an_impression_of_is_not_asked_about() -> Result<()> {
    let pool = pool().await?;
    seed(&pool, "obs1", MIN_BOUT_RESPONSES - 1, 3 * BOUT_GAP_MS).await?;
    assert!(pending(&pool, "obs1", false).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn a_skip_is_recorded_so_the_question_is_not_asked_again() -> Result<()> {
    // Without a recorded skip the only evidence of having asked would be the
    // absence of a row, which is indistinguishable from never having asked —
    // and the observer would meet the same question about the same evening on
    // every future visit.
    let pool = pool().await?;
    seed(&pool, "obs1", 12, 3 * BOUT_GAP_MS).await?;
    let p = pending(&pool, "obs1", false).await?.expect("first ask");
    submit(
        &pool,
        &SubmitDebrief {
            observer_id: "obs1".into(),
            bout_start_ms: p.bout.start_ms,
            bout_end_ms: p.bout.end_ms,
            responses: p.bout.responses as i64,
            reasons: vec![],
            note: None,
            skipped: true,
            prompted_at: "return".into(),
        },
    )
    .await?;
    assert!(
        pending(&pool, "obs1", false).await?.is_none(),
        "a declined prompt must not come back"
    );
    Ok(())
}

#[tokio::test]
async fn an_earlier_bout_is_offered_once_the_recent_one_is_answered() -> Result<()> {
    let pool = pool().await?;
    // Two separate evenings, far enough apart to be different bouts.
    seed(&pool, "obs1", 8, 3 * BOUT_GAP_MS).await?;
    seed(&pool, "obs1", 9, 20 * BOUT_GAP_MS).await?;
    let recent = pending(&pool, "obs1", false)
        .await?
        .expect("most recent first");
    assert_eq!(
        recent.bout.responses, 8,
        "the newest bout is asked about first"
    );
    submit(
        &pool,
        &SubmitDebrief {
            observer_id: "obs1".into(),
            bout_start_ms: recent.bout.start_ms,
            bout_end_ms: recent.bout.end_ms,
            responses: recent.bout.responses as i64,
            reasons: vec!["rushed".into()],
            note: None,
            skipped: false,
            prompted_at: "return".into(),
        },
    )
    .await?;
    let older = pending(&pool, "obs1", false)
        .await?
        .expect("then the earlier one");
    assert_eq!(older.bout.responses, 9);
    Ok(())
}

#[tokio::test]
async fn an_unknown_reason_key_is_dropped_rather_than_stored() -> Result<()> {
    // A key no analysis knows how to read is not data. Storing it would put a
    // value in the column that silently means nothing, and somebody would
    // later count it.
    let pool = pool().await?;
    seed(&pool, "obs1", 12, 3 * BOUT_GAP_MS).await?;
    let p = pending(&pool, "obs1", false).await?.expect("a bout");
    submit(
        &pool,
        &SubmitDebrief {
            observer_id: "obs1".into(),
            bout_start_ms: p.bout.start_ms,
            bout_end_ms: p.bout.end_ms,
            responses: p.bout.responses as i64,
            reasons: vec!["rushed".into(), "not_a_real_reason".into()],
            note: Some("  ".into()),
            skipped: false,
            prompted_at: "return".into(),
        },
    )
    .await?;
    let (reasons, note): (String, Option<String>) =
        sqlx::query_as("SELECT reasons, note FROM session_debriefs WHERE observer_id = 'obs1'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(reasons, "rushed");
    assert_eq!(note, None, "whitespace-only note is not a note");
    Ok(())
}

#[tokio::test]
async fn an_observer_with_no_answers_is_never_prompted() -> Result<()> {
    let pool = pool().await?;
    sqlx::query("INSERT INTO observers (id, created_at) VALUES ('fresh', 0)")
        .execute(&pool)
        .await?;
    assert!(pending(&pool, "fresh", false).await?.is_none());
    assert!(pending(&pool, "fresh", true).await?.is_none());
    Ok(())
}

/// The client mirrors `UNDO_DEPTH` so it can hide the button rather than
/// offering one that 409s. Two copies of a number is two chances to disagree,
/// so this fails if they drift.
#[test]
fn the_client_and_server_agree_on_how_far_back_undo_reaches() {
    let ts = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/web/src/trial.ts"))
        .expect("read trial.ts");
    let want = format!("const UNDO_DEPTH = {};", squintly::handlers::UNDO_DEPTH);
    assert!(
        ts.contains(&want),
        "web/src/trial.ts must declare `{want}` to match handlers::UNDO_DEPTH"
    );
}
