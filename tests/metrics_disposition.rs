//! Metric ingest and the disposition report, against a real schema.
//!
//! The unit tests in `src/metrics.rs` pin the parsing; this pins the parts they
//! cannot see — that the SQL matches the tables, and that the report refuses
//! the things it is supposed to refuse rather than quietly producing a number.

use anyhow::Result;
use sqlx::sqlite::SqlitePoolOptions;
use squintly::disposition::{MIN_PAIRS_FOR_RHO, MIN_REPEATS_FOR_CEILING, compute};
use squintly::metrics::{MetricRow, parse_delimited};

async fn pool() -> Result<sqlx::SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

async fn ingest(pool: &sqlx::SqlitePool, rows: &[MetricRow]) -> Result<()> {
    for r in rows {
        sqlx::query(
            "INSERT INTO encoding_metrics (encoding_id, metric, value, source, ingested_at) \
             VALUES (?, ?, ?, 'test', 0) \
             ON CONFLICT(encoding_id, metric) DO UPDATE SET value = excluded.value",
        )
        .bind(&r.encoding_id)
        .bind(&r.metric)
        .bind(r.value)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Seed `n` pair comparisons. `agree` decides whether the observer picks the
/// encoding with the better metric score, so a test can dial ρ directly.
///
/// Encodings are named `encN`/`encN_worse`, and the metric scores are seeded so
/// the plain name is always the better one.
async fn seed_pairs(
    pool: &sqlx::SqlitePool,
    study: &str,
    observer: &str,
    n: usize,
    agree: impl Fn(usize) -> bool,
) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO observers (id, created_at) VALUES (?, 0)")
        .bind(observer)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT OR IGNORE INTO sessions (id, observer_id, started_at, device_pixel_ratio, \
         screen_width_css, screen_height_css, study_id) VALUES (?, ?, 0, 3.0, 390, 844, ?)",
    )
    .bind(format!("sess-{observer}"))
    .bind(observer)
    .bind(study)
    .execute(pool)
    .await?;

    for i in 0..n {
        let trial = format!("{study}-{observer}-t{i}");
        // Slot A always carries the better encoding here. Real trials
        // counterbalance; this fixture does not, because what is under test is
        // the agreement arithmetic, and the repeat test below is the one that
        // exercises slot independence.
        sqlx::query(
            "INSERT INTO trials (id, session_id, kind, source_hash, a_encoding_id, b_encoding_id, \
             a_codec, b_codec, intrinsic_w, intrinsic_h, served_at) \
             VALUES (?, ?, 'pair', 'src', ?, ?, 'jpegli', 'jpegli', 256, 256, 0)",
        )
        .bind(&trial)
        .bind(format!("sess-{observer}"))
        .bind(format!("enc{i}"))
        .bind(format!("enc{i}_worse"))
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO responses (trial_id, choice, dwell_ms, reveal_count, reveal_ms_total, zoom_used, viewport_w_css, viewport_h_css, orientation, image_displayed_w_css, image_displayed_h_css, intrinsic_to_device_ratio, responded_at) VALUES (?, ?, 1000, 1, 0, 0, 390, 844, 'portrait', 256.0, 256.0, 1.0, 0)",
        )
        .bind(&trial)
        .bind(if agree(i) { "a" } else { "b" })
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn seed_scores(pool: &sqlx::SqlitePool, metric: &str, n: usize) -> Result<()> {
    let mut rows = Vec::new();
    for i in 0..n {
        // Higher is better for ssim2, so the plain name scores higher.
        rows.push(MetricRow {
            encoding_id: format!("enc{i}"),
            metric: metric.to_string(),
            value: 90.0,
        });
        rows.push(MetricRow {
            encoding_id: format!("enc{i}_worse"),
            metric: metric.to_string(),
            value: 60.0,
        });
    }
    ingest(pool, &rows).await
}

#[tokio::test]
async fn a_metric_that_agrees_with_everyone_scores_one() -> Result<()> {
    let pool = pool().await?;
    let study = "ssim2-nonphoto";
    seed_pairs(&pool, study, "obs1", 40, |_| true).await?;
    seed_scores(&pool, "ssim2", 40).await?;

    let d = compute(&pool, study).await?;
    assert_eq!(d.comparisons, 40);
    let m = d
        .metrics
        .iter()
        .find(|m| m.metric == "ssim2")
        .expect("ssim2");
    assert_eq!(m.direction, "higher_is_better");
    assert_eq!(m.comparisons, 40);
    assert_eq!(m.rho, Some(1.0));
    Ok(())
}

#[tokio::test]
async fn direction_decides_the_sign_and_getting_it_wrong_would_invert_rho() -> Result<()> {
    // The same observations scored against a lower-is-better metric must give
    // the complementary answer. This is the arithmetic the Unknown guard exists
    // to protect: if a metric's direction were guessed, rho would come out
    // 1 - rho, which reads exactly like "the metric disagrees with humans".
    let pool = pool().await?;
    let study = "ssim2-nonphoto";
    seed_pairs(&pool, study, "obs1", 40, |_| true).await?;
    // butteraugli is lower-is-better, so seeding the SAME numbers means the
    // plain encoding is now the WORSE one by the metric's own convention.
    seed_scores(&pool, "butteraugli_pnorm3", 40).await?;

    let d = compute(&pool, study).await?;
    let m = d
        .metrics
        .iter()
        .find(|m| m.metric == "butteraugli_pnorm3")
        .expect("butteraugli");
    assert_eq!(m.direction, "lower_is_better");
    assert_eq!(m.rho, Some(0.0), "a lower-better metric must invert");
    Ok(())
}

#[tokio::test]
async fn an_unknown_metric_is_refused_by_name_rather_than_correlated() -> Result<()> {
    let pool = pool().await?;
    let study = "ssim2-nonphoto";
    seed_pairs(&pool, study, "obs1", 40, |_| true).await?;
    seed_scores(&pool, "some_metric_from_2027", 40).await?;

    let d = compute(&pool, study).await?;
    assert!(
        d.metrics.is_empty(),
        "an unknown-direction metric must not produce a rho"
    );
    let u = d.unusable.first().expect("reported as unusable");
    assert_eq!(u.metric, "some_metric_from_2027");
    assert!(u.reason.contains("direction"), "unhelpful: {}", u.reason);
    Ok(())
}

#[tokio::test]
async fn rho_is_withheld_below_the_minimum_rather_than_printed_noisily() -> Result<()> {
    let pool = pool().await?;
    let study = "ssim2-nonphoto";
    let few = MIN_PAIRS_FOR_RHO - 1;
    seed_pairs(&pool, study, "obs1", few, |_| true).await?;
    seed_scores(&pool, "ssim2", few).await?;

    let d = compute(&pool, study).await?;
    let m = d
        .metrics
        .iter()
        .find(|m| m.metric == "ssim2")
        .expect("ssim2");
    assert_eq!(m.comparisons, few);
    assert_eq!(
        m.rho, None,
        "below the minimum, rho is not a number to print"
    );
    assert_eq!(m.rho_over_ceiling, None);
    Ok(())
}

#[tokio::test]
async fn rho_over_ceiling_needs_a_ceiling_and_stays_none_without_one() -> Result<()> {
    // The whole point: "ssim2 scored 0.7" reads completely differently against
    // a ceiling of 0.95 than against 0.72. With no repeats served there is no
    // ceiling, so the reportable figure must be absent — not silently equal to
    // rho, which is what dividing by an assumed 1.0 would give.
    let pool = pool().await?;
    let study = "ssim2-nonphoto";
    seed_pairs(&pool, study, "obs1", 40, |_| true).await?;
    seed_scores(&pool, "ssim2", 40).await?;

    let d = compute(&pool, study).await?;
    assert_eq!(d.ceiling.repeat_pairs, 0);
    assert_eq!(d.ceiling.ceiling, None);
    let m = d
        .metrics
        .iter()
        .find(|m| m.metric == "ssim2")
        .expect("ssim2");
    assert_eq!(m.rho, Some(1.0));
    assert_eq!(
        m.rho_over_ceiling, None,
        "a rho must never be reported against an assumed ceiling"
    );
    Ok(())
}

#[tokio::test]
async fn self_agreement_is_measured_over_repeated_pairs() -> Result<()> {
    let pool = pool().await?;
    let study = "ssim2-nonphoto";
    sqlx::query("INSERT INTO observers (id, created_at) VALUES ('obs1', 0)")
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO sessions (id, observer_id, started_at, device_pixel_ratio, \
         screen_width_css, screen_height_css, study_id) \
         VALUES ('s1', 'obs1', 0, 3.0, 390, 844, ?)",
    )
    .bind(study)
    .execute(&pool)
    .await?;

    // Serve each pair twice. The first `consistent` of them get the same answer
    // both times; the rest get opposite answers.
    let total = MIN_REPEATS_FOR_CEILING + 2;
    let consistent = total - 3;
    for i in 0..total {
        for rep in 0..2 {
            let trial = format!("t{i}-{rep}");
            sqlx::query(
                "INSERT INTO trials (id, session_id, kind, source_hash, a_encoding_id, \
                 b_encoding_id, a_codec, b_codec, intrinsic_w, intrinsic_h, served_at) \
                 VALUES (?, 's1', 'pair', 'src', ?, ?, 'jpegli', 'jpegli', 256, 256, 0)",
            )
            .bind(&trial)
            .bind(format!("enc{i}"))
            .bind(format!("enc{i}_worse"))
            .execute(&pool)
            .await?;
            let same = i < consistent;
            let choice = if rep == 0 || same { "a" } else { "b" };
            sqlx::query(
                "INSERT INTO responses (trial_id, choice, dwell_ms, reveal_count, reveal_ms_total, zoom_used, viewport_w_css, viewport_h_css, orientation, image_displayed_w_css, image_displayed_h_css, intrinsic_to_device_ratio, responded_at) VALUES (?, ?, 1000, 1, 0, 0, 390, 844, 'portrait', 256.0, 256.0, 1.0, 0)",
            )
            .bind(&trial)
            .bind(choice)
            .execute(&pool)
            .await?;
        }
    }

    let d = compute(&pool, study).await?;
    assert_eq!(d.ceiling.repeat_pairs, total);
    assert_eq!(d.ceiling.agreed, consistent);
    let c = d.ceiling.ceiling.expect("above the minimum");
    assert!((c - consistent as f64 / total as f64).abs() < 1e-9);
    Ok(())
}

