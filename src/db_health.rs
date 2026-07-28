//! Hourly per-table row count + DB-file metadata snapshot. Closes the
//! data-sprawl exit gate from README §7 ("per-table row counts in a
//! db_health table updated hourly so we notice when drift starts").
//!
//! Design:
//! - Tracks a curated allowlist of tables (the operational ones — sessions,
//!   trials, responses, observers, observer_grades, staircases, curator_*,
//!   etc.). Internal-only tables (sqlite_sequence, sqlx _migrations)
//!   intentionally skipped.
//! - Each refresh inserts one row per table at the same `refreshed_at` ms
//!   timestamp, plus a `db_health_meta` row with the file-level numbers.
//! - The hourly cadence (set by `main.rs::spawn_db_health_task`) gives
//!   a 24-row-per-day-per-table footprint; at ≤ 50 tracked tables that's
//!   ~1.2k rows/day, plenty cheap to keep indefinitely.

use anyhow::Result;
use sqlx::Row;
use sqlx::SqlitePool;

use crate::db::now_ms;

/// The set of operational tables tracked in db_health. Edit this list
/// when adding a new operational table; sqlx _migrations and SQLite
/// internals are deliberately excluded.
pub const TRACKED_TABLES: &[&str] = &[
    "observers",
    "sessions",
    "trials",
    "responses",
    "staircases",
    "observer_grades",
    "corpus_anchors",
    "auth_tokens",
    "observer_aliases",
    "manifest_snapshots",
    "curator_decisions",
    "curator_variants",
    "curator_source_q",
    "suggestions",
];

/// Snapshot every tracked table's row count + the DB-file metadata into
/// `db_health` / `db_health_meta`. Returns the number of (table_name,
/// row_count) rows written. Logs and skips individual tables that don't
/// exist (so the function survives schema rollbacks during dev).
pub async fn refresh(pool: &SqlitePool) -> Result<u64> {
    let ts = now_ms();
    let mut written = 0u64;
    for table in TRACKED_TABLES {
        // Read row counts via a parameterless plain string COUNT — SQLite
        // doesn't support binding table names. The allowlist above means
        // these strings come from compiled-in const data, not user input,
        // so no injection risk.
        let count_sql = format!("SELECT COUNT(*) FROM \"{table}\"");
        match sqlx::query(&count_sql).fetch_one(pool).await {
            Ok(row) => {
                let n: i64 = row.try_get(0).unwrap_or(0);
                sqlx::query(
                    "INSERT INTO db_health (refreshed_at, table_name, row_count) \
                     VALUES (?, ?, ?)",
                )
                .bind(ts)
                .bind(table)
                .bind(n)
                .execute(pool)
                .await?;
                written += 1;
            }
            Err(e) => {
                tracing::debug!(table, error = %e, "db_health: table absent or query failed; skipping");
            }
        }
    }

    // DB-file metadata. PRAGMAs that need integer parsing:
    //   page_count, page_size                   -> size_bytes = product
    //   user_version                            -> migration cursor
    //   schema_version (different from user_v)  -> not stored
    let page_count: i64 = pragma_int(pool, "page_count").await.unwrap_or(0);
    let page_size: i64 = pragma_int(pool, "page_size").await.unwrap_or(0);
    let sqlite_version: String = sqlx::query("SELECT sqlite_version()")
        .fetch_one(pool)
        .await
        .ok()
        .and_then(|r| r.try_get::<String, _>(0).ok())
        .unwrap_or_else(|| "unknown".into());
    sqlx::query(
        "INSERT INTO db_health_meta (refreshed_at, db_size_bytes, page_count, page_size, sqlite_version) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(ts)
    .bind(page_count * page_size)
    .bind(page_count)
    .bind(page_size)
    .bind(sqlite_version)
    .execute(pool)
    .await?;
    Ok(written)
}

async fn pragma_int(pool: &SqlitePool, name: &str) -> Result<i64> {
    let sql = format!("PRAGMA {name}");
    let row = sqlx::query(&sql).fetch_one(pool).await?;
    Ok(row.try_get::<i64, _>(0)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn refresh_populates_one_row_per_extant_table_plus_meta() -> Result<()> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;

        // Seed a couple of rows so the counts aren't all 0.
        sqlx::query("INSERT INTO observers (id, created_at) VALUES ('o1', 1), ('o2', 2)")
            .execute(&pool)
            .await?;

        let written = refresh(&pool).await?;
        assert!(
            written >= 1,
            "expected at least one tracked table to be written"
        );

        // observers count should be 2.
        let (n,): (i64,) = sqlx::query_as(
            "SELECT row_count FROM db_health WHERE table_name = 'observers' \
             ORDER BY refreshed_at DESC LIMIT 1",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(n, 2);

        // db_health_meta has one row with positive db_size_bytes.
        let (size,): (i64,) = sqlx::query_as(
            "SELECT db_size_bytes FROM db_health_meta ORDER BY refreshed_at DESC LIMIT 1",
        )
        .fetch_one(&pool)
        .await?;
        assert!(size > 0, "db_size_bytes should be positive, got {size}");

        // Idempotent across multiple refreshes — each one creates a new
        // refreshed_at bucket; old rows persist for trend analysis.
        // Need a millisecond gap to avoid PK collisions.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let again = refresh(&pool).await?;
        assert_eq!(
            again, written,
            "second refresh should write the same table count"
        );
        let (total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM db_health")
            .fetch_one(&pool)
            .await?;
        assert_eq!(
            total as u64,
            written * 2,
            "two refreshes should double the row count"
        );
        Ok(())
    }
}
