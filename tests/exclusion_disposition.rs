//! `rebuild_dispositions` against a real schema.
//!
//! The unit tests in `src/exclusion.rs` pin the statistics; this pins the part
//! they cannot see — that the SQL actually matches the tables. It caught
//! `encoding_a_id` (the column is `a_encoding_id`), which builds fine and fails
//! only at runtime because the query is not compile-checked.

use anyhow::Result;
use sqlx::Row;
use sqlx::sqlite::SqlitePoolOptions;
use squintly::exclusion::{ExclusionPolicy, rebuild_dispositions};

async fn seed(pool: &sqlx::SqlitePool, study: &str, ratings: &[(&str, &str, &str)]) -> Result<()> {
    let mut seen_observers = std::collections::HashSet::new();
    for (observer, _, _) in ratings {
        if seen_observers.insert(*observer) {
            sqlx::query("INSERT INTO observers (id, created_at) VALUES (?, 0)")
                .bind(observer)
                .execute(pool)
                .await?;
            sqlx::query(
                "INSERT INTO sessions (id, observer_id, started_at, device_pixel_ratio, \
                 screen_width_css, screen_height_css, study_id) \
                 VALUES (?, ?, 0, 3.0, 390, 844, ?)",
            )
            .bind(format!("sess-{observer}"))
            .bind(observer)
            .bind(study)
            .execute(pool)
            .await?;
        }
    }
    for (i, (observer, stimulus, choice)) in ratings.iter().enumerate() {
        // Prefixed by study so a second seed() call in one test does not
        // collide with the first on trials.id.
        let trial_id = format!("{study}-t{i}");
        sqlx::query(
            "INSERT INTO trials (id, session_id, kind, source_hash, a_encoding_id, a_codec, \
             intrinsic_w, intrinsic_h, served_at) \
             VALUES (?, ?, 'single', 'src', ?, 'mozjpeg', 256, 256, 0)",
        )
        .bind(&trial_id)
        .bind(format!("sess-{observer}"))
        .bind(stimulus)
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO responses (trial_id, choice, dwell_ms, reveal_count, \
             reveal_ms_total, zoom_used, viewport_w_css, viewport_h_css, orientation, \
             image_displayed_w_css, image_displayed_h_css, intrinsic_to_device_ratio, \
             responded_at) \
             VALUES (?, ?, 1500, 1, 400, 0, 390, 844, 'portrait', 256.0, 256.0, 1.0, 0)",
        )
        .bind(&trial_id)
        .bind(choice)
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn dispositions(pool: &sqlx::SqlitePool) -> Result<Vec<(String, String, i64, Option<f64>)>> {
    let rows = sqlx::query(
        "SELECT observer_id, disposition, n_comparable, r_s FROM observer_dispositions \
         ORDER BY observer_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                r.get::<String, _>("observer_id"),
                r.get::<String, _>("disposition"),
                r.get::<i64, _>("n_comparable"),
                r.get::<Option<f64>, _>("r_s"),
            )
        })
        .collect())
}

#[tokio::test]
async fn a_crowd_is_screened_and_a_lone_expert_is_not() -> Result<()> {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    // Ten stimuli. `agree-1/2/3` broadly track each other; `contrary` answers
    // upside down on every one.
    let truth = [1, 2, 3, 4, 1, 2, 3, 4, 2, 3];
    let mut rows: Vec<(String, String, String)> = Vec::new();
    for (i, t) in truth.iter().enumerate() {
        let stim = format!("enc{i}");
        for who in ["agree-1", "agree-2", "agree-3"] {
            rows.push((who.to_string(), stim.clone(), t.to_string()));
        }
        rows.push(("contrary".to_string(), stim.clone(), (5 - t).to_string()));
    }
    let borrowed: Vec<(&str, &str, &str)> = rows
        .iter()
        .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str()))
        .collect();
    seed(&pool, "main", &borrowed).await?;

    let policy = ExclusionPolicy {
        enabled: true,
        min_ratings: 4,
        min_comparable: 4,
        ..ExclusionPolicy::default()
    };
    let n = rebuild_dispositions(&pool, |_| policy).await?;
    assert_eq!(n, 4, "one row per observer");

    let got = dispositions(&pool).await?;
    let by: std::collections::HashMap<_, _> = got
        .iter()
        .map(|(o, d, n, r)| (o.as_str(), (d.as_str(), *n, *r)))
        .collect();

    for who in ["agree-1", "agree-2", "agree-3"] {
        assert_eq!(by[who].0, "included", "{who} agrees with the crowd");
        assert_eq!(by[who].1, 10, "{who} had peers on every stimulus");
    }
    assert_eq!(by["contrary"].0, "excluded");
    assert!(
        by["contrary"].2.unwrap() < 0.0,
        "an inverted rater must anti-correlate: {:?}",
        by["contrary"].2
    );

    // Rebuilding is idempotent — it must not accumulate duplicate rows.
    rebuild_dispositions(&pool, |_| policy).await?;
    assert_eq!(dispositions(&pool).await?.len(), 4);

    // --- the solo-expert case, in its own study -------------------------
    let solo: Vec<(String, String, String)> = (0..12)
        .map(|i| {
            (
                "expert".to_string(),
                format!("x{i}"),
                ((i % 4) + 1).to_string(),
            )
        })
        .collect();
    let borrowed: Vec<(&str, &str, &str)> = solo
        .iter()
        .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str()))
        .collect();
    seed(&pool, "ssim2-nonphoto", &borrowed).await?;
    rebuild_dispositions(&pool, |_| policy).await?;

    let got = dispositions(&pool).await?;
    let expert = got
        .iter()
        .find(|(o, ..)| o == "expert")
        .expect("expert row");
    assert_eq!(
        expert.1, "insufficient_data",
        "nobody else rated these, so there is nothing to be an outlier against — \
         and 'could not check' must never be recorded as 'excluded'"
    );
    assert_eq!(expert.2, 0);

    Ok(())
}

/// Turning the policy off must not stop the screens running: the disposition is
/// still computed and recorded, only its enforcement changes.
#[tokio::test]
async fn switching_the_policy_off_still_records_the_verdict() -> Result<()> {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let truth = [1, 2, 3, 4, 1, 2, 3, 4];
    let mut rows: Vec<(String, String, String)> = Vec::new();
    for (i, t) in truth.iter().enumerate() {
        let stim = format!("enc{i}");
        for who in ["p1", "p2", "p3"] {
            rows.push((who.to_string(), stim.clone(), t.to_string()));
        }
        rows.push(("contrary".to_string(), stim.clone(), (5 - t).to_string()));
    }
    let borrowed: Vec<(&str, &str, &str)> = rows
        .iter()
        .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str()))
        .collect();
    seed(&pool, "main", &borrowed).await?;

    let off = ExclusionPolicy {
        enabled: false,
        min_ratings: 4,
        min_comparable: 4,
        ..ExclusionPolicy::default()
    };
    rebuild_dispositions(&pool, |_| off).await?;

    let row = sqlx::query(
        "SELECT disposition, policy_enabled, reason FROM observer_dispositions \
         WHERE observer_id = 'contrary'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        row.get::<String, _>("disposition"),
        "excluded",
        "the screen still runs with the policy off"
    );
    assert_eq!(
        row.get::<i64, _>("policy_enabled"),
        0,
        "and the row records that it was not being enforced"
    );
    assert!(
        row.get::<Option<String>, _>("reason")
            .unwrap_or_default()
            .contains("§4.4"),
        "the reason must cite which screen fired"
    );
    Ok(())
}