#[tokio::test]
async fn a_repeat_answered_the_same_way_counts_as_agreement_whichever_slot_it_landed_in()
-> Result<()> {
    // Pair slots are counterbalanced, so the same pair comes back with the
    // encodings swapped. Comparing raw `choice` strings would score a perfectly
    // consistent observer as inconsistent every time the slots flipped —
    // halving the measured ceiling and making every rho/ceiling look twice as
    // good as it is.
    let pool = pool().await?;
    let study = "ssim2-nonphoto";
    sqlx::query("INSERT INTO observers (id, created_at) VALUES ('obs1', 0)")
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO sessions (id, observer_id, started_at, device_pixel_ratio, \
         screen_width_css, screen_height_css, study_id) \
         VALUES ('s1', 'obs1', 0, 3.0, 390, 844, ?)",
    )
    .bind(study)
    .execute(&pool)
    .await?;

    for i in 0..MIN_REPEATS_FOR_CEILING {
        // First serving: better encoding in slot A, observer picks A.
        // Second serving: SWAPPED, observer picks B — the same encoding.
        for (rep, (a, b, choice)) in [("good", "bad", "a"), ("bad", "good", "b")]
            .into_iter()
            .enumerate()
        {
            let trial = format!("t{i}-{rep}");
            sqlx::query(
                "INSERT INTO trials (id, session_id, kind, source_hash, a_encoding_id, \
                 b_encoding_id, a_codec, b_codec, intrinsic_w, intrinsic_h, served_at) \
                 VALUES (?, 's1', 'pair', 'src', ?, ?, 'jpegli', 'jpegli', 256, 256, 0)",
            )
            .bind(&trial)
            .bind(format!("enc{i}_{a}"))
            .bind(format!("enc{i}_{b}"))
            .execute(&pool)
            .await?;
            sqlx::query(
                "INSERT INTO responses (trial_id, choice, dwell_ms, reveal_count, reveal_ms_total, zoom_used, viewport_w_css, viewport_h_css, orientation, image_displayed_w_css, image_displayed_h_css, intrinsic_to_device_ratio, responded_at) VALUES (?, ?, 1000, 1, 0, 0, 390, 844, 'portrait', 256.0, 256.0, 1.0, 0)",
            )
            .bind(&trial)
            .bind(choice)
            .execute(&pool)
            .await?;
        }
    }

    let d = compute(&pool, study).await?;
    assert_eq!(d.ceiling.repeat_pairs, MIN_REPEATS_FOR_CEILING);
    assert_eq!(
        d.ceiling.ceiling,
        Some(1.0),
        "the same encoding chosen twice is agreement regardless of slot"
    );
    Ok(())
}

#[tokio::test]
async fn pairs_the_metric_cannot_score_are_reported_not_counted() -> Result<()> {
    let pool = pool().await?;
    let study = "ssim2-nonphoto";
    seed_pairs(&pool, study, "obs1", 40, |_| true).await?;
    // Score only the first 25 pairs. The rest have no metric value, so the
    // metric has no opinion and they must not land in the denominator.
    seed_scores(&pool, "ssim2", 25).await?;

    let d = compute(&pool, study).await?;
    let m = d
        .metrics
        .iter()
        .find(|m| m.metric == "ssim2")
        .expect("ssim2");
    assert_eq!(m.comparisons, 25);
    assert_eq!(m.uncovered, 15);
    assert_eq!(m.rho, Some(1.0), "coverage must not dilute agreement");
    Ok(())
}

#[tokio::test]
async fn a_tie_is_an_outcome_and_is_kept_out_of_the_denominator() -> Result<()> {
    // Davidson's model has a tie term precisely so a tie is an outcome rather
    // than noise. Counting one as a metric miss would punish the metric for a
    // pair the observer could not separate; counting it as a hit would reward
    // it for the same. Neither: it is reported on its own.
    let pool = pool().await?;
    let study = "ssim2-nonphoto";
    seed_pairs(&pool, study, "obs1", 30, |_| true).await?;
    seed_scores(&pool, "ssim2", 30).await?;
    sqlx::query("UPDATE responses SET choice = 'tie' WHERE trial_id LIKE ? AND rowid % 3 = 0")
        .bind(format!("{study}-obs1-%"))
        .execute(&pool)
        .await?;

    let d = compute(&pool, study).await?;
    let m = d
        .metrics
        .iter()
        .find(|m| m.metric == "ssim2")
        .expect("ssim2");
    assert!(m.ties > 0, "the fixture should have produced ties");
    assert_eq!(
        m.comparisons + m.ties,
        30,
        "every comparison is either scored or a tie, never dropped"
    );
    assert_eq!(m.rho, Some(1.0));
    Ok(())
}

#[tokio::test]
async fn goldens_score_the_first_answer_so_undo_cannot_defeat_them() -> Result<()> {
    let pool = pool().await?;
    let study = "ssim2-nonphoto";
    seed_pairs(&pool, study, "obs1", 4, |_| true).await?;
    // Mark them all golden, expecting 'a' — which is what was answered.
    sqlx::query("UPDATE trials SET is_golden = 1, expected_choice = 'a'")
        .execute(&pool)
        .await?;
    // One of them was answered 'b' FIRST and revised to 'a'. That is a failed
    // attention check that the observer took back, and it must still count as
    // failed — otherwise the honeypot is defeated by pressing undo.
    sqlx::query("UPDATE responses SET original_choice = 'b' WHERE trial_id = ?")
        .bind(format!("{study}-obs1-t0"))
        .execute(&pool)
        .await?;

    let d = compute(&pool, study).await?;
    assert_eq!(d.golden_trials, 4);
    assert_eq!(
        d.golden_pass_rate,
        Some(0.75),
        "the revised answer must not launder a failed check"
    );
    Ok(())
}

#[test]
fn a_zenmetrics_shaped_tsv_parses_into_every_family() {
    // The real column shape, versioned names and all.
    let tsv = "encoding_id\tcodec\tbytes\tssim2\tbutteraugli_pnorm3\tdssim\tiwssim_gpu\t\
               cvvdp_imazen_v0_0_1\tzensim\n\
               enc1\tjpegli\t12345\t88.5\t1.1\t0.004\t0.97\t9.2\t95.1\n";
    let rows = parse_delimited(tsv, b'\t').expect("parse");
    let names: Vec<&str> = rows.iter().map(|r| r.metric.as_str()).collect();
    for want in [
        "ssim2",
        "butteraugli_pnorm3",
        "dssim",
        "iwssim_gpu",
        "cvvdp_imazen_v0_0_1",
        "zensim",
    ] {
        assert!(names.contains(&want), "missing {want} in {names:?}");
    }
    assert!(!names.contains(&"bytes"), "bytes is not a quality metric");
    assert!(!names.contains(&"codec"));
}
